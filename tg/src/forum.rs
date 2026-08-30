//! Forum topics of [`Telegram`]: detect forum supergroups, list and create
//! topics, and post text into a topic thread.
//!
//! Split out of `client.rs` following the [`crate::admin`] (itself
//! [`crate::auth`]) precedent so network features keep growing without
//! crowding one module; this is still the same type, another `impl` block.
//!
//! TL shapes were verified against the vendored grammers generated sources:
//! - `messages.getForumTopics#3ba47bff {peer: InputPeer, q:flags.0?string,
//!   offset_date: int, offset_id: int, offset_topic: int, limit: int}
//!   = messages.ForumTopics` — generated as `GetForumTopics {peer, q:
//!   Option<String>, offset_date, offset_id, offset_topic, limit}`, answered
//!   by `messages.forumTopics#367617d3 {count, topics: Vector<ForumTopic>,
//!   messages, chats, users, pts}` (single constructor `Topics`).
//! - `messages.createForumTopic#2f98c3d5 {flags, peer, title,
//!   icon_color:flags.0?int, icon_emoji_id:flags.3?long, random_id: long,
//!   send_as:flags.2?InputPeer} = Updates`.
//! - `forumTopic#cdff0eca {…, id: int, date: int, peer: Peer, title: string,
//!   icon_color: int, …, top_message: int, …}` — `id` is BOTH the topic id
//!   and the root message id of the thread (`top_message` is the LAST
//!   message, not the root; [`TopicInfo`] keeps `id`/`root_msg_id` as the
//!   same server value).
//! - `channel#… flags:# … forum:flags.30?true …` — the forum flag read by
//!   [`Telegram::is_forum`].
//! - Posting into a topic anchors the message to the thread with
//!   `inputReplyToMessage {reply_to_msg_id: 0, top_msg_id: <root id>}`;
//!   grammers' `InputMessage::reply_to` hardcodes `top_msg_id: None`, so
//!   [`Telegram::send_to_topic`] invokes `messages.sendMessage` directly.
//! - `createForumTopic` answers with the root service message
//!   (`Update::NewMessage(MessageService {action: TopicCreate, …})`), whose
//!   id is the new topic id (see [`Telegram::create_topic`]).

use anyhow::{Context, Result};
use grammers_client::tl;
use grammers_session::types::PeerRef;

use super::client::Telegram;
use super::model::TopicInfo;

/// How many topics one listing page asks for (a forum rarely has more).
const TOPICS_LIMIT: i32 = 100;

/// Fresh `random_id` for idempotent send/create calls (second clock in the
/// high bits, sub-second nanos in the low ones — collision-free enough for
/// a desktop client).
fn random_id() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    ((now.as_secs() as i64) << 20) ^ (now.subsec_nanos() as i64)
}

impl Telegram {
    /// True when `channel` is a forum host: a supergroup/channel with forums
    /// (topics) enabled (`channels.getChannels`, `Channel.forum` flag), or a
    /// private bot peer flagged as a forum host (`bot_forum_view` /
    /// `bot_forum_can_manage_topics` on the bot's `User` — such peers are not
    /// `Channel`s, so `GetChannels` returns an empty list for them).
    pub async fn is_forum(&self, channel: &PeerRef) -> Result<bool> {
        let res = self
            .client()
            .invoke(&tl::functions::channels::GetChannels {
                id: vec![(*channel).into()],
            })
            .await
            .context("fetching channel")?;
        if res.chats().into_iter().any(|c| match c {
            tl::enums::Chat::Channel(ch) => ch.forum,
            _ => false,
        }) {
            return Ok(true);
        }
        if let grammers_client::peer::Peer::User(u) = self
            .client()
            .resolve_peer(*channel)
            .await
            .context("resolving forum peer")?
        {
            if let tl::enums::User::User(raw) = u.raw {
                return Ok(raw.bot_forum_view || raw.bot_forum_can_manage_topics);
            }
        }
        Ok(false)
    }

