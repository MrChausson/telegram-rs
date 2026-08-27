//! Authentication flows of [`Telegram`]: session check, phone-code sign-in,
//! and two-factor verification.
//!
//! Split out of `client.rs` so network features keep growing without
//! crowding one module; this is still the same type, just another
//! `impl` block.

use anyhow::{Context, Result};
use grammers_client::client::{LoginToken, PasswordToken, SignInError};
use grammers_client::tl;
use grammers_client::Client;

use super::client::Telegram;

impl Telegram {
    /// Returns true if a valid session is already connected.
    pub async fn is_authorized(&self) -> Result<bool> {
        self.client()
            .is_authorized()
            .await
            .context("checking authorization")
    }

    /// Step 1: request a verification code for a phone number.
    pub async fn request_login_code(&self, api_hash: &str, phone: &str) -> Result<LoginToken> {
        self.client()
            .request_login_code(phone, api_hash)
            .await
            .context("requesting code")
    }

    /// Step 2: sign in with the received code (may require a 2FA password).
    pub async fn sign_in(
        &self,
        token: &LoginToken,
        code: &str,
    ) -> Result<Result<(), SignInError>> {
        Ok(self.client().sign_in(token, code).await.map(|_| ()))
    }

    /// Step 2b: sign in with the 2FA password.
    ///
    /// `Ok(Ok(()))` means signed in; `Ok(Err(new_token))` means the password
    /// was wrong and Telegram supplied a fresh token to retry with.
    pub async fn check_password(
        &self,
        password_token: PasswordToken,
        password: &str,
    ) -> Result<Result<(), PasswordToken>, anyhow::Error> {
        match self.client().check_password(password_token, password).await {
            Ok(_) => Ok(Ok(())),
            Err(SignInError::InvalidPassword(new_token)) => Ok(Err(new_token)),
            Err(SignInError::PasswordRequired(new_token)) => Ok(Err(new_token)),
            Err(e) => Err(anyhow::anyhow!("2FA sign in: {e}")),
        }
    }

    /// Logs this session out server-side (`auth.logOut`).
    pub async fn log_out(&self) -> Result<()> {
        use grammers_client::tl;
        self.client()
            .invoke(&tl::functions::auth::LogOut {})
            .await
            .map(|_: tl::enums::auth::LoggedOut| ())
            .context("logging out")
    }

    /// Starts a QR-login session (raw `auth.exportLoginToken`).
    pub async fn export_login_token(
        &self,
        api_id: i32,
        api_hash: &str,
    ) -> Result<tl::enums::auth::LoginToken> {
        export_login_token(self.client(), api_id, api_hash).await
    }

    /// Polls a QR-login session (`auth.importLoginToken`).
    pub async fn import_login_token(&self, token: Vec<u8>) -> Result<tl::enums::auth::LoginToken> {
        import_login_token(self.client(), token).await
    }
}

/// Raw `auth.exportLoginToken`. Free-standing over any [`Client`] because
/// the QR-login poller runs detached with its own client clone.
pub async fn export_login_token(
    client: &Client,
    api_id: i32,
    api_hash: &str,
) -> Result<tl::enums::auth::LoginToken> {
    client
        .invoke(&tl::functions::auth::ExportLoginToken {
            except_ids: Vec::new(),
            api_id,
            api_hash: api_hash.to_string(),
        })
        .await
        .context("exporting login token")
}

/// Raw `auth.importLoginToken` (QR-login polling step).
pub async fn import_login_token(
    client: &Client,
    token: Vec<u8>,
) -> Result<tl::enums::auth::LoginToken> {
    client
        .invoke(&tl::functions::auth::ImportLoginToken { token })
        .await
        .context("importing login token")
}
