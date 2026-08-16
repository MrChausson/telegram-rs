//! Main binary: bridge between the network runtime (tokio) and the window (winit).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use grammers_client::client::UpdatesConfiguration;
use grammers_client::update::Update;
use grammers_session::types::PeerRef;
use tg::client::Telegram;
use tg::session::load_or_new;
use tokio::sync::mpsc;
use ui::bridge::{Request, UiMessage};
use ui::chatlist::ChatRow;

// mimalloc allocator: returns freed memory to the OS (less resident RSS
// than glibc, whose arenas keep previously allocated memory).
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
use ui::messages::MsgRow;

const ENV_PATH: &str = ".env";
const SESSION_PATH: &str = ".tg.session";
const MESSAGE_LIMIT: usize = 200;

fn load_env(path: &str) -> HashMap<String, String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (k, v) = line.split_once('=')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

fn var(env: &HashMap<String, String>, name: &str) -> anyhow::Result<String> {
    env.get(name)
        .cloned()
        .or_else(|| std::env::var(name).ok())
        .ok_or_else(|| anyhow::anyhow!("missing variable {name} (in {ENV_PATH})"))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let env = load_env(ENV_PATH);
    let api_id: i32 = var(&env, "API_ID")?.parse()?;
    // `--open "<title>"` opens that chat at startup (useful for tests);
    // `--open-first` is a shortcut for the first chat (equivalent to
    // `--open "*"`).
    let mut auto_open = None;
    let mut iter = std::env::args().skip(1);
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--open" => auto_open = iter.next(),
            "--open-first" => auto_open = Some("*".to_string()),
            _ => {}
        }
    }

    let (ui_tx, ui_rx) = mpsc::unbounded_channel::<UiMessage>();
    let (req_tx, mut req_rx) = mpsc::unbounded_channel::<Request>();

    // Network runtime on a separate thread (winit lives on the main thread).
    // Reduced stack for the network thread (tokio fits in 1 MiB; the default
    // 2-8 MiB stacks inflate RSS).
    let _ = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(async move {
            let session = Arc::new(load_or_new(Path::new(SESSION_PATH)));
            let tg = match Telegram::connect(session, api_id).await {
                Ok(tg) => tg,
                Err(e) => {
                    let _ = ui_tx.send(UiMessage::Error(format!("Could not connect: {e}")));
                    std::future::pending::<()>().await;
                    return;
                }
            };

            if !tg.is_authorized().await.unwrap_or(false) {
                let _ = ui_tx.send(UiMessage::Error(
                    "Not signed in. Run first: cargo run -p tg --example login".to_string(),
                ));
                std::future::pending::<()>().await;
                return;
            }

            // Cache the self user: otherwise `stream_updates` never triggers
            // the initial `GetState` and the updates stream stays silent.
            let _ = tg.client().get_me().await;

            match tg.get_dialogs().await {
                Ok(dialogs) => {
                    let mut peers: HashMap<i64, (String, PeerRef)> = HashMap::new();
                    let rows: Vec<ChatRow> = dialogs
                        .iter()
                        .map(|d| {
                            peers.insert(
                                d.id.bot_api_dialog_id(),
                                (d.title.clone(), d.peer_ref),
                            );
                            ChatRow {
                                id: d.id.bot_api_dialog_id(),
                                title: d.title.clone(),
                                subtitle: d.last_message.clone().unwrap_or_default(),
                                unread: d.unread_count,
                            }
                        })
                        .collect();
                    let _ = ui_tx.send(UiMessage::Dialogs(rows));
                    serve(tg, &ui_tx, &mut req_rx, &peers).await;
                }
                Err(e) => {
                    let _ = ui_tx.send(UiMessage::Error(format!(
                        "Could not load chats: {e}"
                    )));
                }
            }

            std::future::pending::<()>().await;
        });
    });

    ui::window::run(ui_rx, req_tx, auto_open)?;
    Ok(())
}