    /// Lists the topics of a forum channel (`messages.getForumTopics`,
    /// first page). Non-forum channels answer with an empty list.
    pub async fn get_topics(&self, channel: &PeerRef) -> Result<Vec<TopicInfo>> {
        let res = self
            .client()
            .invoke(&tl::functions::messages::GetForumTopics {
                peer: (*channel).into(),
                q: None,
                offset_date: 0,
                offset_id: 0,
                offset_topic: 0,
                limit: TOPICS_LIMIT,
            })
            .await
            .context("fetching forum topics")?;
        let topics = match res {
            tl::enums::messages::ForumTopics::Topics(t) => t.topics,
        };
        Ok(topics
            .into_iter()
            .filter_map(|t| match t {
                tl::enums::ForumTopic::Topic(t) => Some(TopicInfo {
                    id: t.id as i64,
                    root_msg_id: t.id,
                    title: t.title,
                    icon_color: t.icon_color,
                }),
                // `forumTopicDeleted` — a tombstone, not a live topic.
                tl::enums::ForumTopic::Deleted(_) => None,
            })
            .collect())
    }

    /// Creates a topic titled `title` (`messages.createForumTopic`); returns
    /// the parsed root service message (`Some`), or `None` when the answer
    /// didn't carry one — the caller should re-list the topics in that case.
    pub async fn create_topic(
        &self,
        channel: &PeerRef,
        title: &str,
    ) -> Result<Option<TopicInfo>> {
        let res = self
            .client()
            .invoke(&tl::functions::messages::CreateForumTopic {
                title_missing: false,
                peer: (*channel).into(),
                title: title.to_string(),
                icon_color: None,
                icon_emoji_id: None,
                random_id: random_id(),
                send_as: None,
            })
            .await
            .context("creating topic")?;
        let updates = match res {
            tl::enums::Updates::Updates(u) => u.updates,
            tl::enums::Updates::Combined(u) => u.updates,
            _ => return Ok(None),
        };
        for u in updates {
            if let tl::enums::Update::NewMessage(tl::types::UpdateNewMessage {
                message: tl::enums::Message::Service(svc),
                ..
            }) = u
            {
                if let tl::enums::MessageAction::TopicCreate(t) = svc.action {
                    return Ok(Some(TopicInfo {
                        id: svc.id as i64,
                        root_msg_id: svc.id,
                        title: t.title,
                        icon_color: t.icon_color,
                    }));
                }
            }
        }
        Ok(None)
    }

    /// Sends a text message into the topic thread anchored by `topic_root`
    /// (`messages.sendMessage` with `inputReplyToMessage.top_msg_id` — the
    /// grammers `InputMessage` helper cannot set `top_msg_id`, see the
    /// module docs).
    pub async fn send_to_topic(
        &self,
        channel: &PeerRef,
        text: &str,
        topic_root: i32,
    ) -> Result<()> {
        let res = self
            .client()
            .invoke(&tl::functions::messages::SendMessage {
                no_webpage: false,
                silent: false,
                background: false,
                clear_draft: false,
                noforwards: false,
                update_stickersets_order: false,
                invert_media: false,
                allow_paid_floodskip: false,
                peer: (*channel).into(),
                reply_to: Some(tl::enums::InputReplyTo::Message(
                    tl::types::InputReplyToMessage {
                        reply_to_msg_id: 0,
                        top_msg_id: Some(topic_root),
                        reply_to_peer_id: None,
                        quote_text: None,
                        quote_entities: None,
                        quote_offset: None,
                        monoforum_peer_id: None,
                        todo_item_id: None,
                    },
                )),
                message: text.to_string(),
                random_id: random_id(),
                reply_markup: None,
                entities: None,
                schedule_date: None,
                schedule_repeat_period: None,
                send_as: None,
                quick_reply_shortcut: None,
                effect: None,
                allow_paid_stars: None,
                suggested_post: None,
            })
            .await
            .context("sending topic message")?;
        let _: tl::enums::Updates = res;
        Ok(())
    }
}
