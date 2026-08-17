//! Network runtime: a tokio thread (current_thread) that talks MTProto, plus
//! the Iced `Subscription` that streams `UiMessage`s into the UI. Adapted from
//! the custom `app` binary so the exact same `Request`/`UiMessage` bridge is
//! reused.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use grammers_client::client::{LoginToken, PasswordToken, SignInError, UpdatesConfiguration};
use grammers_client::update::Update;
use grammers_session::types::PeerRef;
use tg::client::Telegram;
use tg::session::load_or_new;
use tokio::sync::mpsc;

use crate::bridge::{ChatRow, MsgRow, Request, UiMessage};

const ENV_FILE: &str = ".env";
const SESSION_FILE: &str = ".tg.session";
const MESSAGE_LIMIT: usize = 200;

const DEFAULT_API_ID: Option<&str> = option_env!("TG_API_ID");
const DEFAULT_API_HASH: Option<&str> = option_env!("TG_API_HASH");

/// Receiver end of the UI feed, taken once by the Iced subscription.
static UI_RX: std::sync::Mutex<Option<mpsc::UnboundedReceiver<UiMessage>>> =
    std::sync::Mutex::new(None);

/// Per-user data directory (same logic as the custom `app`).
pub fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("TG_DATA_DIR") {
        return PathBuf::from(p);
    }
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

pub fn env_path() -> PathBuf {
    data_dir().join(ENV_FILE)
}