/// Network loop: handles UI requests and real-time updates.
async fn serve(
    mut tg: Telegram,
    ui_tx: &mpsc::UnboundedSender<UiMessage>,
    req_rx: &mut mpsc::UnboundedReceiver<Request>,
    peers: &HashMap<i64, (String, PeerRef)>,
) {
    let updates_rx = tg.take_updates();
    // `catch_up = true`: syncs the update state at startup (like the official
    // app); subsequent updates are pushed live by the server.
    let mut updates = tg
        .client()
        .stream_updates(
            updates_rx,
            UpdatesConfiguration {
                catch_up: true,
                update_queue_limit: Some(100),
            },
        )
        .await;

// Discreet safety net (15 s, ~4 req/min on the open chat): catches any
    // missed update without spamming Telegram.
    let refresh = std::time::Duration::from_secs(15);
    let mut last_refresh = std::time::Instant::now();
    let mut open_id: Option<i64> = None;
    let mut open_sig: Option<(usize, String)> = None;

    // Periodic session save (pts + peer cache): avoids replaying a big
    // `catch_up` on every restart (hence less memory and startup work).
    let save_every = std::time::Duration::from_secs(30);
    let mut last_save = std::time::Instant::now();
    let session = tg.session().clone();

    // Grace period: `catch_up` replays history at startup; we do not relay
    // that replay to the UI (it would only be old updates).
    let started = std::time::Instant::now();
    let grace = std::time::Duration::from_secs(10);

    loop {
        // Drain queued requests (without blocking update reception).
        while let Ok(req) = req_rx.try_recv() {
            if let Request::OpenChat { id } = req {
                open_id = Some(id);
                open_sig = None;
            }
            handle_request(&tg, ui_tx, req, peers).await;
        }

        // Discrete catch-up of the open chat.
        if last_refresh.elapsed() >= refresh {
            last_refresh = std::time::Instant::now();
            if let Some(open) = open_id {
                if let Some((title, peer_ref)) = peers.get(&open) {
                    if let Ok(msgs) = tg.get_messages(peer_ref, MESSAGE_LIMIT).await {
                        let sig = (
                            msgs.len(),
                            msgs.iter()
                                .map(|m| format!("{}:{}", m.id, m.text.len()))
                                .collect::<String>(),
                        );
                        if Some(&sig) != open_sig.as_ref() {
                            open_sig = Some(sig);
                            let rows = msgs
                                .into_iter()
                                .map(|m| MsgRow {
                                    id: m.id,
                                    text: m.text,
                                    out: m.out,
                                })
                                .collect();
                            let _ = ui_tx.send(UiMessage::Messages {
                                id: open,
                                title: title.clone(),
                                rows,
                            });
                        }
                    }
                }
            }
        }

        // Periodic session save (update state, peer cache).
        if last_save.elapsed() >= save_every {
            last_save = std::time::Instant::now();
            if let Err(e) = tg::session::save(&session, Path::new(SESSION_PATH)) {
                eprintln!("session save failed: {e}");
            }
        }

        // Await a server-pushed update (up to 200 ms).
        if let Ok(Ok(update)) = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            updates.next(),
        )
        .await
        {
            // During the grace period, drain the replay without showing it.
            if started.elapsed() >= grace {
                handle_update(ui_tx, update);
            }
        }
    }
}

/// Relays pushed updates (new messages, edits, deletions).
///
/// Unlike local sends (optimistic), a message sent with `out=true` from
/// ANOTHER device (e.g. the phone) must be displayed: we do not filter
/// outgoing — the UI deduplicates against the optimistic local send.
fn handle_update(ui_tx: &mpsc::UnboundedSender<UiMessage>, update: Update) {
    match update {
        Update::NewMessage(msg) => {
            if let Some(peer) = msg.peer() {
                let _ = ui_tx.send(UiMessage::NewMessage {
                    chat_id: peer.id().bot_api_dialog_id(),
                    id: msg.id(),
                    text: msg.text().to_string(),
                    out: msg.outgoing(),
                });
            }
        }
        Update::MessageEdited(msg) => {
            if let Some(peer) = msg.peer() {
                let _ = ui_tx.send(UiMessage::MessageEdited {
                    chat_id: peer.id().bot_api_dialog_id(),
                    id: msg.id(),
                    text: msg.text().to_string(),
                });
            }
        }
        Update::MessageDeleted(m) => {
            let _ = ui_tx.send(UiMessage::MessageDeleted {
                ids: m.messages().to_vec(),
            });
        }
        _ => {}
    }
}

/// Handles a UI request.
async fn handle_request(
    tg: &Telegram,
    ui_tx: &mpsc::UnboundedSender<UiMessage>,
    req: Request,
    peers: &HashMap<i64, (String, PeerRef)>,
) {
    match req {
        Request::OpenChat { id } => match peers.get(&id) {
            Some((title, peer_ref)) => match tg.get_messages(peer_ref, MESSAGE_LIMIT).await {
                Ok(messages) => {
                    let rows: Vec<MsgRow> = messages
                        .into_iter()
                        .map(|m| MsgRow {
                            id: m.id,
                            text: m.text,
                            out: m.out,
                        })
                        .collect();
                    let _ = ui_tx.send(UiMessage::Messages {
                        id,
                        title: title.clone(),
                        rows,
                    });
                }
                Err(e) => {
                    let _ = ui_tx.send(UiMessage::Error(format!(
                        "Could not load messages: {e}"
                    )));
                }
            },
            None => {
                let _ = ui_tx.send(UiMessage::Error("Unknown chat".to_string()));
            }
        },
        Request::SendMessage { id, text } => match peers.get(&id) {
            Some((_, peer_ref)) => {
                if let Err(e) = tg.send_message(peer_ref, &text).await {
                    let _ = ui_tx.send(UiMessage::Error(format!("Send failed: {e}")));
                }
            }
            None => {
                let _ = ui_tx.send(UiMessage::Error("Unknown chat".to_string()));
            }
        },
    }
}