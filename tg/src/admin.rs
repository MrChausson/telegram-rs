//! Group admin tools of [`Telegram`]: promote/demote admins, ban and remove
//! members of channels/supergroups.
//!
//! Split out of `client.rs` following the [`crate::auth`] precedent so
//! network features keep growing without crowding one module; this is still
//! the same type, just another `impl` block.
//!
//! TL shapes were verified against the vendored grammers generated sources:
//! `channels.editAdmin#d33c8902 {channel: InputChannel, user_id: InputUser,
//! admin_rights: ChatAdminRights, rank: string}` and
//! `channels.editBanned#96e6cd81 {channel: InputChannel,
//! participant: InputPeer, banned_rights: ChatBannedRights}`.
//! `ChatAdminRights` is sixteen plain `bool` flags (set = right granted);
//! `ChatBannedRights` flags are inverted (set = right revoked) plus a
//! required `until_date: i32`. "Ban" therefore sends every flag set with
//! `until_date = 0` (forever); "remove" sends the same revocations with
//! `until_date = now`, i.e. a zero-duration ban that has already expired —
//! the user is out but may rejoin (the same trick grammers' own
//! `kick_participant` uses with its 60 s variant).

use anyhow::{Context, Result};
use grammers_client::tl;
use grammers_session::types::{PeerAuth, PeerId, PeerRef};

use super::client::Telegram;

/// Admin rights with every capability granted (promote) — or none (demote).
fn admin_rights(all: bool) -> tl::enums::ChatAdminRights {
    tl::enums::ChatAdminRights::Rights(tl::types::ChatAdminRights {
        change_info: all,
        post_messages: all,
        edit_messages: all,
        delete_messages: all,
        ban_users: all,
        invite_users: all,
        pin_messages: all,
        add_admins: all,
        anonymous: all,
        manage_call: all,
        other: all,
        manage_topics: all,
        post_stories: all,
        edit_stories: all,
        delete_stories: all,
        manage_direct_messages: all,
    })
}

/// Banned rights revoking every messaging right (`view_messages` set = the
/// member cannot even read the group) until `until_date` (`0` = forever).
fn banned_rights(until_date: i32) -> tl::enums::ChatBannedRights {
    tl::enums::ChatBannedRights::Rights(tl::types::ChatBannedRights {
        view_messages: true,
        send_messages: true,
        send_media: true,
        send_stickers: true,
        send_gifs: true,
        send_games: true,
        send_inline: true,
        embed_links: true,
        send_polls: true,
        change_info: true,
        invite_users: true,
        pin_messages: true,
        manage_topics: true,
        send_photos: true,
        send_videos: true,
        send_roundvideos: true,
        send_audios: true,
        send_voices: true,
        send_docs: true,
        send_plain: true,
        until_date,
    })
}

/// Wraps a bare user id into an `InputUser`, resolving it through the session
/// first so the request carries the cached access hash (same sequence as
/// `Telegram::kick_participant`).
fn input_user(user_id: i64) -> tl::enums::InputUser {
    tl::enums::InputUser::User(tl::types::InputUser {
        user_id,
        access_hash: PeerAuth::default().hash(),
    })
}

impl Telegram {
    /// Grants every admin right to `user_id` (`channels.editAdmin`, all
    /// flags set). Channels/supergroups only.
    pub async fn promote_admin(&self, channel: &PeerRef, user_id: i64) -> Result<()> {
        let res = self
            .client()
            .invoke(&tl::functions::channels::EditAdmin {
                channel: (*channel).into(),
                user_id: input_user(user_id),
                admin_rights: admin_rights(true),
                rank: String::new(),
            })
            .await
            .context("promoting to admin")?;
        let _: tl::enums::Updates = res;
        Ok(())
    }

    /// Revokes every admin right from `user_id` (`channels.editAdmin`, all
    /// flags cleared). Channels/supergroups only.
    pub async fn demote_admin(&self, channel: &PeerRef, user_id: i64) -> Result<()> {
        let res = self
            .client()
            .invoke(&tl::functions::channels::EditAdmin {
                channel: (*channel).into(),
                user_id: input_user(user_id),
                admin_rights: admin_rights(false),
                rank: String::new(),
            })
            .await
            .context("demoting admin")?;
        let _: tl::enums::Updates = res;
        Ok(())
    }

    /// Bans `user_id` from the group (`kick_only=false`, forever) or just
    /// removes them while letting them rejoin (`kick_only=true`).
    ///
    /// Channels/supergroups use a single `channels.editBanned` call: a full
    /// rights revocation with `until_date = 0` (ban) or `until_date = now`
    /// (remove — an already-expired ban, i.e. a plain kick). Basic groups
    /// have no server-side ban concept in MTProto, so both flavours fall
    /// back to [`Telegram::kick_participant`].
    pub async fn ban_member(
        &self,
        channel: &PeerRef,
        user_id: i64,
        kick_only: bool,
    ) -> Result<()> {
        if channel.id.kind() == grammers_session::types::PeerKind::Chat {
            return self.kick_participant(channel, user_id).await;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i32)
            .unwrap_or(0);
        let until_date = if kick_only { now } else { 0 };
        // Mirror `kick_participant`: a PeerRef with the ambient authority
        // converts to InputPeer/InputUser carrying the session-cached hash.
        let user = PeerRef {
            id: PeerId::user(user_id),
            auth: PeerAuth::default(),
        };
        let _ = self.client().resolve_peer(user).await;
        let res = self
            .client()
            .invoke(&tl::functions::channels::EditBanned {
                channel: (*channel).into(),
                participant: user.into(),
                banned_rights: banned_rights(until_date),
            })
            .await
            .context(if kick_only { "removing member" } else { "banning member" })?;
        let _: tl::enums::Updates = res;
        Ok(())
    }

    /// Resolves the signed-in user's own bot-api id (`users.getUsers` with
    /// `InputUser::UserSelf`); the UI needs it to hide admin actions on
    /// yourself.
    pub async fn self_user_id(&self) -> Result<i64> {
        let me = self
            .client()
            .get_me()
            .await
            .context("resolving own user id")?;
        Ok(me.id().bot_api_dialog_id())
    }
}
