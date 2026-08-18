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

pub fn cache_dir() -> PathBuf {
    data_dir().join("data")
}

/// Maps chat ids to already-cached avatar files (no download needed).
fn avatar_files() -> Vec<(i64, String)> {
    let dir = cache_dir().join("avatars");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| {
            let e = e.ok()?;
            if e.path().extension().and_then(|x| x.to_str()) != Some("jpg") {
                return None;
            }
            let id = e.file_name().to_string_lossy().trim_end_matches(".jpg").parse::<i64>().ok()?;
            Some((id, e.path().to_string_lossy().into_owned()))
        })
        .collect()
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
    let net_tx = req_tx.clone();
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
                        // Warm the avatar cache in the background.
                        let net_tx = net_tx.clone();
                        let ui_tx2 = ui_tx.clone();
                        let peers2 = peers.clone();
                        tokio::task::spawn(async move {
                            for id in peers2.keys() {
                                let _ = net_tx.send(Request::DownloadAvatar { chat_id: *id });
                            }
                            // Populate any already-downloaded avatars immediately.
                            for (id, path) in avatar_files() {
                                let _ = ui_tx2.send(UiMessage::AvatarReady {
                                    chat_id: id,
                                    path: Some(path),
                                });
                            }
                        });
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

/// A canned demo chat.
struct DemoChat {
    id: i64,
    title: &'static str,
    subtitle: &'static str,
    /// Seconds ago of the last message.
    last_ago: i32,
    unread: i32,
    hue: f32,
}

/// Generated demo assets: one avatar and one landscape photo per chat.
struct DemoAssets {
    avatars: HashMap<i64, String>,
    photos: HashMap<i64, String>,
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h = (h % 1.0) * 6.0;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r, g, b) = if h < 1.0 {
        (c, x, 0.0)
    } else if h < 2.0 {
        (x, c, 0.0)
    } else if h < 3.0 {
        (0.0, c, x)
    } else if h < 4.0 {
        (0.0, x, c)
    } else if h < 5.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    let m = l - c / 2.0;
    (
        ((r + m) * 255.0).clamp(0.0, 255.0) as u8,
        ((g + m) * 255.0).clamp(0.0, 255.0) as u8,
        ((b + m) * 255.0).clamp(0.0, 255.0) as u8,
    )
}

fn save_png(pixmap: &tiny_skia::Pixmap, path: &Path) {
    let _ = std::fs::create_dir_all(path.parent().expect("parent dir"));
    if let Ok(bytes) = pixmap.encode_png() {
        let _ = std::fs::write(path, bytes);
    }
}

/// Fills `pixmap` with a vertical gradient between two HSL colors.
fn gradient_fill(pm: &mut tiny_skia::Pixmap, h1: f32, s1: f32, l1: f32, h2: f32, s2: f32, l2: f32) {
    let (r1, g1, b1) = hsl_to_rgb(h1, s1, l1);
    let (r2, g2, b2) = hsl_to_rgb(h2, s2, l2);
    let (w, h) = (pm.width(), pm.height());
    for y in 0..h {
        let t = y as f32 / h as f32;
        let r = (r1 as f32 + (r2 as f32 - r1 as f32) * t) as u8;
        let g = (g1 as f32 + (g2 as f32 - g1 as f32) * t) as u8;
        let b = (b1 as f32 + (b2 as f32 - b1 as f32) * t) as u8;
        for x in 0..w {
            let off = (y * w + x) as usize * 4;
            pm.data_mut()[off..off + 4].copy_from_slice(&[r, g, b, 255]);
        }
    }
}

