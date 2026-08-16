//! Console login: connection check and/or interactive authentication.
//!
//! Usage:
//!   cargo run -p tg --example login -- --check   # connect without sending a code
//!   cargo run -p tg --example login              # interactive login (phone -> code -> 2FA)

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::sync::Arc;

use grammers_client::client::SignInError;
use tg::client::Telegram;
use tg::session::{load_or_new, save};

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
        .ok_or_else(|| anyhow::anyhow!("missing variable {name} (in {ENV_PATH})"))
}

fn prompt(msg: &str) -> io::Result<String> {
    print!("{msg}");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

async fn interactive_login(tg: &Telegram, api_hash: &str) -> anyhow::Result<()> {
    if tg.is_authorized().await? {
        println!("Already signed in.");
        return Ok(());
    }
    let phone = prompt("Phone (international format, e.g. +33...): ")?;
    let token = tg.request_login_code(api_hash, &phone).await?;

    loop {
        let code = prompt("Code received by SMS: ")?;
        match tg.sign_in(&token, &code).await? {
            Ok(()) => {
                println!("Signed in!");
                return Ok(());
            }
            Err(SignInError::PasswordRequired(password_token)) => {
                let hint = password_token.hint().unwrap_or("none");
                let password = prompt(&format!("2FA required (hint: {hint}): "))?;
                match tg.check_password(password_token, &password).await {
                    Ok(Ok(())) => {
                        println!("Signed in (2FA)!");
                        return Ok(());
                    }
                    Ok(Err(_)) => {
                        anyhow::bail!("Invalid password, try again.");
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            Err(SignInError::InvalidCode) => {
                println!("Invalid code, try again.");
            }
            Err(SignInError::SignUpRequired) => {
                anyhow::bail!("This phone number is not registered. Sign up via the official app first.");
            }
            Err(e) => return Err(e.into()),
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let check = std::env::args().any(|a| a == "--check");

    let env = load_env(ENV_PATH);
    let api_id: i32 = var(&env, "API_ID")?.parse()?;
    let api_hash = var(&env, "API_HASH")?;

    let session = Arc::new(load_or_new(Path::new(SESSION_PATH)));
    let tg = Telegram::connect(session.clone(), api_id).await?;

    let authorized = tg.is_authorized().await?;
    println!("Server connection: OK (authorized = {authorized})");

    if check {
        tg.shutdown().await;
        return Ok(());
    }

    if !authorized {
        interactive_login(&tg, &api_hash).await?;
    }

    if tg.is_authorized().await? {
        save(&session, Path::new(SESSION_PATH))?;
        println!("Session saved to {SESSION_PATH}");
        if let Ok(me) = tg.client().get_me().await {
            println!("Signed in as: {}", me.first_name().unwrap_or("?"));
        }
    }

    tg.shutdown().await;
    Ok(())
}