pub fn session_path() -> PathBuf {
    data_dir().join(SESSION_FILE)
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

pub fn api_id(env: &HashMap<String, String>) -> anyhow::Result<i32> {
    let raw = env_var(env, "API_ID").or_else(|| DEFAULT_API_ID.map(String::from));
    let Some(raw) = raw else {
        anyhow::bail!("API_ID is missing — embed it at build time or add it to .env");
    };
    raw.parse()
        .map_err(|_| anyhow::anyhow!("API_ID must be a number"))
}

pub fn api_hash(env: &HashMap<String, String>) -> Option<String> {
    env_var(env, "API_HASH").or_else(|| DEFAULT_API_HASH.map(String::from))
}

/// Spawns the network thread and returns the request sender for the UI.
///
/// `demo=true` runs the canned offline backend (with a photo message so image
/// rendering can be tested too); `false` connects to the real account.
pub fn spawn_network(demo: bool) -> UnboundedSender<Request> {
    let (ui_tx, ui_rx) = mpsc::unbounded_channel::<UiMessage>();
    let (req_tx, mut req_rx) = mpsc::unbounded_channel::<Request>();
    {
        let mut guard = UI_RX.lock().expect("UI_RX lock");
        *guard = Some(ui_rx);
    }

    std::thread::Builder::new()
        .stack_size(1 << 20)
        .name("network".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(async move {
                if demo {
                    serve_demo(&ui_tx, &mut req_rx).await;
                    std::future::pending::<()>().await;
                }
                let session_path = session_path();
                let session = Arc::new(load_or_new(&session_path));
                let env = load_env(&env_path());
                let api_id = api_id(&env).expect("API_ID");
                let api_hash = api_hash(&env);
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
        })
        .expect("spawn network thread");

    req_tx
}

/// Iced subscription: a stream over the network's `UiMessage` channel.
pub fn network_subscription() -> iced::Subscription<crate::Message> {
    use iced::futures::stream::{poll_fn, Stream};
    use std::task::Poll;

    fn stream() -> impl Stream<Item = crate::Message> {
        // Take the receiver out of the static once; the app only runs one
        // subscription instance.
        let mut rx = UI_RX
            .lock()
            .expect("UI_RX lock")
            .take()
            .expect("UI_RX initialized before app run");
        poll_fn(move |cx| match rx.poll_recv(cx) {
            Poll::Ready(Some(msg)) => Poll::Ready(Some(crate::Message::Ui(msg))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        })
    }
    iced::Subscription::run(stream)
}

/// Offline "backend" for `--demo`: replays canned dialogs/messages, echoes
/// edits/deletes, and includes one photo message so image rendering is tested.
async fn serve_demo(
    ui_tx: &mpsc::UnboundedSender<UiMessage>,
    req_rx: &mut mpsc::UnboundedReceiver<Request>,
) {
    const DEMO_CHAT: i64 = 42;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i32;

    let rows = vec![
        ChatRow {
            id: DEMO_CHAT,
            title: "Démo TG".to_string(),
            subtitle: "dis bonjour 👋 aux coches".to_string(),
            date: now - 30,
            unread: 3,
            avatar_path: None,
        },
        ChatRow {
            id: 43,
            title: "Liste de courses".to_string(),
            subtitle: "Pense à acheter du lait".to_string(),
            date: now - 7200,
            unread: 1,
            avatar_path: None,
        },
        ChatRow {
            id: 44,
            title: "Canal Photos".to_string(),
            subtitle: "Les vacances d'été 🏖".to_string(),
            date: now - 86400,
            unread: 0,
            avatar_path: None,
        },
    ];
    let _ = ui_tx.send(UiMessage::Dialogs(rows));
    // Demo starts signed-in: the chat UI is the surface we want to exercise.
    let _ = ui_tx.send(UiMessage::LoginOk {
        name: "Demo".to_string(),
    });

    // One message carries a photo (dimensions only; the demo never downloads),
    // so the image widget path is exercised.
    let messages = vec![
        MsgRow {
            id: 1,
            text: "Hello, this is a longer incoming message designed to wrap across several lines".to_string(),
            date: now - 3600,
            out: false,
            photo: None,
            photo_path: None,
            read: false,
        },
        MsgRow {
            id: 2,
            text: "First reply from me".to_string(),
            date: now - 3000,
            out: true,
            photo: Some((640, 480)),
            photo_path: None,
            read: true,
        },
        MsgRow {
            id: 3,
            text: "Second one, shorter".to_string(),
            date: now - 2000,
            out: true,
            photo: None,
            photo_path: None,
            read: false,
        },
        MsgRow {
            id: 4,
            text: "An incoming note".to_string(),
            date: now - 1000,
            out: false,
            photo: None,
            photo_path: None,
            read: false,
        },
    ];

    let mut edits: HashMap<i32, String> = HashMap::new();
    loop {
        let mut pending: Vec<Request> =
            std::iter::from_fn(|| req_rx.try_recv().ok()).collect();
        for req in pending.drain(..) {
            match req {
                Request::OpenChat { id } => {
                    let _ = ui_tx.send(UiMessage::Messages {
                        id,
                        title: "Démo TG".to_string(),
                        rows: messages
                            .iter()
                            .map(|m| {
                                let mut m = m.clone();
                                if let Some(text) = edits.get(&m.id) {
                                    m.text = text.clone();
                                }
                                m
                            })
                            .collect(),
                    });
                }
                Request::EditMessage { id, msg_id, text } => {
                    edits.insert(msg_id, text.clone());
                    let _ = ui_tx.send(UiMessage::MessageEdited {
                        chat_id: id,
                        id: msg_id,
                        text,
                        date: now,
                    });
                }
                Request::DeleteMessage { id: _, msg_id } => {
                    edits.remove(&msg_id);
                    let _ = ui_tx.send(UiMessage::MessageDeleted { ids: vec![msg_id] });
                }
                Request::MarkRead { id } => {
                    let _ = ui_tx.send(UiMessage::ChatRead { id });
                }
                _ => {}
            }
        }
        tokio::task::yield_now().await;
    }
}

/// Sign-in flow (phone → code → 2FA).
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
                            let _ = ui_tx.send(UiMessage::LoginOk { name: String::new() });
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
                Request::LoginPassword { password: pwd } => {
                    let Some(t) = password.take() else { continue };
                    match tg.check_password(t, &pwd).await {
                        Ok(Ok(())) => {
                            let _ = ui_tx.send(UiMessage::LoginOk { name: String::new() });
                            return;
                        }
                        Ok(Err(new_token)) => {
                            password = Some(new_token);
                            let _ = ui_tx.send(UiMessage::Error(
                                "Wrong password, try again".to_string(),
                            ));
                        }
                        Err(e) => {
                            let _ = ui_tx.send(UiMessage::Error(format!("2FA failed: {e}")));
                        }
                    }
                }
                _ => {}
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
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

    let refresh = std::time::Duration::from_secs(15);
    let mut last_refresh = std::time::Instant::now();
    let mut open_id: Option<i64> = None;
    let mut open_sig: Option<(usize, String)> = None;

    let save_every = std::time::Duration::from_secs(30);
    let mut last_save = std::time::Instant::now();
    let session = tg.session().clone();

    let started = std::time::Instant::now();
    let grace = std::time::Duration::from_secs(10);

    loop {
        let mut pending: Vec<Request> =
            std::iter::from_fn(|| req_rx.try_recv().ok()).collect();
        pending.sort_by_key(|r| !matches!(r, Request::OpenChat { .. }));
        for req in pending {
            if let Request::OpenChat { id } = req {
                open_id = Some(id);
                open_sig = None;
            }
            handle_request(&tg, ui_tx, req, peers).await;
        }

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
                                        tg::model::MediaKind::Photo { width, height } => {
                                            (width, height)
                                        }
                                    }),
                                    photo_path: None,
                                    read: false,
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

        if last_save.elapsed() >= save_every {
            last_save = std::time::Instant::now();
            if let Err(e) = tg::session::save(&session, &session_path()) {
                eprintln!("session save failed: {e}");
            }
        }

        if let Ok(Ok(update)) = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            updates.next(),
        )
        .await
        {
            if started.elapsed() >= grace {
                handle_update(ui_tx, update);
            }
        }
    }
}

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
        Update::Raw(raw) => match &*raw {
            grammers_client::tl::enums::Update::ReadHistoryInbox(u) => {
                let id = grammers_session::types::PeerId::from(u.peer.clone()).bot_api_dialog_id();
                let _ = ui_tx.send(UiMessage::UnreadCount {
                    chat_id: id,
                    count: u.still_unread_count.max(0),
                });
            }
            grammers_client::tl::enums::Update::ReadHistoryOutbox(u) => {
                let id = grammers_session::types::PeerId::from(u.peer.clone()).bot_api_dialog_id();
                let _ = ui_tx.send(UiMessage::ChatRead { id });
                let _ = ui_tx.send(UiMessage::OutboxRead {
                    chat_id: id,
                    max_id: u.max_id,
                });
            }
            grammers_client::tl::enums::Update::UserTyping(u) => {
                let _ = ui_tx.send(UiMessage::PeerTyping {
                    chat_id: grammers_session::types::PeerId::user(u.user_id).bot_api_dialog_id(),
                    typing: !action_is_cancel(&u.action),
                });
            }
            grammers_client::tl::enums::Update::ChatUserTyping(u) => {
                let _ = ui_tx.send(UiMessage::PeerTyping {
                    chat_id: grammers_session::types::PeerId::chat(u.chat_id).bot_api_dialog_id(),
                    typing: !action_is_cancel(&u.action),
                });
            }
            grammers_client::tl::enums::Update::ChannelUserTyping(u) => {
                let _ = ui_tx.send(UiMessage::PeerTyping {
                    chat_id: grammers_session::types::PeerId::channel(u.channel_id)
                        .bot_api_dialog_id(),
                    typing: !action_is_cancel(&u.action),
                });
            }
            _ => {}
        },
        _ => {}
    }
}

