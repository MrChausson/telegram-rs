//! Main binary: bridge between the network runtime (tokio) and the window (winit).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use grammers_client::client::{LoginToken, PasswordToken, SignInError, UpdatesConfiguration};
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

const ENV_FILE: &str = ".env";
const SESSION_FILE: &str = ".tg.session";
const MESSAGE_LIMIT: usize = 200;

/// Compiled-in default API credentials, injected at build time from the
/// `TG_API_ID` / `TG_API_HASH` secrets (GitHub Actions). The end user only
/// sees the in-app login screen; `.env` / env vars still override these.
const DEFAULT_API_ID: Option<&str> = option_env!("TG_API_ID");
const DEFAULT_API_HASH: Option<&str> = option_env!("TG_API_HASH");

/// Per-user data directory holding `.env`, `.tg.session` and the cache so the
/// binary works whichever directory it is launched from. Falls back to the
/// current directory when developing (a `.env` present there).
fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("TG_DATA_DIR") {
        return PathBuf::from(p);
    }
    // A `.env` in the current directory means "dev mode": keep the old
    // behavior for the repo checkout.
    if std::fs::metadata(ENV_FILE).is_ok() {
        return PathBuf::from(".");
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let base: Option<PathBuf> = {
        #[cfg(target_os = "linux")]
        {
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .or_else(|| home.clone().map(|h| h.join(".local/share")))
        }
        #[cfg(target_os = "macos")]
        {
            home.clone().map(|h| h.join("Library/Application Support"))
        }
        #[cfg(target_os = "windows")]
        {
            std::env::var_os("APPDATA").map(PathBuf::from)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            home
        }
    };
    base.map(|b| b.join("tg"))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn env_path() -> PathBuf {
    data_dir().join(ENV_FILE)
}

fn session_path() -> PathBuf {
    data_dir().join(SESSION_FILE)
}

fn cache_dir() -> PathBuf {
    data_dir().join("data")
}

fn load_env(path: &Path) -> HashMap<String, String> {
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

fn env_var(env: &HashMap<String, String>, name: &str) -> Option<String> {
    env.get(name)
        .cloned()
        .or_else(|| std::env::var(name).ok())
        .filter(|v| !v.is_empty())
}

fn api_id(env: &HashMap<String, String>) -> anyhow::Result<i32> {
    let raw = env_var(env, "API_ID").or_else(|| DEFAULT_API_ID.map(String::from));
    let Some(raw) = raw else {
        anyhow::bail!("API_ID is missing — embed it at build time or add it to .env");
    };
    raw.parse()
        .map_err(|_| anyhow::anyhow!("API_ID must be a number"))
}

fn api_hash(env: &HashMap<String, String>) -> Option<String> {
    env_var(env, "API_HASH").or_else(|| DEFAULT_API_HASH.map(String::from))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let env = load_env(&env_path());
    let api_id: i32 = api_id(&env)?;
    let api_hash = api_hash(&env);
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

    // A pristine home for the session and the cache, regardless of the
    // directory the binary is launched from.
    if let Some(dir) = data_dir().parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::create_dir_all(cache_dir());
    let session_path = session_path();

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
            let session = Arc::new(load_or_new(&session_path));
            let tg = match Telegram::connect(session.clone(), api_id).await {
                Ok(tg) => tg,
                Err(e) => {
                    let _ = ui_tx.send(UiMessage::Error(format!("Could not connect: {e}")));
                    std::future::pending::<()>().await;
                    return;
                }
            };

            if !tg.is_authorized().await.unwrap_or(false) {
                serve_login(&tg, api_hash.as_deref(), &ui_tx, &mut req_rx).await;
                let _ = tg::session::save(&session, &session_path);
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
                                date: d.last_date.unwrap_or(0),
                                unread: d.unread_count,
                                avatar_path: None,
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

/// Resolves the sign-in flow, fielding the login requests until the account
/// is authorized (or the user closes the window).
async fn serve_login(
    tg: &Telegram,
    api_hash: Option<&str>,
    ui_tx: &mpsc::UnboundedSender<UiMessage>,
    req_rx: &mut mpsc::UnboundedReceiver<Request>,
) {
    let mut token: Option<LoginToken> = None;
    let mut password: Option<PasswordToken> = None;

    loop {
        let pending: Vec<Request> = std::iter::from_fn(|| req_rx.try_recv().ok()).collect();
        for req in pending {
            match req {
                Request::LoginPhone { phone } => {
                    let Some(hash) = api_hash else {
                        let _ = ui_tx.send(UiMessage::Error(
                            "API_HASH is missing — rebuild with TG_API_HASH or add it to .env"
                                .to_string(),
                        ));
                        continue;
                    };
                    match tg.request_login_code(hash, &phone).await {
                        Ok(t) => {
                            token = Some(t);
                            password = None;
                            let _ = ui_tx.send(UiMessage::LoginCodeRequired);
                        }
                        Err(e) => {
                            let _ = ui_tx.send(UiMessage::Error(format!(
                                "Could not send the code: {e}"
                            )));
                        }
                    }
                }
                Request::LoginCode { code } => {
                    let Some(t) = token.as_ref() else {
                        let _ = ui_tx.send(UiMessage::Error(
                            "No code request in progress — enter your phone again".to_string(),
                        ));
                        continue;
                    };
                    match tg.sign_in(t, &code).await {
                        Ok(Ok(())) => {
                            let name = me_name(tg).await;
                            let _ = ui_tx.send(UiMessage::LoginOk { name });
                            return;
                        }
                        Ok(Err(SignInError::PasswordRequired(tok))) => {
                            let hint = tok.hint().map(|s| s.to_string()).unwrap_or_default();
                            password = Some(tok);
                            let _ = ui_tx.send(UiMessage::LoginPasswordRequired { hint });
                        }
                        Ok(Err(SignInError::InvalidCode)) => {
                            let _ = ui_tx.send(UiMessage::Error(
                                "Invalid code, try again".to_string(),
                            ));
                        }
                        Ok(Err(SignInError::SignUpRequired)) => {
                            let _ = ui_tx.send(UiMessage::Error(
                                "This number is not registered. Sign up in the official Telegram app first (free)"
                                    .to_string(),
                            ));
                        }
                        Ok(Err(e)) => {
                            let _ = ui_tx.send(UiMessage::Error(format!("Sign in failed: {e}")));
                        }
                        Err(e) => {
                            let _ = ui_tx.send(UiMessage::Error(format!("Sign in failed: {e}")));
                        }
                    }
                }
                Request::LoginPassword { password: password_input } => {
                    let Some(tok) = password.take() else {
                        let _ = ui_tx.send(UiMessage::Error(
                            "No password request in progress".to_string(),
                        ));
                        continue;
                    };
                    match tg.check_password(tok, &password_input).await {
                        Ok(Ok(())) => {
                            let name = me_name(tg).await;
                            let _ = ui_tx.send(UiMessage::LoginOk { name });
                            return;
                        }
                        Ok(Err(new_token)) => {
                            // Wrong password: Telegram gave us a fresh token
                            // to try again.
                            password = Some(new_token);
                            let _ = ui_tx.send(UiMessage::Error(
                                "Wrong password, try again".to_string(),
                            ));
                        }
                        Err(e) => {
                            let _ = ui_tx.send(UiMessage::Error(format!(
                                "2FA failed: {e}"
                            )));
                        }
                    }
                }
                _ => {}
            }
        }
        // Back off briefly: the winit loop sends requests lazily.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Best-effort display name of the signed-in account.
async fn me_name(tg: &Telegram) -> String {
    match tg.client().get_me().await {
        Ok(me) => me
            .first_name()
            .map(|f| f.to_string())
            .unwrap_or_else(|| "your account".to_string()),
        Err(_) => "your account".to_string(),
    }
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
        let mut pending: Vec<Request> =
            std::iter::from_fn(|| req_rx.try_recv().ok()).collect();
        // Open the requested chat first: thumbnails/downloads behind it in
        // the queue must not starve the history load.
        pending.sort_by_key(|r| !matches!(r, Request::OpenChat { .. }));
        for req in pending {
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
                                    date: m.date,
                                    out: m.out,
                                    photo: m.media.map(|k| match k {
                                        tg::model::MediaKind::Photo { width, height } => (width, height),
                                    }),
                                    photo_path: None,
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
            if let Err(e) = tg::session::save(&session, &session_path()) {
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
                    date: msg.date().timestamp() as i32,
                    out: msg.outgoing(),
                    photo: tg::client::media_kind(msg.media().as_ref()).and_then(|k| match k {
                        tg::model::MediaKind::Photo { width, height } => Some((width, height)),
                    }),
                });
            }
        }
        Update::MessageEdited(msg) => {
            if let Some(peer) = msg.peer() {
                let _ = ui_tx.send(UiMessage::MessageEdited {
                    chat_id: peer.id().bot_api_dialog_id(),
                    id: msg.id(),
                    text: msg.text().to_string(),
                    date: msg.date().timestamp() as i32,
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
        Request::OpenChat { id } => {
            match peers.get(&id) {
            Some((title, peer_ref)) => match tg.get_messages(peer_ref, MESSAGE_LIMIT).await {
                Ok(messages) => {
                    let rows: Vec<MsgRow> = messages
                        .into_iter()
                        .map(|m| MsgRow {
                            id: m.id,
                            text: m.text,
                            date: m.date,
                            out: m.out,
                            photo: m.media.map(|k| match k {
                                tg::model::MediaKind::Photo { width, height } => (width, height),
                            }),
                            photo_path: None,
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
        }
        },
        Request::DownloadPhoto { chat_id, msg_id } => match peers.get(&chat_id) {
            Some((_, peer_ref)) => {
                let dir = cache_dir().join("media").join(chat_id.to_string());
                let path = match tg.download_photo(peer_ref, msg_id, &dir).await {
                    Ok(p) => p.map(|p| p.to_string_lossy().into_owned()),
                    Err(_) => None,
                };
                let _ = ui_tx.send(UiMessage::PhotoReady {
                    chat_id,
                    msg_id,
                    path,
                });
            }
            None => {}
        },
        Request::DownloadAvatar { chat_id } => match peers.get(&chat_id) {
            Some((_, peer_ref)) => {
                let dir = cache_dir().join("avatars");
                let path = match tg.download_avatar(peer_ref, &dir).await {
                    Ok(p) => p.map(|p| p.to_string_lossy().into_owned()),
                    Err(_) => None,
                };
                let _ = ui_tx.send(UiMessage::AvatarReady { chat_id, path });
            }
            None => {}
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
        // Sign-in requests are consumed by `serve_login`; nothing to do here.
        Request::LoginPhone { .. } | Request::LoginCode { .. } | Request::LoginPassword { .. } => {}
    }
}