//! Sends an image file (photo) to the default test channel.
//! Usage: cargo run -p tg --example send_photo -- /path/to/img.jpg [--to "Chat"]

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use tg::client::Telegram;
use tg::session::load_or_new;

const ENV_PATH: &str = ".env";
const SESSION_PATH: &str = ".tg.session";
const DEFAULT_TARGET: &str = "Solo testing";

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
    env.get(name).cloned().or_else(|| std::env::var(name).ok())
        .ok_or_else(|| anyhow::anyhow!("missing {name}"))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut target = DEFAULT_TARGET.to_string();
    let mut image = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--to" {
            target = args[i + 1].clone();
            i += 2;
        } else {
            image = Some(args[i].clone());
            i += 1;
        }
    }
    let image = image.ok_or_else(|| anyhow::anyhow!("no image path given"))?;

    let env = load_env(ENV_PATH);
    let api_id: i32 = var(&env, "API_ID")?.parse()?;
    let session = Arc::new(load_or_new(Path::new(SESSION_PATH)));
    let tg = Telegram::connect(session, api_id).await?;
    if !tg.is_authorized().await? {
        anyhow::bail!("not signed in");
    }
    let dialogs = tg.get_dialogs().await?;
    let chat = dialogs.iter().find(|d| d.title == target)
        .ok_or_else(|| anyhow::anyhow!("chat \"{target}\" not found"))?;

    let cli = tg.client();
    let uploaded = cli.upload_file(&image).await?;
    let media = grammers_client::tl::enums::InputMedia::UploadedPhoto(
        grammers_client::tl::types::InputMediaUploadedPhoto {
            spoiler: false,
            file: uploaded.raw,
            stickers: None,
            ttl_seconds: None,
        },
    );
    cli.send_message(
        chat.peer_ref,
        grammers_client::message::InputMessage::new().media(media),
    )
    .await?;
    eprintln!("[send_photo] sent {image} to \"{target}\"");
    Ok(())
}
