//! Authentication flows of [`Telegram`]: session check, phone-code sign-in,
//! and two-factor verification.
//!
//! Split out of `client.rs` so network features keep growing without
//! crowding one module; this is still the same type, just another
//! `impl` block.

use anyhow::{Context, Result};
use grammers_client::client::{LoginToken, PasswordToken, SignInError};

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
}