/// Generates demo avatars/photos with tiny-skia, written under `demo/`.
fn ensure_demo_assets(chats: &[DemoChat]) -> DemoAssets {
    use tiny_skia::{Color, Paint, Transform};

    let base = cache_dir().join("demo");
    let mut assets = DemoAssets {
        avatars: HashMap::new(),
        photos: HashMap::new(),
    };

    for chat in chats {
        // Avatar: 160x160 two-tone gradient with a white "sun" circle.
        let size = 160u32;
        let mut p = tiny_skia::Pixmap::new(size, size).expect("avatar pixmap");
        gradient_fill(&mut p, chat.hue, 0.6, 0.45, (chat.hue + 0.1) % 1.0, 0.6, 0.7);
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(255, 255, 255, 210));
        let circle = tiny_skia::PathBuilder::from_circle(size as f32 / 2.0, size as f32 / 2.0 + 10.0, 34.0)
            .expect("sun path");
        p.fill_path(&circle, &paint, tiny_skia::FillRule::Winding, Transform::identity(), None);
        let path = base.join("avatars").join(format!("{}.png", chat.id));
        save_png(&p, &path);
        assets.avatars.insert(chat.id, path.to_string_lossy().into_owned());

        // Landscape photo: 640x480 sky gradient + sun + water band.
        let (w, h) = (640u32, 480u32);
        let mut pm = tiny_skia::Pixmap::new(w, h).expect("photo pixmap");
        gradient_fill(&mut pm, (chat.hue + 0.5) % 1.0, 0.7, 0.6, (chat.hue + 0.85) % 1.0, 0.7, 0.15);
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(255, 230, 160, 240));
        let sun = tiny_skia::PathBuilder::from_circle(w as f32 * 0.75, h as f32 * 0.3, 60.0).expect("sun");
        pm.fill_path(&sun, &paint, tiny_skia::FillRule::Winding, Transform::identity(), None);
        // Dark water band (bottom).
        paint.set_color(Color::from_rgba8(20, 50, 100, 255));
        let band = tiny_skia::PathBuilder::from_rect(
            tiny_skia::Rect::from_xywh(0.0, h as f32 * 0.72, w as f32, h as f32 * 0.28).expect("band rect"),
        );
        pm.fill_path(&band, &paint, tiny_skia::FillRule::Winding, Transform::identity(), None);
        let path = base.join("media").join(format!("{}.png", chat.id));
        save_png(&pm, &path);
        assets.photos.insert(chat.id, path.to_string_lossy().into_owned());
    }

    assets
}

