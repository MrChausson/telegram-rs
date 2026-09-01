//! User blocking/privacy of [`Telegram`]: `messages.block` / `messages.unblock`.
//!
//! Split out of `client.rs` following the [`crate::admin`] precedent — another
//! `impl` block on the same type.
//!
//! TL shapes verified against the vendored grammers generated sources:
//! `messages.block#2e4f1dcd {id: InputPeer, my_stories_from: flags.0?true} = Bool`
//! and `messages.unblock#bea65d50 {id: InputPeer, my_stories_from: flags.0?true} = Bool`.
//! Both take an `InputPeer` (built from the chat's `PeerRef` via `.into()`, the
//! same trick `client.rs:set_typing` already uses) and return `Bool` on success.

use anyhow::{Context, Result};
use grammers_session::types::PeerRef;

use super::client::Telegram;

impl Telegram {
    /// Blocks the user behind `peer_ref` (`messages.block`): they can no
    /// longer message you, see your online status, or add you to groups.
    pub async fn block_user(&self, peer_ref: &PeerRef) -> Result<()> {
        self.client()
            .invoke(&grammers_client::tl::functions::contacts::Block {
                id: (*peer_ref).into(),
                my_stories_from: false,
            })
            .await
            .context("blocking user")?;
        Ok(())
    }

    /// Lifts a block on `peer_ref` (`messages.unblock`).
    pub async fn unblock_user(&self, peer_ref: &PeerRef) -> Result<()> {
        self.client()
            .invoke(&grammers_client::tl::functions::contacts::Unblock {
                id: (*peer_ref).into(),
                my_stories_from: false,
            })
            .await
            .context("unblocking user")?;
        Ok(())
    }
}