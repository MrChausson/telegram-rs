//! Network runtime: a tokio thread (current_thread) that talks MTProto, plus
//! the Iced `Subscription` that streams `UiMessage`s into the UI. Adapted from
//! the custom `app` binary so the exact same `Request`/`UiMessage` bridge is
//! reused.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tokio::sync::Semaphore;

use grammers_client::client::{LoginToken, PasswordToken, SignInError, UpdatesConfiguration};
use grammers_client::update::Update;
use grammers_session::types::PeerRef;
use grammers_session::updates::UpdatesLike;
use tg::client::Telegram;
use tg::session::load_or_new;
use tokio::sync::mpsc;

use crate::bridge::{
    ChatDetail, ChatKind, ChatRow, DocKind, DocMeta, MsgRow, ParticipantRole, ParticipantRow,
    Request, SearchHit, StickerMeta, StickerSetBridge, UiMessage,
};

const ENV_FILE: &str = ".env";
const SESSION_FILE: &str = ".tg.session";
const MESSAGE_LIMIT: usize = 200;
/// Cap on search results returned per query.
const SEARCH_LIMIT: usize = 30;
/// Minimum delay between two re-runs of the *same* search query (the UI sends
/// one `Request::Search` per keystroke; MTProto round-trips are expensive).
const SEARCH_THROTTLE: std::time::Duration = std::time::Duration::from_millis(400);
/// Max simultaneous MTProto transfers (avatars + photo thumbnails). Kept small
/// so interactive requests share the connection without waiting on a backlog.
const DOWNLOAD_CONCURRENCY: usize = 4;

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
/// rendering can be tested too); `big=true` seeds the first demo chat with a
/// ~400-message history so scroll performance can be exercised (the 5-message
/// demo chat never reproduces the lag of a real history). `false` connects to
/// the real account.
pub fn spawn_network(demo: bool, big: bool) -> UnboundedSender<Request> {
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
                    serve_demo(&ui_tx, &mut req_rx, big).await;
                    std::future::pending::<()>().await;
                }
                let session_path = session_path();
                let session = Arc::new(load_or_new(&session_path));
                let env = load_env(&env_path());
                let api_id = api_id(&env).expect("API_ID");
                let api_hash = api_hash(&env);
                let mut tg = match Telegram::connect(session.clone(), api_id).await {
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
                let updates_rx = tg.take_updates();
                let tg = Arc::new(tg);

                let mut peers: HashMap<i64, (String, PeerRef)> = HashMap::new();
                match refresh_dialogs(&tg, &ui_tx, &mut peers).await {
                    Ok(()) => {
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
                        serve(tg, updates_rx, &ui_tx, &mut req_rx, &mut peers).await;
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

/// A canned demo chat. Titles are owned (not `&'static str`) so the
/// create/rename demo handlers can add or edit chats at runtime.
struct DemoChat {
    id: i64,
    title: String,
    subtitle: String,
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

/// Generates a minimal valid WAV (1 s 440 Hz 16-bit mono PCM) — the canned
/// stand-in for downloaded voice notes so the demo exercises playback
/// end-to-end. Returns the path written.
fn demo_voice_wav(path: &std::path::Path) -> String {
    let rate = 8000u32;
    let secs = 1u32;
    let data_len = rate * secs; // 16-bit mono = 2 bytes/sample
    let mut bytes = Vec::with_capacity(44 + (data_len * 2) as usize);
    // RIFF header.
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len * 2).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    // fmt chunk.
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 2).to_le_bytes()); // byte rate
    bytes.extend_from_slice(&2u16.to_le_bytes()); // block align
    bytes.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    // data chunk.
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data_len * 2).to_le_bytes());
    for i in 0..data_len {
        let t = i as f32 / rate as f32;
        let sample = (t * 440.0 * std::f32::consts::TAU).sin();
        let v = (sample * i16::MAX as f32).round() as i16;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    let _ = std::fs::create_dir_all(path);
    let wav = path.join("voice.wav");
    let _ = std::fs::write(&wav, bytes);
    wav.to_string_lossy().into_owned()
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

/// One generated demo sticker document.
struct DemoSticker {
    doc_id: i64,
    access_hash: i64,
    alt: String,
    path: String,
}

/// A generated demo sticker pack (stand-in for `messages.getAllStickers`).
struct DemoStickerSet {
    title: String,
    short_name: String,
    docs: Vec<DemoSticker>,
}

/// Generates two demo sticker packs of 10 stickers each as 512x512 PNGs
/// (simple tiny-skia shapes on transparent background, one shape family per
/// index). Written under `demo/stickers/<set>/`; deterministic, so repeated
/// runs reuse the same files.
fn ensure_demo_stickers() -> Vec<DemoStickerSet> {
    use tiny_skia::{Color, Paint, Transform};

    let alts = [
        "😀", "🎉", "❤️", "⭐", "🌙", "🔥", "🍀", "💎", "🎵", "⚡",
    ];
    let palettes: [[u8; 3]; 10] = [
        [244, 67, 54],
        [233, 30, 99],
        [156, 39, 176],
        [103, 58, 183],
        [33, 150, 243],
        [0, 188, 212],
        [76, 175, 80],
        [205, 220, 57],
        [255, 152, 0],
        [121, 85, 72],
    ];
    let sets = [
        ("Happy Blocks", "happy_blocks"),
        ("Rust Buddies", "rust_buddies"),
    ];
    let mut out = Vec::new();
    for (si, (title, short)) in sets.iter().enumerate() {
        let mut docs = Vec::new();
        for (i, alt) in alts.iter().enumerate() {
            let size = 512u32;
            let c = palettes[(i + si * 3) % palettes.len()];
            let mut pm = tiny_skia::Pixmap::new(size, size).expect("sticker pixmap");
            let paint = |a: u8| {
                let mut p = Paint::default();
                p.set_color(Color::from_rgba8(c[0], c[1], c[2], a));
                p
            };
            let fill = |pm: &mut tiny_skia::Pixmap, path: &tiny_skia::Path, a: u8| {
                pm.fill_path(path, &paint(a), tiny_skia::FillRule::Winding, Transform::identity(), None);
            };
            let cx = size as f32 / 2.0;
            let cy = size as f32 / 2.0;
            let r = size as f32 * 0.36;
            match i % 5 {
                0 => {
                    // Filled disc + white ring highlight.
                    if let Some(p) = tiny_skia::PathBuilder::from_circle(cx, cy, r) {
                        fill(&mut pm, &p, 255);
                    }
                    if let Some(p) = tiny_skia::PathBuilder::from_circle(cx - r * 0.3, cy - r * 0.35, r * 0.22) {
                        fill(&mut pm, &p, 140);
                    }
                }
                1 => {
                    // Five-point star.
                    let mut pb = tiny_skia::PathBuilder::new();
                    for k in 0..10 {
                        let ang = -std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::PI / 5.0;
                        let rad = if k % 2 == 0 { r } else { r * 0.45 };
                        let (x, y) = (cx + rad * ang.cos(), cy + rad * ang.sin());
                        if k == 0 {
                            pb.move_to(x, y);
                        } else {
                            pb.line_to(x, y);
                        }
                    }
                    pb.close();
                    if let Some(p) = pb.finish() {
                        fill(&mut pm, &p, 235);
                    }
                }
                2 => {
                    // Heart: two discs + a triangle.
                    if let (Some(l), Some(rr)) = (
                        tiny_skia::PathBuilder::from_circle(cx - r * 0.45, cy - r * 0.25, r * 0.48),
                        tiny_skia::PathBuilder::from_circle(cx + r * 0.45, cy - r * 0.25, r * 0.48),
                    ) {
                        let mut pb = tiny_skia::PathBuilder::new();
                        pb.move_to(cx - r * 0.9, cy - r * 0.05);
                        pb.line_to(cx + r * 0.9, cy - r * 0.05);
                        pb.line_to(cx, cy + r);
                        pb.close();
                        fill(&mut pm, &l, 240);
                        fill(&mut pm, &rr, 240);
                        if let Some(t) = pb.finish() {
                            fill(&mut pm, &t, 240);
                        }
                    }
                }
                3 => {
                    // Rounded square rotated 45° (diamond-ish).
                    let d = r * 0.95;
                    let mut pb = tiny_skia::PathBuilder::new();
                    pb.move_to(cx, cy - d);
                    pb.line_to(cx + d, cy);
                    pb.line_to(cx, cy + d);
                    pb.line_to(cx - d, cy);
                    pb.close();
                    if let Some(p) = pb.finish() {
                        fill(&mut pm, &p, 225);
                    }
                }
                _ => {
                    // Donut: big disc minus an inner punch-out via even-odd.
                    let mut pb = tiny_skia::PathBuilder::new();
                    let outer = tiny_skia::PathBuilder::from_circle(cx, cy, r)
                        .expect("outer ring");
                    let inner = tiny_skia::PathBuilder::from_circle(cx, cy, r * 0.45)
                        .expect("inner ring");
                    pb.push_path(&outer);
                    pb.push_path(&inner);
                    if let Some(p) = pb.finish() {
                        pm.fill_path(
                            &p,
                            &paint(230),
                            tiny_skia::FillRule::EvenOdd,
                            Transform::identity(),
                            None,
                        );
                    }
                }
            }
            let path = cache_dir()
                .join("demo")
                .join("stickers")
                .join(short)
                .join(format!("{i}.png"));
            save_png(&pm, &path);
            let doc_id = 900_000_000i64 + (si as i64) * 1000 + i as i64;
            docs.push(DemoSticker {
                doc_id,
                access_hash: doc_id.wrapping_mul(7919),
                alt: alt.to_string(),
                path: path.to_string_lossy().into_owned(),
            });
        }
        out.push(DemoStickerSet {
            title: title.to_string(),
            short_name: short.to_string(),
            docs,
        });
    }
    out
}

/// Canned info-panel detail for a demo chat (`None` = unknown chat).
fn demo_chat_detail(id: i64) -> Option<ChatDetail> {
    let d = match id {
        1001 => ChatDetail {
            id,
            title: "Camille".into(),
            kind: ChatKind::User,
            username: Some("camille_dev".into()),
            bio: Some("Rust & mountains. Weekend hike photos inside 🏔".into()),
            phone: Some("+33 6 12 34 56 78".into()),
            members_count: None,
        },
        1002 => ChatDetail {
            id,
            title: "Rust Group".into(),
            kind: ChatKind::Group,
            username: None,
            bio: Some("Weekly project reviews — Fridays at 18:00.".into()),
            phone: None,
            members_count: Some(5),
        },
        1003 => ChatDetail {
            id,
            title: "Canal Paysages".into(),
            kind: ChatKind::Channel,
            username: Some("paysages".into()),
            bio: Some("Paysages du monde entier, une photo par jour 🌍".into()),
            phone: None,
            members_count: Some(1284),
        },
        1004 => ChatDetail {
            id,
            title: "Groupe Famille".into(),
            kind: ChatKind::Group,
            username: None,
            bio: None,
            phone: None,
            members_count: Some(4),
        },
        1005 => ChatDetail {
            id,
            title: "Paris Bots".into(),
            kind: ChatKind::Bot,
            username: Some("parisbot".into()),
            bio: Some("Release news for Paris bots. /help for commands.".into()),
            phone: None,
            members_count: None,
        },
        _ => return None,
    };
    Some(d)
}

/// Canned member list of a demo group/channel (empty = hidden/none).
fn demo_participants(id: i64) -> Vec<ParticipantRow> {
    let p = |id: i64, name: &str, role| ParticipantRow {
        id,
        name: name.to_string(),
        username: None,
        role,
    };
    let named = |id: i64, name: &str, username: &str, role| ParticipantRow {
        id,
        name: name.to_string(),
        username: Some(username.to_string()),
        role,
    };
    match id {
        1002 => vec![
            named(2001, "Camille", "camille_dev", ParticipantRole::Creator),
            named(2002, "Léo", "leo_builds", ParticipantRole::Admin),
            named(2003, "Thomas", "thomas_rs", ParticipantRole::Member),
            p(2004, "Sofia", ParticipantRole::Member),
            p(2005, "Marc", ParticipantRole::Member),
        ],
        1003 => vec![
            p(2201, "Camille", ParticipantRole::Creator),
            p(2202, "Léo", ParticipantRole::Admin),
        ],
        1004 => vec![
            p(2101, "Maman", ParticipantRole::Creator),
            p(2102, "Papa", ParticipantRole::Member),
            p(2103, "Sophie", ParticipantRole::Member),
            p(2104, "Toi", ParticipantRole::Member),
        ],
        _ => Vec::new(),
    }
}

/// Offline "backend" for `--demo`: replays canned dialogs/messages, echoes
/// edits/deletes, and includes generated avatar/photo images so the full
/// image pipeline is exercised. `big=true` extends the first chat (Camille)
/// to a ~400-message generated history for scroll-performance measurement.
async fn serve_demo(
    ui_tx: &mpsc::UnboundedSender<UiMessage>,
    req_rx: &mut mpsc::UnboundedReceiver<Request>,
    big: bool,
) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i32;

    let chats = vec![
        DemoChat { id: 1001, title: "Camille".into(), subtitle: "Great! see you tomorrow 👋".into(), last_ago: 42, unread: 3, hue: 0.55 },
        DemoChat { id: 1002, title: "Rust Group".into(), subtitle: "Thomas: novel review of the PR?".into(), last_ago: 7200, unread: 0, hue: 0.1 },
        DemoChat { id: 1003, title: "Landscape Channel".into(), subtitle: "Sunset over the sea 🏖".into(), last_ago: 86400, unread: 0, hue: 0.95 },
        DemoChat { id: 1004, title: "Family Group".into(), subtitle: "Mom: when are you coming over?".into(), last_ago: 172800, unread: 12, hue: 0.3 },
        DemoChat { id: 1005, title: "Paris Bots".into(), subtitle: "New version 2.4.0 released".into(), last_ago: 604800, unread: 0, hue: 0.75 },
    ];
    // Shared with the request handlers below so create/leave/delete/rename
    // can mutate the list at runtime (the closures only borrow it).
    let chats = std::rc::Rc::new(std::cell::RefCell::new(chats));

    let assets = ensure_demo_assets(&chats.borrow());
    // Two canned sticker packs (512x512 generated PNGs) + their picker data.
    let demo_stickers = ensure_demo_stickers();
    // Snapshot of the chat list as dialog rows (also used by the
    // create/leave/delete handlers to refresh the UI).
    let build_rows = |chats: &[DemoChat]| -> Vec<ChatRow> {
        chats
            .iter()
            .map(|c| ChatRow {
                id: c.id,
                title: c.title.clone(),
                subtitle: c.subtitle.clone(),
                date: now - c.last_ago,
                unread: c.unread,
                avatar_path: assets.avatars.get(&c.id).cloned(),
            })
            .collect()
    };
    let _ = ui_tx.send(UiMessage::Dialogs(build_rows(&chats.borrow())));
    // Demo starts signed-in: the chat UI is the surface we want to exercise.
    let _ = ui_tx.send(UiMessage::LoginOk {
        name: "Demo".to_string(),
    });

    let photo_of = |chat: &DemoChat| assets.photos.get(&chat.id).cloned();
    let msgs_for = |id: i64| -> Vec<MsgRow> {
        let chats = chats.borrow();
        let chat = chats.iter().find(|c| c.id == id);
        let photo = chat.and_then(&photo_of);
        let doc_row = |id: i32, text: &str, date: i32, name: &str, size: i64| MsgRow {
            doc: Some(DocMeta { name: name.into(), size, kind: DocKind::File, duration: None }),
            ..MsgRow::text(id, text, date, false)
        };
        match id {
            1001 if big => {
                let count: usize = std::env::var("TG_BIG_N")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(420);
                (0..count)
                    .map(|i| {
                    let id = i as i32;
                    let body = if i % 7 == 0 {
                        "A longer message to force line wrapping and test the bubble height calculation over several lines, with a few emojis 🎉 and some text, plus a little more text to measure wrapping.".to_string()
                    } else {
                        format!(
                            "Message {id} of the big test conversation — the quick brown fox jumps over the lazy dog {id}"
                        )
                    };
                    MsgRow {
                        photo: if i % 40 == 7 { Some((640, 480)) } else { None },
                        photo_path: if i % 40 == 7 { photo.clone() } else { None },
                        reply_to: if i % 25 == 5 { Some(id - 1) } else { None },
                        ..MsgRow::text(
                            id,
                            body,
                            now - (i as i32) * 10,
                            i % 2 == 0,
                        )
                    }
                })
                .collect()
            },
            1001 => vec![
                MsgRow::text(1, "Hi! did you see my new photo?", now - 700, false),
                MsgRow {
                    photo: Some((640, 480)),
                    photo_path: photo.clone(),
                    ..MsgRow::text(2, "", now - 650, true)
                },
                MsgRow {
                    reply_to: Some(1),
                    ..MsgRow::text(3, "Stunning! taken this morning?", now - 600, true)
                },
                MsgRow {
                    doc: Some(DocMeta {
                        name: "sunset.mp4".into(),
                        size: 8_423_168,
                        kind: DocKind::Video,
                        duration: Some(47.0),
                    }),
                    ..MsgRow::text(4, "A short clip of the sunset 🌅", now - 575, true)
                },
                MsgRow {
                    forwarded_from: Some("Landscape Channel".into()),
                    photo: Some((640, 480)),
                    photo_path: photo.clone(),
                    ..MsgRow::text(5, "Look at this one 😍", now - 550, false)
                },
                MsgRow {
                    doc: Some(DocMeta {
                        name: "voice-memo.ogg".into(),
                        size: 310_000,
                        kind: DocKind::Audio { voice: true },
                        duration: Some(12.0),
                    }),
                    ..MsgRow::text(6, "", now - 520, false)
                },
                MsgRow {
                    doc: Some(DocMeta { name: "quarterly-plan.xlsx".into(), size: 96_470, kind: DocKind::File, duration: None }),
                    ..MsgRow::text(7, "", now - 500, true)
                },
                MsgRow::text(8, "Awesome, going there Saturday? 😎", now - 300, false),
                MsgRow::text(
                    9,
                    "Hey, the docs are here: https://doc.rust-lang.org/book/ 😉",
                    now - 120,
                    false,
                ),
                MsgRow::text(
                    10,
                    "And the full changelog: https://github.com/rust-lang/rust/blob/master/RELEASES.md#version-1800-2021-10-21",
                    now - 100,
                    true,
                ),
                MsgRow::text(11, "Yes! see you tomorrow 👋", now - 42, true),
            ],
            1002 => vec![
                MsgRow {
                    sender_name: Some("Camille".into()),
                    sender_id: Some(2001),
                    pinned: true,
                    ..MsgRow::text(1, "Who wants to present their project on Friday?", now - 14400, false)
                },
                MsgRow {
                    forwarded_from: Some("Landscape Channel".into()),
                    sender_name: Some("Léo".into()),
                    sender_id: Some(2002),
                    ..MsgRow::text(2, "[forwarded] Photo from last weekend 🌄", now - 10800, false)
                },
                MsgRow {
                    sticker: demo_stickers
                        .first()
                        .and_then(|s| s.docs.first())
                        .map(|d| StickerMeta { alt: d.alt.clone() }),
                    sticker_path: demo_stickers
                        .first()
                        .and_then(|s| s.docs.first())
                        .map(|d| d.path.clone()),
                    sender_name: Some("Léo".into()),
                    sender_id: Some(2002),
                    ..MsgRow::text(3, "", now - 7500, false)
                },
                MsgRow {
                    sender_name: Some("Camille".into()),
                    sender_id: Some(2001),
                    ..MsgRow::text(4, "@Léo can you review it before Friday?", now - 7300, false)
                },
                MsgRow::text(5, "I can, the CI finally passes 🎉", now - 7200, true),
                doc_row(
                    6,
                    "",
                    now - 7000,
                    "quarterly-report.pdf",
                    2_458_112,
                ),
                MsgRow {
                    sticker: demo_stickers
                        .get(1)
                        .and_then(|s| s.docs.get(4))
                        .map(|d| StickerMeta { alt: d.alt.clone() }),
                    sticker_path: demo_stickers
                        .get(1)
                        .and_then(|s| s.docs.get(4))
                        .map(|d| d.path.clone()),
                    ..MsgRow::text(7, "", now - 6500, true)
                },
            ],
            1003 => vec![
                MsgRow {
                    photo: Some((640, 480)),
                    photo_path: photo,
                    ..MsgRow::text(1, "Photo from last weekend 🌄", now - 90000, true)
                },
                MsgRow::text(2, "Beautiful, we'll make it the cover!", now - 89000, false),
            ],
            1004 => vec![
                MsgRow::text(1, "Sunday's lunch is moved", now - 172800, false),
                MsgRow::text(2, "Ok, we'll bring dessert 🍰", now - 160000, true),
            ],
            1005 => vec![
                MsgRow::text(1, "v2.4.0: new online status API", now - 604800, false),
                MsgRow::text(2, "Thanks for the update!", now - 600000, true),
            ],
            _ => vec![],
        }
    };

    let mut edits: HashMap<i32, String> = HashMap::new();
    // Next message id handed to optimistic local sends (or the echo below).
    let mut next_id = 10000i32;
    // Id of the next chat created through CreateChannel (1006+), and the
    // currently-open demo chat (renames refresh its conversation header).
    let mut next_chat_id = 1006i64;
    let mut demo_open: Option<i64> = None;
    // Simulate the peer replying in Camille's chat every ~12 s: a typing
    // burst of ~3 s followed by the message.
    let mut incoming_idx = 0usize;
    let incoming = [
        "Hey there!",
        "Did you see the news this morning?",
        "On my way, picking you up in 20 min.",
        "Don't forget to grab the package on the way 😉",
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
                    demo_open = Some(id);
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
                        .borrow()
                        .iter()
                        .find(|c| c.id == id)
                        .map(|c| c.title.clone())
                        .unwrap_or_else(|| "Chat".to_string());
                    let _ = ui_tx.send(UiMessage::Messages { id, title, rows });
                    // Demo pinned message: the group pins its topic message.
                    let _ = ui_tx.send(UiMessage::PinnedMessage {
                        chat_id: id,
                        msg_id: (id == 1002).then_some(1),
                    });
                }
                Request::SendMessage { id, text, reply_to } => {
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
                        doc: None,
                        sticker: None,
                        reply_to,
                        forwarded_from: None,
                        sender_name: None,
                        sender_id: None,
                    });
                }
                Request::SendMedia { id, path, caption, is_photo, reply_to, token } => {
                    let nid = next_id;
                    next_id += 1;
                    // Simulate an upload: progress ticks over ~1.5 s, then
                    // the echo carries the media (the demo photo stands in
                    // as the "uploaded" image).
                    let ui_tx = ui_tx.clone();
                    let photo = chats
                        .borrow()
                        .iter()
                        .find(|c| c.id == id)
                        .and_then(|c| assets.photos.get(&c.id).cloned());
                    tokio::spawn(async move {
                        const STEPS: u32 = 10;
                        for step in 1..=STEPS {
                            let _ = ui_tx.send(UiMessage::UploadProgress {
                                chat_id: id,
                                token,
                                progress: step as f32 / STEPS as f32,
                            });
                            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                        }
                        let _ = ui_tx.send(UiMessage::UploadDone { chat_id: id, token });
                        let name = std::path::Path::new(&path)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "file".into());
                        let size = std::fs::metadata(&path)
                            .map(|m| m.len() as i64)
                            .unwrap_or(0);
                        let _ = ui_tx.send(UiMessage::NewMessage {
                            chat_id: id,
                            id: nid,
                            text: caption,
                            date: now,
                            out: true,
                            photo: is_photo.then_some((640, 480)),
                            doc: (!is_photo).then_some(DocMeta {
                                name,
                                size,
                                kind: DocKind::File,
                                duration: None,
                            }),
                            sticker: None,
                            reply_to,
                            forwarded_from: None,
                            sender_name: None,
                            sender_id: None,
                        });
                        if !is_photo {
                            // The document is already on disk (we just picked
                            // it); expose it directly as downloaded.
                            let _ = ui_tx.send(UiMessage::DocReady {
                                chat_id: id,
                                msg_id: nid,
                                path: Some(path),
                            });
                        } else if let Some(p) = photo {
                            let _ = ui_tx.send(UiMessage::PhotoReady {
                                chat_id: id,
                                msg_id: nid,
                                path: Some(p),
                            });
                        }
                    });
                }
                Request::ForwardMessage { from_chat, msg_id, to_chat } => {
                    // Find the original row and echo a forwarded copy into
                    // the destination chat.
                    let origin = msgs_for(from_chat)
                        .into_iter()
                        .find(|m| m.id == msg_id);
                    let Some(origin) = origin else { continue };
                    let from_title = chats
                        .borrow()
                        .iter()
                        .find(|c| c.id == from_chat)
                        .map(|c| c.title.clone())
                        .unwrap_or_default();
                    let nid = next_id;
                    next_id += 1;
                    let _ = ui_tx.send(UiMessage::NewMessage {
                        chat_id: to_chat,
                        id: nid,
                        text: origin.text,
                        date: now,
                        out: true,
                        photo: origin.photo,
                        doc: origin.doc,
                        sticker: None,
                        reply_to: None,
                        forwarded_from: Some(from_title),
                        sender_name: None,
                        sender_id: None,
                    });
                    if let Some(p) = origin.photo_path {
                        let _ = ui_tx.send(UiMessage::PhotoReady {
                            chat_id: to_chat,
                            msg_id: nid,
                            path: Some(p),
                        });
                    }
                    if let Some(p) = origin.doc_path {
                        let _ = ui_tx.send(UiMessage::DocReady {
                            chat_id: to_chat,
                            msg_id: nid,
                            path: Some(p),
                        });
                    }
                }