/// Offline "backend" for `--demo`: replays canned dialogs/messages, echoes
/// edits/deletes, and includes generated avatar/photo images so the full
/// image pipeline is exercised.
async fn serve_demo(
    ui_tx: &mpsc::UnboundedSender<UiMessage>,
    req_rx: &mut mpsc::UnboundedReceiver<Request>,
) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i32;

    let chats = vec![
        DemoChat { id: 1001, title: "Camille", subtitle: "Super ! à demain 👋", last_ago: 42, unread: 3, hue: 0.55 },
        DemoChat { id: 1002, title: "Rust Groupe", subtitle: "Thomas: novelle review du PR ?", last_ago: 7200, unread: 0, hue: 0.1 },
        DemoChat { id: 1003, title: "Canal Paysages", subtitle: "Coucher de soleil sur la mer 🏖", last_ago: 86400, unread: 0, hue: 0.95 },
        DemoChat { id: 1004, title: "Groupe Famille", subtitle: "Maman : tu passes quand ?", last_ago: 172800, unread: 12, hue: 0.3 },
        DemoChat { id: 1005, title: "Paris Bots", subtitle: "New version 2.4.0 released", last_ago: 604800, unread: 0, hue: 0.75 },
    ];

    let assets = ensure_demo_assets(&chats);
    let rows: Vec<ChatRow> = chats
        .iter()
        .map(|c| ChatRow {
            id: c.id,
            title: c.title.to_string(),
            subtitle: c.subtitle.to_string(),
            date: now - c.last_ago,
            unread: c.unread,
            avatar_path: assets.avatars.get(&c.id).cloned(),
        })
        .collect();
    let _ = ui_tx.send(UiMessage::Dialogs(rows));
    // Demo starts signed-in: the chat UI is the surface we want to exercise.
    let _ = ui_tx.send(UiMessage::LoginOk {
        name: "Demo".to_string(),
    });

    let photo_of = |chat: &DemoChat| assets.photos.get(&chat.id).cloned();
    let msgs_for = |id: i64| -> Vec<MsgRow> {
        let chat = chats.iter().find(|c| c.id == id);
        let photo = chat.and_then(|c| photo_of(c));
        match id {
            1001 => vec![
                MsgRow { id: 1, text: "Salut ! tu as vu ma nouvelle photo ?".to_string(), date: now - 700, out: false, photo: None, photo_path: None, read: false },
                MsgRow { id: 2, text: "Trop jolie ! tu l'as prise où ?".to_string(), date: now - 650, out: false, photo: None, photo_path: None, read: false },
                MsgRow { id: 3, text: "Sur la plage, au coucher du soleil.".to_string(), date: now - 600, out: true, photo: Some((640, 480)), photo_path: photo.clone(), read: true },
                MsgRow { id: 4, text: "Génial, on y va samedi ? 😎".to_string(), date: now - 300, out: false, photo: None, photo_path: None, read: false },
                MsgRow { id: 5, text: "Oui ! à demain 👋".to_string(), date: now - 42, out: true, photo: None, photo_path: None, read: true },
            ],
            1002 => vec![
                MsgRow { id: 1, text: "Qui veut présenter son projet vendredi ?".to_string(), date: now - 14400, out: false, photo: None, photo_path: None, read: true },
                MsgRow { id: 2, text: "Moi je peux, la CI passe enfin 🎉".to_string(), date: now - 10800, out: true, photo: None, photo_path: None, read: true },
                MsgRow { id: 3, text: "Soumettez le lien de la v0.2 ?".to_string(), date: now - 7200, out: false, photo: None, photo_path: None, read: true },
            ],
            1003 => vec![
                MsgRow { id: 1, text: "Photo du week-end dernier 🌄".to_string(), date: now - 90000, out: true, photo: Some((640, 480)), photo_path: photo, read: true },
                MsgRow { id: 2, text: "Magnifique, on la met en couverture !".to_string(), date: now - 89000, out: false, photo: None, photo_path: None, read: false },
            ],
            1004 => vec![
                MsgRow { id: 1, text: "Le repas de dimanche est déplacé".to_string(), date: now - 172800, out: false, photo: None, photo_path: None, read: true },
                MsgRow { id: 2, text: "Ok, on ramène le dessert 🍰".to_string(), date: now - 160000, out: true, photo: None, photo_path: None, read: false },
            ],
            1005 => vec![
                MsgRow { id: 1, text: "v2.4.0: nouvelle API de statut en ligne".to_string(), date: now - 604800, out: false, photo: None, photo_path: None, read: true },
                MsgRow { id: 2, text: "Merci pour l'update !".to_string(), date: now - 600000, out: true, photo: None, photo_path: None, read: true },
            ],
            _ => vec![],
        }
    };

    let mut edits: HashMap<i32, String> = HashMap::new();
    // Next message id handed to optimistic local sends (or the echo below).
    let mut next_id = 10000i32;
    // Simulate the peer replying in Camille's chat every ~12 s: a typing
    // burst of ~3 s followed by the message.
    let mut incoming_idx = 0usize;
    let incoming = [
        "Coucou !",
        "T'as vu l'actu ce matin ?",
        "Je suis en route, je passe te chercher dans 20 min.",
        "Pense à prendre le colis au passage 😉",
    ];
    // Next time an incoming message is delivered; typing starts 3 s earlier.
    let mut next_incoming = std::time::Instant::now() + std::time::Duration::from_secs(4);
    let mut typing_sent = false;

    loop {
        let mut pending: Vec<Request> =
            std::iter::from_fn(|| req_rx.try_recv().ok()).collect();
        for req in pending.drain(..) {
            match req {
                Request::OpenChat { id } => {
                    let rows = msgs_for(id)
                        .into_iter()
                        .map(|mut m| {
                            if let Some(text) = edits.get(&m.id) {
                                m.text = text.clone();
                            }
                            m
                        })
                        .collect();
                    let title = chats
                        .iter()
                        .find(|c| c.id == id)
                        .map(|c| c.title.to_string())
                        .unwrap_or_else(|| "Chat".to_string());
                    let _ = ui_tx.send(UiMessage::Messages { id, title, rows });
                }
                Request::SendMessage { id, text } => {
                    let nid = next_id;
                    next_id += 1;
                    // The server would echo the outgoing message back through
                    // an update; reply with it so the optimistic flow works.
                    let _ = ui_tx.send(UiMessage::NewMessage {
                        chat_id: id,
                        id: nid,
                        text,
                        date: now,
                        out: true,
                        photo: None,
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
                Request::Typing { .. } => {}
                _ => {}
            }
        }
        // Simulated peer activity in Camille's chat: typing burst → message.
        let now_inst = std::time::Instant::now();
        let typing_on = now_inst + std::time::Duration::from_secs(3) >= next_incoming
            && now_inst < next_incoming;
        if typing_on != typing_sent {
            typing_sent = typing_on;
            let _ = ui_tx.send(UiMessage::PeerTyping {
                chat_id: 1001,
                typing: typing_on,
            });
        }
        if now_inst >= next_incoming {
            let text = incoming[incoming_idx % incoming.len()].to_string();
            incoming_idx += 1;
            let _ = ui_tx.send(UiMessage::PeerTyping {
                chat_id: 1001,
                typing: false,
            });
            typing_sent = false;
            let _ = ui_tx.send(UiMessage::NewMessage {
                chat_id: 1001,
                id: next_id,
                text,
                date: now,
                out: false,
                photo: None,
            });
            next_id += 1;
            next_incoming = now_inst + std::time::Duration::from_secs(12);
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
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
    // Downloaded photo paths keyed by (chat id, message id), so periodic
    // refreshes keep showing them instead of resetting to None.
    let mut photo_paths: HashMap<(i64, i32), String> = HashMap::new();

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
            handle_request(&tg, ui_tx, req, peers, &mut photo_paths).await;
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
                                .iter()
                                .map(|m| {
                                    let photo = m.media.map(|k| match k {
                                        tg::model::MediaKind::Photo { width, height } => {
                                            (width, height)
                                        }
                                    });
                                    MsgRow {
                                        id: m.id,
                                        text: m.text.clone(),
                                        date: m.date,
                                        out: m.out,
                                        photo,
                                        photo_path: photo_paths.get(&(open, m.id)).cloned(),
                                        read: false,
                                    }
                                })
                                .collect();
                            let _ = ui_tx.send(UiMessage::Messages {
                                id: open,
                                title: title.clone(),
                                rows,
                            });
                            // Warm photo thumbnails for any new photo messages.
                            for m in &msgs {
                                if m.media.is_some() {
                                    fetch_photo(&tg, ui_tx, peers, open, m.id, &mut photo_paths).await;
                                }
                            }
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
    photo_paths: &mut HashMap<(i64, i32), String>,
) {
    match req {
        Request::MarkRead { id } => {
            if let Some((_, peer_ref)) = peers.get(&id) {
                if tg.mark_read(peer_ref).await.is_ok() {
                    let _ = ui_tx.send(UiMessage::ChatRead { id });
                }
            }
        }
        Request::Typing { id, typing } => {
            if let Some((_, peer_ref)) = peers.get(&id) {
                let _ = tg.set_typing(peer_ref, typing).await;
            }
        }
        Request::OpenChat { id } => match peers.get(&id) {
            Some((title, peer_ref)) => match tg.get_messages(peer_ref, MESSAGE_LIMIT).await {
                Ok(messages) => {
                    let rows: Vec<MsgRow> = messages
                        .into_iter()
                        .map(|m| {
                            let photo = m.media.map(|k| match k {
                                tg::model::MediaKind::Photo { width, height } => {
                                    (width, height)
                                }
                            });
                            MsgRow {
                                id: m.id,
                                text: m.text,
                                date: m.date,
                                out: m.out,
                                photo,
                                photo_path: photo_paths.get(&(id, m.id)).cloned(),
                                read: false,
                            }
                        })
                        .collect();
                    let _ = ui_tx.send(UiMessage::Messages {
                        id,
                        title: title.clone(),
                        rows: rows.clone(),
                    });
                    // Fetch photo thumbnails for any photo messages in the background.
                    for m in rows {
                        if m.photo.is_some() {
                            fetch_photo(tg, ui_tx, peers, id, m.id, photo_paths).await;
                        }
                    }
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

/// Downloads a message's photo thumbnail (cached), records it in `photo_paths`
/// and notifies the UI with the on-disk path.
async fn fetch_photo(
    tg: &Telegram,
    ui_tx: &mpsc::UnboundedSender<UiMessage>,
    peers: &HashMap<i64, (String, PeerRef)>,
    chat_id: i64,
    msg_id: i32,
    photo_paths: &mut HashMap<(i64, i32), String>,
) {
    if photo_paths.contains_key(&(chat_id, msg_id)) {
        return;
    }
    let Some((_, peer_ref)) = peers.get(&chat_id) else { return };
    let dir = cache_dir().join("media").join(chat_id.to_string());
    let path = match tg.download_photo(peer_ref, msg_id, &dir).await {
        Ok(Some(p)) => Some(p.to_string_lossy().into_owned()),
        _ => None,
    };
    if let Some(p) = &path {
        photo_paths.insert((chat_id, msg_id), p.clone());
    }
    let _ = ui_tx.send(UiMessage::PhotoReady {
        chat_id,
        msg_id,
        path,
    });
}

/// Re-exported so `main.rs` can type the sender.
pub type UnboundedSender<T> = mpsc::UnboundedSender<T>;
