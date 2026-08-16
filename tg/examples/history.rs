//! Prints the history of the first chat (validates `get_messages`).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use tg::client::Telegram;
use tg::session::load_or_new;

const ENV_PATH: &str = ".env";
const SESSION_PATH: &str = ".tg.session";

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
        .ok_or_else(|| anyhow::anyhow!("missing variable {name}"))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let env = load_env(ENV_PATH);
    let api_id: i32 = var(&env, "API_ID")?.parse()?;

    let session = Arc::new(load_or_new(Path::new(SESSION_PATH)));
    let tg = Telegram::connect(session, api_id).await?;
    if !tg.is_authorized().await? {
        anyhow::bail!("not signed in: run the login example first");
    }

    let dialogs = tg.get_dialogs().await?;
    let Some(first) = dialogs.first() else {
        anyhow::bail!("no chats");
    };
    println!("== {} ({} unread) ==", first.title, first.unread_count);

    let messages = tg.get_messages(&first.peer_ref, 10).await?;
    for m in messages {
        let who = if m.out { "me" } else { "them" };
        let text = m.text.replace('\n', " \\n ");
        let text: String = text.chars().take(120).collect();
        println!("[{who}] {text}");
    }

    tg.shutdown().await;
    Ok(())
}