fn action_is_cancel(action: &grammers_client::tl::enums::SendMessageAction) -> bool {
    matches!(
        action,
        grammers_client::tl::enums::SendMessageAction::SendMessageCancelAction
    )
}

async fn handle_request(
    tg: &Telegram,
    ui_tx: &mpsc::UnboundedSender<UiMessage>,
    req: Request,
    peers: &HashMap<i64, (String, PeerRef)>,
) {
    match req {
        Request::MarkRead { id } => {
            if let Some((_, peer_ref)) = peers.get(&id) {
                if tg.mark_read(peer_ref).await.is_ok() {
                    let _ = ui_tx.send(UiMessage::ChatRead { id });
                }
            }
        }
        Request::OpenChat { id } => match peers.get(&id) {
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
                                tg::model::MediaKind::Photo { width, height } => {
                                    (width, height)
                                }
                            }),
                            photo_path: None,
                            read: false,
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
        Request::EditMessage { id, msg_id, text } => match peers.get(&id) {
            Some((_, peer_ref)) => {
                if let Err(e) = tg.edit_message(peer_ref, msg_id, &text).await {
                    let _ = ui_tx.send(UiMessage::Error(format!("Edit failed: {e}")));
                }
            }
            None => {
                let _ = ui_tx.send(UiMessage::Error("Unknown chat".to_string()));
            }
        },
        Request::DeleteMessage { id, msg_id } => match peers.get(&id) {
            Some((_, peer_ref)) => {
                if let Err(e) = tg.delete_message(peer_ref, msg_id).await {
                    let _ = ui_tx.send(UiMessage::Error(format!("Delete failed: {e}")));
                }
            }
            None => {
                let _ = ui_tx.send(UiMessage::Error("Unknown chat".to_string()));
            }
        },
        Request::LoginPhone { .. } | Request::LoginCode { .. } | Request::LoginPassword { .. } => {}
    }
}

/// Re-exported so `main.rs` can type the sender.
pub type UnboundedSender<T> = mpsc::UnboundedSender<T>;
