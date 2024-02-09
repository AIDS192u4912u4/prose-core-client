// prose-core-client/prose-core-client
//
// Copyright: 2023, Marc Bauer <mb@nesium.com>
// License: Mozilla Public License v2.0 (MPL v2.0)

use anyhow::Result;
use chrono::{DateTime, Utc};
use jid::{BareJid, Jid};
use serde::{Deserialize, Serialize};
use tracing::error;
use uuid::Uuid;
use xmpp_parsers::message::MessageType;

use prose_xmpp::mods::chat::Carbon;
use prose_xmpp::stanza::message;
use prose_xmpp::stanza::message::{mam, stanza_id, Forwarded, Message};

use crate::domain::messaging::models::Attachment;
use crate::domain::shared::models::{OccupantId, ParticipantId, UserId};
use crate::infra::xmpp::type_conversions::stanza_error::StanzaErrorExt;

use super::{MessageId, StanzaId, StanzaParseError};

#[derive(thiserror::Error, Debug)]
pub enum MessageLikeError {
    #[error("No payload in message")]
    NoPayload,
}

/// A type that describes permanent messages, i.e. messages that need to be replayed to restore
/// the complete history of a conversation. Note that ephemeral messages like chat states are
/// handled differently.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct MessageLike {
    pub id: MessageLikeId,
    pub stanza_id: Option<StanzaId>,
    pub target: Option<MessageId>,
    pub to: Option<BareJid>,
    pub from: ParticipantId,
    pub timestamp: DateTime<Utc>,
    pub payload: Payload,
}

/// A ID that can act as a placeholder in the rare cases when a message doesn't have an ID. Since
/// our DataCache backends require some ID for each message we simply generate one.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct MessageLikeId(MessageId);

impl MessageLikeId {
    pub fn new(id: Option<MessageId>) -> Self {
        if let Some(id) = id {
            return MessageLikeId(id);
        }
        return MessageLikeId(format!("!!{}", Uuid::new_v4().to_string()).into());
    }

    /// Returns either the original message ID or the generated one.
    pub fn id(&self) -> &MessageId {
        &self.0
    }

    /// Returns the original message ID or None if we contain a generated ID.
    pub fn into_original_id(self) -> Option<MessageId> {
        if self.0.as_ref().starts_with("!!") {
            return None;
        }
        return Some(self.0);
    }

    pub fn original_id(&self) -> Option<&MessageId> {
        if self.0.as_ref().starts_with("!!") {
            return None;
        }
        return Some(&self.0);
    }
}

impl std::str::FromStr for MessageLikeId {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(MessageLikeId(s.to_string().into()))
    }
}

impl ToString for MessageLikeId {
    fn to_string(&self) -> String {
        self.0.to_string()
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(tag = "type")]
pub enum Payload {
    Correction {
        body: String,
        attachments: Vec<Attachment>,
    },
    DeliveryReceipt,
    ReadReceipt,
    Message {
        body: String,
        attachments: Vec<Attachment>,
    },
    Reaction {
        emojis: Vec<message::Emoji>,
    },
    Retraction,
}

impl Payload {
    pub fn is_message(&self) -> bool {
        match self {
            Self::Message { .. } => true,
            _ => false,
        }
    }
}

/// A wrapper for messages that might not contain a `delay` node with a timestamp, i.e. a received
/// or sent message (or more generally: a message not loaded from MAM).
pub struct TimestampedMessage<T> {
    pub message: T,
    pub timestamp: DateTime<Utc>,
}

impl TryFrom<TimestampedMessage<Carbon>> for MessageLike {
    type Error = anyhow::Error;

    fn try_from(envelope: TimestampedMessage<Carbon>) -> Result<Self> {
        let carbon = match envelope.message {
            Carbon::Received(carbon) => carbon,
            Carbon::Sent(carbon) => carbon,
        };

        let stanza_id = carbon
            .stanza
            .as_ref()
            .and_then(|s| s.stanza_id())
            .map(|sid| sid.id);
        MessageLike::try_from((stanza_id, &carbon))
    }
}

impl TryFrom<TimestampedMessage<Message>> for MessageLike {
    type Error = anyhow::Error;

    fn try_from(envelope: TimestampedMessage<Message>) -> Result<Self> {
        let msg = envelope.message;

        let id = MessageLikeId::new(msg.id.as_ref().map(|id| id.into()));
        let stanza_id = msg.stanza_id();
        let from = msg.resolved_from()?;
        let to = msg.to.as_ref();
        let timestamp = msg
            .delay()
            .map(|delay| delay.stamp.0.into())
            .unwrap_or(envelope.timestamp);
        let TargetedPayload {
            target: refs,
            payload,
        } = TargetedPayload::try_from(&msg)?;

        Ok(MessageLike {
            id,
            stanza_id: stanza_id.map(|s| s.id.as_ref().into()),
            target: refs.map(|id| id.as_ref().into()),
            to: to.map(|jid| jid.to_bare()),
            from,
            timestamp: timestamp.into(),
            payload,
        })
    }
}

impl TryFrom<&mam::ArchivedMessage> for MessageLike {
    type Error = anyhow::Error;