Request::DownloadDoc { chat_id, msg_id } => {
                    // Demo docs resolve to their canned stand-in file: a real
                    // generated WAV for voice notes (so playback can be exercised),
                    // a text file otherwise.
                    let path = cache_dir().join("demo");
                    let is_voice = msgs_for(chat_id)
                        .iter()
                        .find(|m| m.id == msg_id)
                        .is_some_and(|m| {
                            matches!(
                                m.doc.as_ref().map(|d| d.kind),
                                Some(DocKind::Audio { voice: true })
                            )
                        });
                    let file = if is_voice {
                        demo_voice_wav(&path)
                    } else {
                        path.join("doc.txt").to_string_lossy().into_owned()
                    };
                    let _ = ui_tx.send(UiMessage::DocReady {
                        chat_id,
                        msg_id,
                        path: Some(file),
                    });
                }
                Request::Search { id, query } => {
                    let q = query.to_lowercase();
                    let hits: Vec<SearchHit> = if let Some(chat_id) = id {
                        // In-chat: filter the chat's canned history.
                        let chat_title = chats
                            .borrow()
                            .iter()
                            .find(|c| c.id == chat_id)
                            .map(|c| c.title.clone())
                            .unwrap_or_else(|| "Chat".to_string());
                        msgs_for(chat_id)
                            .into_iter()
                            .filter(|m| !q.is_empty() && m.text.to_lowercase().contains(&q))
                            .map(|row| SearchHit { chat_id, chat_title: chat_title.clone(), row })
                            .take(SEARCH_LIMIT)
                            .collect()
                    } else {
                        // Global: match dialog titles + message bodies.
                        let mut hits = Vec::new();
                        for c in chats.borrow().iter() {
                            let mut found = false;
                            if !q.is_empty() && c.title.to_lowercase().contains(&q) {
                                found = true;
                            }
                            let mut chat_hits: Vec<MsgRow> = msgs_for(c.id)
                                .into_iter()
                                .filter(|m| {
                                    found || (!q.is_empty() && m.text.to_lowercase().contains(&q))
                                })
                                .collect();
                            if !chat_hits.is_empty() {
                                hits.push(SearchHit {
                                    chat_id: c.id,
                                    chat_title: c.title.to_string(),
                                    row: chat_hits.remove(0),
                                });
                            }
                        }
                        hits.truncate(SEARCH_LIMIT);
                        hits
                    };
                    let _ = ui_tx.send(UiMessage::SearchResults {
                        id,
                        query,
                        hits,
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
                Request::PinMessage { id, msg_id, pin } => {
                    let _ = ui_tx.send(UiMessage::PinnedMessage {
                        chat_id: id,
                        msg_id: pin.then_some(msg_id),
                    });
                }
                Request::CreateChannel { title, about: _, megagroup: _, members: _ } => {
                    let id = next_chat_id;
                    next_chat_id += 1;
                    // Deterministic hue from the id so the letter-circle
                    // avatar (no file is generated for runtime chats) gets a
                    // stable palette color.
                    let hue = ((id as f32 * 0.618_034) % 1.0 + 1.0) % 1.0;
                    chats.borrow_mut().push(DemoChat {
                        id,
                        title,
                        subtitle: String::new(),
                        last_ago: 0,
                        unread: 0,
                        hue,
                    });
                    let _ = ui_tx.send(UiMessage::Dialogs(build_rows(&chats.borrow())));
                    let _ = ui_tx.send(UiMessage::ChatCreated { id });
                }
                Request::LeaveChat { id } | Request::DeleteChat { id } => {
                    chats.borrow_mut().retain(|c| c.id != id);
                    let _ = ui_tx.send(UiMessage::Dialogs(build_rows(&chats.borrow())));
                    let _ = ui_tx.send(UiMessage::ChatGone { id });
                }
                Request::EditChatTitle { id, title } => {
                    if let Some(c) = chats
                        .borrow_mut()
                        .iter_mut()
                        .find(|c| c.id == id)
                    {
                        c.title = title.clone();
                    }
                    let _ = ui_tx.send(UiMessage::Dialogs(build_rows(&chats.borrow())));
                    if demo_open == Some(id) {
                        let rows = msgs_for(id);
                        let _ = ui_tx.send(UiMessage::Messages { id, title, rows });
                    }
                }
                Request::MarkRead { id } => {
                    let _ = ui_tx.send(UiMessage::ChatRead { id });
                }
                Request::GetChatInfo { id } => {
                    if let Some(detail) = demo_chat_detail(id) {
                        let _ = ui_tx.send(UiMessage::ChatInfo(detail));
                    }
                }
                Request::GetParticipants { id } => {
                    let _ = ui_tx.send(UiMessage::Participants(demo_participants(id)));
                }
                Request::SetMuted { .. } => {}
                Request::KickParticipant { user_id, .. } => {
                    // The server would confirm through an update; echo it.
                    let _ = ui_tx.send(UiMessage::ParticipantKicked { user_id });
                }
                Request::Typing { .. } => {}
                Request::GetStickerSets => {
                    // The packs + thumbnails are pre-generated: everything
                    // answers immediately from disk.
                    let bridge_sets: Vec<StickerSetBridge> = demo_stickers
                        .iter()
                        .map(|s| StickerSetBridge {
                            title: s.title.clone(),
                            short_name: s.short_name.clone(),
                            docs: s
                                .docs
                                .iter()
                                .map(|d| (d.doc_id, d.access_hash, d.alt.clone()))
                                .collect(),
                        })
                        .collect();
                    let _ = ui_tx.send(UiMessage::StickerSets(bridge_sets));
                    for set in &demo_stickers {
                        for d in &set.docs {
                            let _ = ui_tx.send(UiMessage::StickerThumbReady {
                                doc_id: d.doc_id,
                                path: Some(d.path.clone()),
                            });
                        }
                    }
                }
                Request::SendSticker { id, doc_id, access_hash: _ } => {
                    let doc = demo_stickers
                        .iter()
                        .flat_map(|s| &s.docs)
                        .find(|d| d.doc_id == doc_id);
                    let Some(doc) = doc else { continue };
                    let nid = next_id;
                    next_id += 1;
                    let _ = ui_tx.send(UiMessage::NewMessage {
                        chat_id: id,
                        id: nid,
                        text: String::new(),
                        date: now,
                        out: true,
                        photo: None,
                        doc: None,
                        sticker: Some(StickerMeta { alt: doc.alt.clone() }),
                        reply_to: None,
                        forwarded_from: None,
                        sender_name: None,
                        sender_id: None,
                    });
                    let _ = ui_tx.send(UiMessage::StickerPathReady {
                        chat_id: id,
                        msg_id: nid,
                        path: Some(doc.path.clone()),
                    });
                }
                Request::DownloadSticker { chat_id, msg_id } => {
                    let path = msgs_for(chat_id)
                        .into_iter()
                        .find(|m| m.id == msg_id)
                        .and_then(|m| m.sticker_path);
                    if let Some(p) = path {
                        let _ = ui_tx.send(UiMessage::StickerPathReady {
                            chat_id,
                            msg_id,
                            path: Some(p),
                        });
                    }
                }
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
            let date = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i32;
            let _ = ui_tx.send(UiMessage::NewMessage {
                chat_id: 1001,
                id: next_id,
                text,
                date,
                out: false,
                photo: None,
                doc: None,
                sticker: None,
                reply_to: None,
                forwarded_from: None,
                sender_name: None,
                sender_id: None,
            });
            next_id += 1;
            next_incoming = now_inst + std::time::Duration::from_secs(45);
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

/// Shared download state: on-disk photo/document paths (keyed by chat/message),
/// sticker document references (for re-sending) and a concurrency cap for
/// background MTProto transfers.
struct Downloads {
    photos: Mutex<HashMap<(i64, i32), String>>,
    docs: Mutex<HashMap<(i64, i32), String>>,
    stickers: Mutex<HashMap<(i64, i32), String>>,
    /// Document references seen in the picker listing:
    /// `doc_id -> (access_hash, file_reference)`, needed by `SendSticker`.
    sticker_refs: Mutex<HashMap<i64, (i64, Vec<u8>)>>,
    /// On-disk picker thumbnails: `doc_id -> path`.
    thumbs: Mutex<HashMap<i64, String>>,
    sem: Arc<Semaphore>,
}

impl Downloads {
    fn new() -> Self {
        Self {
            photos: Mutex::new(HashMap::new()),
            docs: Mutex::new(HashMap::new()),
            stickers: Mutex::new(HashMap::new()),
            sticker_refs: Mutex::new(HashMap::new()),
            thumbs: Mutex::new(HashMap::new()),
            sem: Arc::new(Semaphore::new(DOWNLOAD_CONCURRENCY)),
        }
    }

    fn path(&self, chat_id: i64, msg_id: i32) -> Option<String> {
        self.photos.lock().unwrap().get(&(chat_id, msg_id)).cloned()
    }

    fn insert(&self, chat_id: i64, msg_id: i32, path: String) {
        self.photos.lock().unwrap().insert((chat_id, msg_id), path);
    }

    fn doc_path(&self, chat_id: i64, msg_id: i32) -> Option<String> {
        self.docs.lock().unwrap().get(&(chat_id, msg_id)).cloned()
    }

    fn insert_doc(&self, chat_id: i64, msg_id: i32, path: String) {
        self.docs.lock().unwrap().insert((chat_id, msg_id), path);
    }

    fn sticker_path(&self, chat_id: i64, msg_id: i32) -> Option<String> {
        self.stickers
            .lock()
            .unwrap()
            .get(&(chat_id, msg_id))
            .cloned()
    }

    fn insert_sticker(&self, chat_id: i64, msg_id: i32, path: String) {
        self.stickers
            .lock()
            .unwrap()
            .insert((chat_id, msg_id), path);
    }
}

/// Sort used by the network loop so `OpenChat` (history loading) is always
/// handled before slower background work (avatar downloads) in the same batch.
fn prioritize(pending: &mut [Request]) {
    pending.sort_by_key(|r| !matches!(r, Request::OpenChat { .. }));
}

/// Splits a core `MediaKind` into the bridge's photo / document / sticker
/// triple.
fn media_to_row(
    media: Option<tg::model::MediaKind>,
) -> (
    Option<(u32, u32)>,
    Option<DocMeta>,
    Option<StickerMeta>,
) {
    use crate::bridge::DocKind;
    use tg::model::MediaKind as MK;
    match media {
        Some(MK::Photo { width, height }) => (Some((width, height)), None, None),
        Some(MK::Document { name, size }) => (
            None,
            Some(DocMeta {
                name,
                size,
                kind: DocKind::File,
                duration: None,
            }),
            None,
        ),
        Some(MK::Video { name, size, duration }) => (
            None,
            Some(DocMeta {
                name,
                size,
                kind: DocKind::Video,
                duration: Some(duration),
            }),
            None,
        ),
        Some(MK::Gif { name, size }) => (
            None,
            Some(DocMeta {
                name,
                size,
                kind: DocKind::Gif,
                duration: None,
            }),
            None,
        ),
        Some(MK::Audio { name, size, voice }) => (
            None,
            Some(DocMeta {
                name,
                size,
                kind: DocKind::Audio { voice },
                duration: None,
            }),
            None,
        ),
        // Stickers are NOT a document card: frameless image rendering.
        Some(MK::Sticker { alt, .. }) => (None, None, Some(StickerMeta { alt })),
        None => (None, None, None),
    }
}

/// Maps a core `MessageInfo` to a display row: splits the media kind into
/// photo vs document, resolves forward origins against the dialog list and
/// reuses any already-cached photo path.
fn msg_row_from_info(
    m: tg::model::MessageInfo,
    chat_id: i64,
    downloads: &Downloads,
    peers: &HashMap<i64, (String, PeerRef)>,
) -> MsgRow {
    let (photo, doc, sticker) = media_to_row(m.media);
    let forwarded_from = m.forwarded.and_then(|f| {
        f.name.or_else(|| {
            f.chat_id
                .and_then(|id| peers.get(&id))
                .map(|(title, _)| title.clone())
        })
    });
    let sticker_path = if sticker.is_some() {
        downloads.sticker_path(chat_id, m.id)
    } else {
        None
    };
    MsgRow {
        id: m.id,
        text: m.text,
        date: m.date,
        out: m.out,
        photo_path: photo.as_ref().and_then(|_| downloads.path(chat_id, m.id)),
        photo,
        doc_path: doc.as_ref().and_then(|_| downloads.doc_path(chat_id, m.id)),
        doc,
        sticker,
        sticker_path,
        reply_to: m.reply_to,
        forwarded_from,
        uploading: None,
        upload_token: None,
        read: false,
        sender_name: m.sender_name,
        sender_id: m.sender_id,
        pinned: m.pinned,
    }
}

/// Reloads the dialog list from the server, refreshes the id→peer map and
/// pushes the updated list to the UI. Used at boot and after any chat-list
/// mutation (create / leave / delete / rename), so both the UI rows and the
/// peer cache stay consistent.
async fn refresh_dialogs(
    tg: &Telegram,
    ui_tx: &mpsc::UnboundedSender<UiMessage>,
    peers: &mut HashMap<i64, (String, PeerRef)>,
) -> anyhow::Result<()> {
    let dialogs = tg.get_dialogs().await?;
    peers.clear();
    let rows: Vec<ChatRow> = dialogs
        .iter()
        .map(|d| {
            peers.insert(d.id.bot_api_dialog_id(), (d.title.clone(), d.peer_ref));
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
    Ok(())
}

/// Network loop: handles UI requests and real-time updates.
///
/// Interactive work (OpenChat, sends, edits…) is done inline with a priority
/// sort, so it is never queued behind slow downloads. Avatar and photo
/// thumbnails run in the background through [`Downloads`]' semaphore: awaiting
/// them one-by-one in this loop made a click on a chat take up to a minute
/// (every avatar of the dialog list downloaded first).
async fn serve(
    tg: Arc<Telegram>,
    updates_rx: tokio::sync::mpsc::UnboundedReceiver<UpdatesLike>,
    ui_tx: &mpsc::UnboundedSender<UiMessage>,
    req_rx: &mut mpsc::UnboundedReceiver<Request>,
    peers: &mut HashMap<i64, (String, PeerRef)>,
) {
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

    let downloads = Arc::new(Downloads::new());

    let refresh = std::time::Duration::from_secs(15);
    let mut last_refresh = std::time::Instant::now();
    let mut open_id: Option<i64> = None;
    let mut open_sig: Option<(usize, String)> = None;

    let save_every = std::time::Duration::from_secs(30);
    let mut last_save = std::time::Instant::now();
    let session = tg.session().clone();

    // Latest search request + when it was last actually run (throttle).
    let mut last_search: Option<(Option<i64>, String, std::time::Instant)> = None;

    let started = std::time::Instant::now();
    let grace = std::time::Duration::from_secs(10);

    loop {
        let pending_in: Vec<Request> =
            std::iter::from_fn(|| req_rx.try_recv().ok()).collect();

        // Coalesce Search requests (the UI sends one per keystroke): keep the
        // latest query per mode, and split them from the interactive work.
        let mut searches: Vec<(Option<i64>, String)> = Vec::new();
        let count = pending_in.len();
        let mut pending: Vec<Request> = Vec::with_capacity(count);
        for r in pending_in {
            match r {
                Request::Search { id, query } => {
                    searches.retain(|(m, _)| *m != id);
                    searches.push((id, query.trim().to_string()));
                }
                other => pending.push(other),
            }
        }
        prioritize(&mut pending);
        for req in pending {
            if let Request::OpenChat { id } = req {
                open_id = Some(id);
                open_sig = None;
            }
            handle_request(&tg, ui_tx, req, peers, &downloads).await;
        }
        // Run throttled searches: re-running the same query within the window
        // is skipped (typing floods → one MTProto round-trip per pause).
        for (mode, query) in &searches {
            let throttled = last_search
                .as_ref()
                .is_some_and(|(m, q, t)| m == mode && q == query && t.elapsed() < SEARCH_THROTTLE);
            if throttled {
                continue;
            }
            last_search = Some((*mode, query.clone(), std::time::Instant::now()));
            handle_request(
                &tg,
                ui_tx,
                Request::Search { id: *mode, query: query.clone() },
                peers,
                &downloads,
            )
            .await;
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
                            let rows: Vec<MsgRow> = msgs
                                .iter()
                                .map(|m| {
                                    msg_row_from_info(m.clone(), open, &downloads, peers)
                                })
                                .collect();
                            let _ = ui_tx.send(UiMessage::Messages {
                                id: open,
                                title: title.clone(),
                                rows,
                            });
                            // Warm photo thumbnails for any new photo messages.
                            for m in &msgs {
                                if matches!(
                                    m.media,
                                    Some(tg::model::MediaKind::Photo { .. })
                                ) {
                                    spawn_photo(
                                        tg.clone(),
                                        ui_tx.clone(),
                                        downloads.clone(),
                                        open,
                                        *peer_ref,
                                        m.id,
                                    );
                                }
                                if matches!(
                                    m.media,
                                    Some(tg::model::MediaKind::Sticker { .. })
                                ) {
                                    spawn_sticker_download(
                                        tg.clone(),
                                        ui_tx.clone(),
                                        downloads.clone(),
                                        open,
                                        *peer_ref,
                                        m.id,
                                    );
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
                // A message for a chat that is *not* open rings a desktop
                // notification.
                if let Update::NewMessage(ref msg) = update {
                    if let Some(peer) = msg.peer() {
                        let cid = peer.id().bot_api_dialog_id();
                        if open_id != Some(cid) {
                            let title = peers
                                .get(&cid)
                                .map(|(t, _)| t.clone())
                                .unwrap_or_else(|| "Message".to_string());
                            notify_new_message(
                                open_id,
                                cid,
                                &title,
                                &crate::state::preview_text(
                                    msg.text(),
                                    &None,
                                    &None,
                                    &None,
                                ),
                            );
                        }
                    }
                }
                handle_update(ui_tx, update, peers);
            }
        }
    }
}

fn handle_update(
    ui_tx: &mpsc::UnboundedSender<UiMessage>,
    update: Update,
    peers: &HashMap<i64, (String, PeerRef)>,
) {
    match update {
        Update::NewMessage(msg) => {
            if let Some(peer) = msg.peer() {
                let chat_id = peer.id().bot_api_dialog_id();
                let (photo, doc, sticker) =
                    media_to_row(tg::client::media_kind(msg.media().as_ref()));
                let forwarded_from = msg.forward_header().as_ref().and_then(|h| {
                    // Same resolution as history rows: the header's plain name
                    // wins, else look up the origin chat in the dialog list.
                    forward_name(h, peers)
                });
                // Sender name only matters in group-ish chats (same rule as
                // the history mapper in `tg`).
                let is_private = matches!(
                    msg.peer_id().kind(),
                    grammers_session::types::PeerKind::User | grammers_session::types::PeerKind::UserSelf
                );
                let sender_name = if is_private {
                    None
                } else {
                    msg.sender().and_then(|p| p.name()).map(str::to_string)
                };
                let sender_id = if is_private {
                    None
                } else {
                    msg.sender_id().map(|id| id.bot_api_dialog_id())
                };
                let _ = ui_tx.send(UiMessage::NewMessage {
                    chat_id,
                    id: msg.id(),
                    text: msg.text().to_string(),
                    date: msg.date().timestamp() as i32,
                    out: msg.outgoing(),
                    photo,
                    doc,
                    sticker,
                    reply_to: msg.reply_to_message_id(),
                    forwarded_from,
                    sender_name,
                    sender_id,
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
            grammers_client::tl::enums::Update::PinnedMessages(u) => {
                let _ = ui_tx.send(UiMessage::PinnedMessage {
                    chat_id: grammers_session::types::PeerId::from(u.peer.clone())
                        .bot_api_dialog_id(),
                    msg_id: u.messages.last().copied(),
                });
            }
            grammers_client::tl::enums::Update::PinnedChannelMessages(u) => {
                let _ = ui_tx.send(UiMessage::PinnedMessage {
                    chat_id: grammers_session::types::PeerId::channel(u.channel_id)
                        .bot_api_dialog_id(),
                    msg_id: u.messages.last().copied(),
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

/// Resolves a raw forward header into a display name: the header's anonymous
/// `from_name` when present, else the origin chat's title from the dialog
/// list (covers the common "forwarded from a chat I know" case).
fn forward_name(
    header: &grammers_client::tl::enums::MessageFwdHeader,
    peers: &HashMap<i64, (String, PeerRef)>,
) -> Option<String> {
    let grammers_client::tl::enums::MessageFwdHeader::Header(h) = header;
    if let Some(name) = &h.from_name {
        return Some(name.clone());
    }
    let chat_id = h
        .from_id
        .as_ref()
        .map(|peer| grammers_session::types::PeerId::from(peer.clone()).bot_api_dialog_id())?;
    peers.get(&chat_id).map(|(title, _)| title.clone())
}

/// Maps a core `ChatDetail` onto its bridge duplicate.
fn chat_detail_to_bridge(d: tg::model::ChatDetail) -> ChatDetail {
    let kind = match d.kind {
        tg::model::ChatKind::User => ChatKind::User,
        tg::model::ChatKind::Group => ChatKind::Group,
        tg::model::ChatKind::Channel => ChatKind::Channel,
        tg::model::ChatKind::Bot => ChatKind::Bot,
    };
    ChatDetail {
        id: d.id,
        title: d.title,
        kind,
        username: d.username,
        bio: d.bio,
        phone: d.phone,
        members_count: d.members_count,
    }
}

/// Maps a core `ParticipantRow` onto its bridge duplicate.
fn participant_to_bridge(p: tg::model::ParticipantRow) -> ParticipantRow {
    let role = match p.role {
        tg::model::ParticipantRole::Creator => ParticipantRole::Creator,
        tg::model::ParticipantRole::Admin => ParticipantRole::Admin,
        tg::model::ParticipantRole::Member => ParticipantRole::Member,
    };
    ParticipantRow {
        id: p.id,
        name: p.name,
        username: p.username,
        role,
    }
}

async fn handle_request(
    tg: &Arc<Telegram>,
    ui_tx: &mpsc::UnboundedSender<UiMessage>,
    req: Request,
    peers: &mut HashMap<i64, (String, PeerRef)>,
    downloads: &Arc<Downloads>,
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
                    let msgs: Vec<MsgRow> = messages
                        .into_iter()
                        .map(|m| msg_row_from_info(m, id, downloads, peers))
                        .collect();
                    let _ = ui_tx.send(UiMessage::Messages {
                        id,
                        title: title.clone(),
                        rows: msgs.clone(),
                    });
                    // Fetch photo thumbnails asynchronously: the conversation
                    // is shown immediately, thumbnails stream in as they come.
                    for m in &msgs {
                        if m.photo.is_some() {
                            spawn_photo(
                                tg.clone(),
                                ui_tx.clone(),
                                downloads.clone(),
                                id,
                                *peer_ref,
                                m.id,
                            );
                        }
                        if m.sticker.is_some() {
                            spawn_sticker_download(
                                tg.clone(),
                                ui_tx.clone(),
                                downloads.clone(),
                                id,
                                *peer_ref,
                                m.id,
                            );
                        }
                    }
                    // The chat's pinned message, if any (banner + jump target).
                    match tg.get_pinned_message(peer_ref).await {
                        Ok(Some(pinned)) => {
                            let _ = ui_tx.send(UiMessage::PinnedMessage {
                                chat_id: id,
                                msg_id: Some(pinned.id),
                            });
                        }
                        Ok(None) => {
                            let _ = ui_tx.send(UiMessage::PinnedMessage { chat_id: id, msg_id: None });
                        }
                        Err(_) => {}
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
        Request::DownloadAvatar { chat_id } => if let Some((_, peer_ref)) = peers.get(&chat_id) {
            spawn_avatar(
                tg.clone(),
                ui_tx.clone(),
                downloads.clone(),
                chat_id,
                *peer_ref,
            );
        },
        Request::SendMessage { id, text, reply_to } => match peers.get(&id) {
            Some((_, peer_ref)) => {
                if let Err(e) = tg.send_message(peer_ref, &text, reply_to).await {
                    let _ = ui_tx.send(UiMessage::Error(format!("Send failed: {e}")));
                }
            }
            None => {
                let _ = ui_tx.send(UiMessage::Error("Unknown chat".to_string()));
            }
        },
        Request::SendMedia { id, path, caption, is_photo: _, reply_to, token } => {
            match peers.get(&id) {
                Some((_, peer_ref)) => spawn_upload(
                    tg.clone(),
                    ui_tx.clone(),
                    downloads.clone(),
                    id,
                    *peer_ref,
                    std::path::PathBuf::from(path),
                    caption,
                    reply_to,
                    token,
                ),
                None => {
                    let _ = ui_tx.send(UiMessage::Error("Unknown chat".to_string()));
                }
            }
        }
        Request::ForwardMessage { from_chat, msg_id, to_chat } => {
            match (peers.get(&from_chat), peers.get(&to_chat)) {
                (Some((_, from_peer)), Some((_, to_peer))) => {
                    if let Err(e) = tg.forward_message(from_peer, msg_id, to_peer).await {
                        let _ = ui_tx.send(UiMessage::Error(format!("Forward failed: {e}")));
                    }
                }
                _ => {
                    let _ = ui_tx.send(UiMessage::Error("Unknown chat".to_string()));
                }
            }
        }
        Request::DownloadDoc { chat_id, msg_id } => {
            if let Some((_, peer_ref)) = peers.get(&chat_id) {
                spawn_doc_download(
                    tg.clone(),
                    ui_tx.clone(),
                    downloads.clone(),
                    chat_id,
                    *peer_ref,
                    msg_id,
                );
            }
        }
        Request::DownloadSticker { chat_id, msg_id } => {
            if let Some((_, peer_ref)) = peers.get(&chat_id) {
                spawn_sticker_download(
                    tg.clone(),
                    ui_tx.clone(),
                    downloads.clone(),
                    chat_id,
                    *peer_ref,
                    msg_id,
                );
            }
        }
        Request::SendSticker { id, doc_id, access_hash } => match peers.get(&id) {
            Some((_, peer_ref)) => {
                let file_reference = downloads
                    .sticker_refs
                    .lock()
                    .unwrap()
                    .get(&doc_id)
                    .map(|(_, r)| r.clone())
                    .unwrap_or_default();
                if let Err(e) = tg
                    .send_sticker(peer_ref, doc_id, access_hash, file_reference, None)
                    .await
                {
                    let _ = ui_tx.send(UiMessage::Error(format!("Send failed: {e}")));
                }
            }
            None => {
                let _ = ui_tx.send(UiMessage::Error("Unknown chat".to_string()));
            }
        },
        Request::GetStickerSets => match tg.sticker_sets().await {
            Ok(sets) => {
                // Remember the document references so picker clicks can
                // re-send the exact sticker.
                {
                    let mut refs = downloads.sticker_refs.lock().unwrap();
                    refs.clear();
                    for set in &sets {
                        for d in &set.docs {
                            refs.insert(d.id, (d.access_hash, d.file_reference.clone()));
                        }
                    }
                }
                let bridge_sets: Vec<StickerSetBridge> = sets
                    .iter()
                    .map(|s| StickerSetBridge {
                        title: s.title.clone(),
                        short_name: s.short_name.clone(),
                        docs: s
                            .docs
                            .iter()
                            .map(|d| (d.id, d.access_hash, d.alt.clone()))
                            .collect(),
                    })
                    .collect();
                // Warm every thumbnail in the background; cells show a
                // placeholder until `StickerThumbReady` streams in.
                for s in &sets {
                    for d in &s.docs {
                        spawn_thumb_download(tg.clone(), ui_tx.clone(), downloads.clone(), d.id);
                    }
                }
                let _ = ui_tx.send(UiMessage::StickerSets(bridge_sets));
            }
            Err(e) => {
                let _ = ui_tx.send(UiMessage::Error(format!(
                    "Could not load sticker packs: {e}"
                )));
            }
        },
        Request::Search { id, query } => {
            if query.is_empty() {
                // Nothing typed: clear the results instead of listing "all".
                let _ = ui_tx.send(UiMessage::SearchResults {
                    id,
                    query: String::new(),
                    hits: Vec::new(),
                });
                return;
            }
            let from = if let Some(chat_id) = id {
                peers.get(&chat_id).map(|(title, peer_ref)| (chat_id, title.clone(), Some(*peer_ref)))
            } else {
                None
            };
            let hits = match from {
                // In-chat search.
                Some((chat_id, title, Some(peer_ref))) => {
                    match tg.search_chat(&peer_ref, &query, SEARCH_LIMIT).await {
                        Ok(hits) => hits
                            .into_iter()
                            .map(|m| SearchHit {
                                chat_id,
                                chat_title: title.clone(),
                                row: msg_row_from_info(m, chat_id, downloads, peers),
                            })
                            .collect(),
                        Err(e) => {
                            let _ = ui_tx.send(UiMessage::Error(format!(
                                "Search failed: {e}"
                            )));
                            return;
                        }
                    }
                }
                // Global search.
                _ => match tg.search_global(&query, SEARCH_LIMIT).await {
                    Ok(hits) => hits
                        .into_iter()
                        .map(|g| {
                            let chat_title = peers
                                .get(&g.peer_id)
                                .map(|(t, _)| t.clone())
                                .unwrap_or_else(|| "Chat".to_string());
                            SearchHit {
                                chat_id: g.peer_id,
                                chat_title,
                                row: msg_row_from_info(g.msg, g.peer_id, downloads, peers),
                            }
                        })
                        .collect(),
                    Err(e) => {
                        let _ = ui_tx.send(UiMessage::Error(format!(
                            "Search failed: {e}"
                        )));
                        return;
                    }
                },
            };
            let _ = ui_tx.send(UiMessage::SearchResults { id, query, hits });
        }
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
        Request::PinMessage { id, msg_id, pin } => match peers.get(&id) {
            Some((_, peer_ref)) => {
                let res = if pin {
                    tg.pin_message(peer_ref, msg_id).await
                } else {
                    tg.unpin_message(peer_ref, msg_id).await
                };
                match res {
                    Ok(()) => {
                        let _ = ui_tx.send(UiMessage::PinnedMessage {
                            chat_id: id,
                            msg_id: pin.then_some(msg_id),
                        });
                    }
                    Err(e) => {
                        let _ = ui_tx.send(UiMessage::Error(format!("Pin failed: {e}")));
                    }
                }
            }
            None => {
                let _ = ui_tx.send(UiMessage::Error("Unknown chat".to_string()));
            }
        },
        Request::CreateChannel { title, about, megagroup, members } => {
            // Groups with picked members go through messages.createChat
            // (initial invites); everything else — and any failed invite —
            // creates a channel/supergroup via channels.createChannel.
            let result: anyhow::Result<i64> = if megagroup {
                let refs: Vec<PeerRef> = members
                    .iter()
                    .filter_map(|m| peers.get(m).map(|(_, p)| *p))
                    .collect();
                if refs.is_empty() {
                    tg.create_channel(&title, &about, true).await
                } else {
                    match tg.create_group(&title, &refs).await {
                        Ok(id) => Ok(id),
                        Err(_) => tg.create_channel(&title, &about, true).await,
                    }
                }
            } else {
                tg.create_channel(&title, &about, false).await
            };
            match result {
                Ok(id) => {
                    // The new chat must be resolvable before the UI opens it.
                    let _ = refresh_dialogs(tg, ui_tx, peers).await;
                    let _ = ui_tx.send(UiMessage::ChatCreated { id });
                }
                Err(e) => {
                    let _ = ui_tx.send(UiMessage::Error(format!("Could not create chat: {e}")));
                }
            }
        }
        Request::LeaveChat { id } | Request::DeleteChat { id } => {
            let leave = matches!(req, Request::LeaveChat { .. });
            let result = match peers.get(&id) {
                Some((_, peer_ref)) => {
                    if leave {
                        tg.leave_chat(peer_ref).await
                    } else {
                        tg.delete_chat(peer_ref).await
                    }
                }
                None => Err(anyhow::anyhow!("unknown chat")),
            };
            match result {
                Ok(()) => {
                    peers.remove(&id);
                    // Keep the list in sync (subtitle/date of the gone chat).
                    let _ = refresh_dialogs(tg, ui_tx, peers).await;
                    let _ = ui_tx.send(UiMessage::ChatGone { id });
                }
                Err(e) => {
                    let _ = ui_tx.send(UiMessage::Error(format!(
                        "Could not remove the chat: {e}"
                    )));
                }
            }
        }
        Request::EditChatTitle { id, title } => match peers.get(&id) {
            Some((_, peer_ref)) => match tg.edit_chat_title(peer_ref, &title).await {
                Ok(()) => {
                    // Refreshed list carries the new title.
                    let _ = refresh_dialogs(tg, ui_tx, peers).await;
                }
                Err(e) => {
                    let _ = ui_tx.send(UiMessage::Error(format!("Rename failed: {e}")));
                }
            },
            None => {
                let _ = ui_tx.send(UiMessage::Error("Unknown chat".to_string()));
            }
        },
        Request::GetChatInfo { id } => match peers.get(&id) {
            Some((_, peer_ref)) => match tg.get_chat_detail(peer_ref).await {
                Ok(detail) => {
                    let _ = ui_tx.send(UiMessage::ChatInfo(chat_detail_to_bridge(detail)));
                }
                Err(e) => {
                    let _ = ui_tx.send(UiMessage::Error(format!("Could not load info: {e}")));
                }
            },
            None => {
                let _ = ui_tx.send(UiMessage::Error("Unknown chat".to_string()));
            }
        },
        Request::SetMuted { id, muted } => {
            if let Some((_, peer_ref)) = peers.get(&id) {
                if let Err(e) = tg.set_muted(peer_ref, muted).await {
                    let _ = ui_tx
                        .send(UiMessage::Error(format!("Could not update mute: {e}")));
                }
            }
        }
        Request::GetParticipants { id } => match peers.get(&id) {
            Some((_, peer_ref)) => match tg.get_participants(peer_ref, 200).await {
                Ok(rows) => {
                    let _ = ui_tx.send(UiMessage::Participants(
                        rows.into_iter().map(participant_to_bridge).collect(),
                    ));
                }
                Err(e) => {
                    let _ = ui_tx
                        .send(UiMessage::Error(format!("Could not load members: {e}")));
                }
            },
            None => {
                let _ = ui_tx.send(UiMessage::Error("Unknown chat".to_string()));
            }
        },
        Request::KickParticipant { id, user_id } => match peers.get(&id) {
            Some((_, peer_ref)) => match tg.kick_participant(peer_ref, user_id).await {
                Ok(()) => {
                    let _ = ui_tx.send(UiMessage::ParticipantKicked { user_id });
                }
                Err(e) => {
                    let _ = ui_tx.send(UiMessage::Error(format!("Kick failed: {e}")));
                }
            },
            None => {
                let _ = ui_tx.send(UiMessage::Error("Unknown chat".to_string()));
            }
        },
        Request::LoginPhone { .. } | Request::LoginCode { .. } | Request::LoginPassword { .. } => {}
    }
}

/// Spawns a background avatar download (capped by the semaphore) and reports
/// the result to the UI.
fn spawn_avatar(
    tg: Arc<Telegram>,
    ui_tx: mpsc::UnboundedSender<UiMessage>,
    downloads: Arc<Downloads>,
    chat_id: i64,
    peer_ref: PeerRef,
) {
    tokio::spawn(async move {
        let _permit = downloads
            .sem
            .clone()
            .acquire_owned()
            .await
            .expect("download semaphore");
        let dir = cache_dir().join("avatars");
        let path = match tg.download_avatar(&peer_ref, &dir).await {
            Ok(p) => p.map(|p| p.to_string_lossy().into_owned()),
            Err(_) => None,
        };
        let _ = ui_tx.send(UiMessage::AvatarReady { chat_id, path });
    });
}

/// Spawns a background download of a message's photo thumbnail (cached), and
/// records the on-disk path so periodic refreshes keep showing it.
fn spawn_photo(
    tg: Arc<Telegram>,
    ui_tx: mpsc::UnboundedSender<UiMessage>,
    downloads: Arc<Downloads>,
    chat_id: i64,
    peer_ref: PeerRef,
    msg_id: i32,
) {
    if downloads.photos.lock().unwrap().contains_key(&(chat_id, msg_id)) {
        return;
    }
    tokio::spawn(async move {
        let _permit = downloads
            .sem
            .clone()
            .acquire_owned()
            .await
            .expect("download semaphore");
        let dir = cache_dir().join("media").join(chat_id.to_string());
        let path = match tg.download_photo(&peer_ref, msg_id, &dir).await {
            Ok(Some(p)) => {
                let p = p.to_string_lossy().into_owned();
                downloads.insert(chat_id, msg_id, p.clone());
                Some(p)
            }
            _ => None,
        };
        let _ = ui_tx.send(UiMessage::PhotoReady {
            chat_id,
            msg_id,
            path,
        });
    });
}

/// Spawns a background media upload + send, reporting progress to the UI
/// through the shared semaphore (uploads count as transfers like downloads).
#[allow(clippy::too_many_arguments)]
fn spawn_upload(
    tg: Arc<Telegram>,
    ui_tx: mpsc::UnboundedSender<UiMessage>,
    downloads: Arc<Downloads>,
    chat_id: i64,
    peer_ref: PeerRef,
    path: PathBuf,
    caption: String,
    reply_to: Option<i32>,
    token: u64,
) {
    tokio::spawn(async move {
        let _permit = downloads
            .sem
            .clone()
            .acquire_owned()
            .await
            .expect("download semaphore");
        // Progress callbacks hop through a std channel drained by a tiny
        // forwarder task: the upload callback is sync (`FnMut`) and must not
        // block on the async UI channel.
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel::<f32>();
        let forwarder = tokio::spawn({
            let ui_tx = ui_tx.clone();
            async move {
                while let Some(p) = progress_rx.recv().await {
                    let _ = ui_tx.send(UiMessage::UploadProgress {
                        chat_id,
                        token,
                        progress: p,
                    });
                }
            }
        });
        let result = {
            let progress_tx = progress_tx.clone();
            tg.send_media(&peer_ref, &path, &caption, reply_to, move |sent, total| {
                let p = if total > 0 { sent as f32 / total as f32 } else { 1.0 };
                let _ = progress_tx.send(p.clamp(0.0, 1.0));
            })
            .await
        };
        drop(progress_tx);
        let _ = forwarder.await;
        match result {
            Ok(_) => {
                let _ = ui_tx.send(UiMessage::UploadDone { chat_id, token });
            }
            Err(e) => {
                let _ = ui_tx.send(UiMessage::UploadDone { chat_id, token });
                let _ = ui_tx.send(UiMessage::Error(format!("Send failed: {e}")));
            }
        }
    });
}

/// Spawns a background document download (cached), reporting the on-disk path.
fn spawn_doc_download(
    tg: Arc<Telegram>,
    ui_tx: mpsc::UnboundedSender<UiMessage>,
    downloads: Arc<Downloads>,
    chat_id: i64,
    peer_ref: PeerRef,
    msg_id: i32,
) {
    if downloads.docs.lock().unwrap().contains_key(&(chat_id, msg_id)) {
        return;
    }
    tokio::spawn(async move {
        let _permit = downloads
            .sem
            .clone()
            .acquire_owned()
            .await
            .expect("download semaphore");
        let dir = cache_dir().join("media").join(chat_id.to_string());
        let path = match tg.download_document(&peer_ref, msg_id, &dir).await {
            Ok(Some(p)) => {
                let p = p.to_string_lossy().into_owned();
                downloads.insert_doc(chat_id, msg_id, p.clone());
                Some(p)
            }
            _ => None,
        };
        let _ = ui_tx.send(UiMessage::DocReady { chat_id, msg_id, path });
    });
}

/// Spawns a background sticker download for a message (cached), reporting the
/// on-disk path through [`UiMessage::StickerPathReady`].
fn spawn_sticker_download(
    tg: Arc<Telegram>,
    ui_tx: mpsc::UnboundedSender<UiMessage>,
    downloads: Arc<Downloads>,
    chat_id: i64,
    peer_ref: PeerRef,
    msg_id: i32,
) {
    if downloads
        .stickers
        .lock()
        .unwrap()
        .contains_key(&(chat_id, msg_id))
    {
        return;
    }
    tokio::spawn(async move {
        let _permit = downloads
            .sem
            .clone()
            .acquire_owned()
            .await
            .expect("download semaphore");
        let dir = cache_dir().join("stickers").join(chat_id.to_string());
        let path = match tg.download_sticker(&peer_ref, msg_id, &dir).await {
            Ok(Some(p)) => {
                let p = p.to_string_lossy().into_owned();
                downloads.insert_sticker(chat_id, msg_id, p.clone());
                Some(p)
            }
            _ => None,
        };
        let _ = ui_tx.send(UiMessage::StickerPathReady { chat_id, msg_id, path });
    });
}

/// Spawns a background picker-thumbnail download by document reference
/// (cached), reporting the on-disk path via [`UiMessage::StickerThumbReady`].
fn spawn_thumb_download(
    tg: Arc<Telegram>,
    ui_tx: mpsc::UnboundedSender<UiMessage>,
    downloads: Arc<Downloads>,
    doc_id: i64,
) {
    if downloads.thumbs.lock().unwrap().contains_key(&doc_id) {
        return;
    }
    let (access_hash, file_reference) = match downloads.sticker_refs.lock().unwrap().get(&doc_id) {
        Some((h, r)) => (*h, r.clone()),
        None => return,
    };
    tokio::spawn(async move {
        let _permit = downloads
            .sem
            .clone()
            .acquire_owned()
            .await
            .expect("download semaphore");
        let dir = cache_dir().join("stickers").join("thumbs");
        let path = match tg
            .download_sticker_doc(doc_id, access_hash, file_reference, &dir)
            .await
        {
            Ok(Some(p)) => {
                let p = p.to_string_lossy().into_owned();
                downloads.thumbs.lock().unwrap().insert(doc_id, p.clone());
                Some(p)
            }
            _ => None,
        };
        let _ = ui_tx.send(UiMessage::StickerThumbReady { doc_id, path });
    });
}

/// Sends a desktop notification for a new message, unless it lands in the
/// currently open chat (that's visible already). Best-effort: any
/// notify-rust error is silently ignored.
pub fn notify_new_message(open_chat: Option<i64>, chat_id: i64, title: &str, preview: &str) {
    if open_chat == Some(chat_id) {
        return;
    }
    let _ = notify_rust::Notification::new()
        .summary(title)
        .body(preview)
        .appname("tg")
        .show();
}

/// Re-exported so `main.rs` can type the sender.
pub type UnboundedSender<T> = mpsc::UnboundedSender<T>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_voice_wav_is_a_decodable_wave_file() {
        let dir = std::env::temp_dir().join("tg-wav-test");
        let _ = std::fs::create_dir_all(&dir);
        let p = demo_voice_wav(&dir);
        // The file exists, has a plausible RIFF header, and rodio can decode it.
        let bytes = std::fs::read(&p).expect("wav written");
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        let file = std::fs::File::open(&p).unwrap();
        assert!(
            rodio::Decoder::new(std::io::BufReader::new(file)).is_ok(),
            "rodio must decode the generated demo voice note"
        );
    }

    #[test]
    fn media_to_row_classifies_document_kinds() {
        use crate::bridge::DocKind;
        // Video → DocKind::Video with duration.
        let (photo, doc, _) = media_to_row(Some(tg::model::MediaKind::Video {
            name: "v.mp4".into(),
            size: 10,
            duration: 42.0,
        }));
        assert!(photo.is_none());
        assert_eq!(doc.as_ref().map(|d| d.kind), Some(DocKind::Video));
        assert_eq!(doc.unwrap().duration, Some(42.0));
        // Voice → DocKind::Audio { voice: true }.
        let (_, doc, _) = media_to_row(Some(tg::model::MediaKind::Audio {
            name: "v.ogg".into(),
            size: 20,
            voice: true,
        }));
        assert_eq!(doc.unwrap().kind, DocKind::Audio { voice: true });
        // Plain file stays a file.
        let (_, doc, _) = media_to_row(Some(tg::model::MediaKind::Document {
            name: "f.pdf".into(),
            size: 30,
        }));
        assert_eq!(doc.unwrap().kind, DocKind::File);
    }

    #[test]
    fn media_to_row_classifies_stickers_as_frameless_media() {
        // A sticker is neither a photo nor a document card: it maps to its
        // own frameless slot carrying the emoji.
        let (photo, doc, sticker) = media_to_row(Some(tg::model::MediaKind::Sticker {
            name: String::new(),
            size: 1234,
            alt: "🎉".into(),
        }));
        assert!(photo.is_none());
        assert!(doc.is_none(), "stickers must not render as document cards");
        assert_eq!(sticker.map(|s| s.alt), Some("🎉".to_string()));
    }

    #[test]
    fn ensure_demo_stickers_generates_two_packs_of_pngs() {
        let sets = ensure_demo_stickers();
        assert_eq!(sets.len(), 2);
        for set in &sets {
            assert!(!set.title.is_empty());
            assert_eq!(set.docs.len(), 10);
            for d in &set.docs {
                let bytes = std::fs::read(&d.path).expect("sticker png written");
                assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']), "png magic");
                assert!(!d.alt.is_empty());
                assert_ne!(d.doc_id, 0);
            }
        }
    }

    #[test]
    fn open_chat_requests_are_handled_first() {
        let mut pending = vec![
            Request::MarkRead { id: 1 },
            Request::OpenChat { id: 2 },
            Request::DownloadAvatar { chat_id: 3 },
            Request::OpenChat { id: 4 },
        ];
        prioritize(&mut pending);
        let first_opens: Vec<i64> = pending
            .iter()
            .take_while(|r| matches!(r, Request::OpenChat { .. }))
            .map(|r| match r {
                Request::OpenChat { id } => *id,
                _ => unreachable!(),
            })
            .collect();
        assert!(
            first_opens.contains(&2) && first_opens.contains(&4),
            "all OpenChat requests must be processed before slower work"
        );
        assert!(
            pending
                .iter()
                .skip(first_opens.len())
                .all(|r| !matches!(r, Request::OpenChat { .. })),
            "no OpenChat must remain after the non-OpenChat work"
        );
    }

    #[test]
    fn prioritize_is_stable_with_a_single_open_chat() {
        let mut pending = vec![
            Request::DownloadAvatar { chat_id: 1 },
            Request::OpenChat { id: 9 },
        ];
        prioritize(&mut pending);
        assert!(matches!(pending[0], Request::OpenChat { id: 9 }));
    }
}