    fn try_from(carbon: &mam::ArchivedMessage) -> Result<Self> {
        MessageLike::try_from((Some(carbon.id.clone()), &carbon.forwarded))
    }
}

impl TryFrom<(Option<stanza_id::Id>, &Forwarded)> for MessageLike {
    type Error = anyhow::Error;

    fn try_from(value: (Option<stanza_id::Id>, &Forwarded)) -> Result<Self> {
        let Some(stanza_id) = value.0 else {
            return Err(anyhow::format_err!("Missing stanza_id in ForwardedMessage"));
        };
        let carbon = value.1;

        let message = *carbon
            .stanza
            .as_ref()
            .ok_or(StanzaParseError::missing_child_node("message"))?
            .clone();

        let TargetedPayload {
            target: refs,
            payload,
        } = TargetedPayload::try_from(&message)?;

        let id = MessageLikeId::new(message.id.as_ref().map(|id| id.into()));
        let to = message.to.as_ref();
        let from = message.resolved_from()?;
        let timestamp = &carbon
            .delay
            .as_ref()
            .ok_or(StanzaParseError::missing_child_node("delay"))?
            .stamp;

        Ok(MessageLike {
            id,
            stanza_id: Some(stanza_id.as_ref().into()),
            target: refs.map(|id| id.as_ref().into()),
            to: to.map(|jid| jid.to_bare()),
            from,
            timestamp: timestamp.0.into(),
            payload,
        })
    }
}

struct TargetedPayload {
    target: Option<message::Id>,
    payload: Payload,
}

impl TryFrom<&Message> for TargetedPayload {
    type Error = anyhow::Error;

    fn try_from(message: &Message) -> Result<Self> {
        if let Some(error) = &message.error() {
            return Ok(TargetedPayload {
                target: None,
                payload: Payload::Message {
                    body: format!("Error: {}", error.to_string()),
                    attachments: vec![],
                },
            });
        }

        if let Some(reactions) = message.reactions() {
            return Ok(TargetedPayload {
                target: Some(reactions.id),
                payload: Payload::Reaction {
                    emojis: reactions.reactions,
                },
            });
        };

        if let Some(fastening) = message.fastening() {
            if fastening.retract() {
                return Ok(TargetedPayload {
                    target: Some(fastening.id),
                    payload: Payload::Retraction,
                });
            }
        }

        if let (Some(replace_id), Some(body)) = (message.replace(), message.body()) {
            return Ok(TargetedPayload {
                target: Some(replace_id),
                payload: Payload::Correction {
                    body: body.to_string(),
                    attachments: message
                        .oob_attachments()
                        .iter()
                        .cloned()
                        .map(TryFrom::try_from)
                        .collect::<Result<Vec<_>, _>>()?,
                },
            });
        }

        if let Some(marker) = message.received_marker() {
            return Ok(TargetedPayload {
                target: Some(marker.id),
                payload: Payload::DeliveryReceipt,
            });
        }

        if let Some(marker) = message.displayed_marker() {
            return Ok(TargetedPayload {
                target: Some(marker.id),
                payload: Payload::ReadReceipt,
            });
        }

        if let Some(body) = message.body() {
            return Ok(TargetedPayload {
                target: None,
                payload: Payload::Message {
                    body: body.to_string(),
                    attachments: message
                        .oob_attachments()
                        .iter()
                        .cloned()
                        .map(TryFrom::try_from)
                        .collect::<Result<Vec<_>, _>>()?,
                },
            });
        }

        error!("Failed to parse message {:?}", message);
        Err(MessageLikeError::NoPayload.into())
    }
}

trait MessageExt {
    /// Returns either the real jid from a muc user or the original `from` value.
    fn resolved_from(&self) -> Result<ParticipantId, StanzaParseError>;
}

impl MessageExt for Message {
    fn resolved_from(&self) -> Result<ParticipantId, StanzaParseError> {
        let Some(from) = &self.from else {
            return Err(StanzaParseError::missing_attribute("from"));
        };

        match self.type_ {
            MessageType::Groupchat => {
                if let Some(muc_user) = &self.muc_user() {
                    if let Some(jid) = &muc_user.jid {
                        return Ok(ParticipantId::User(jid.to_bare().into()));
                    }
                }
                let Jid::Full(from) = from else {
                    return Err(StanzaParseError::ParseError {
                        error: "Expected `from` attribute to contain FullJid for groupchat message"
                            .to_string(),
                    });
                };
                Ok(ParticipantId::Occupant(OccupantId::from(from.clone())))
            }
            _ => Ok(ParticipantId::User(UserId::from(from.to_bare()))),
        }
    }
}
