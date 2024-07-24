// prose-core-client/prose-core-client
//
// Copyright: 2023, Marc Bauer <mb@nesium.com>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use tracing::{error, info, warn};

use prose_utils::id_string;

use crate::domain::shared::models::ParticipantId;
use crate::dtos::{Attachment, MessageRemoteId, MessageServerId, HTML};

use super::{Mention, MessageLike, MessageLikePayload, MessageTargetId};

id_string!(Emoji);

#[derive(Clone, Debug, PartialEq)]
pub struct Reaction {
    pub emoji: Emoji,
    pub from: Vec<ParticipantId>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Body {
    pub raw: String,
    pub html: HTML,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub remote_id: Option<MessageRemoteId>,
    pub server_id: Option<MessageServerId>,
    pub from: ParticipantId,
    pub body: Body,
    pub timestamp: DateTime<Utc>,
    pub is_read: bool,
    pub is_edited: bool,
    pub is_delivered: bool,
    pub is_transient: bool,
    pub is_encrypted: bool,
    pub reactions: Vec<Reaction>,
    pub attachments: Vec<Attachment>,
    pub mentions: Vec<Mention>,
}

impl Message {
    pub fn toggle_reaction(&mut self, user_id: &ParticipantId, emoji: Emoji) {
        let Some(reaction) = self
            .reactions
            .iter_mut()
            .find(|reaction| reaction.emoji == emoji)
        else {
            self.reactions.push(Reaction {
                emoji,
                from: vec![user_id.clone()],
            });
            return;
        };

        if let Some(idx) = reaction.from.iter().position(|jid| jid == user_id) {
            reaction.from.remove(idx);
        } else {
            reaction.from.push(user_id.clone())
        }
    }

    pub fn reactions_from<'a, 'b: 'a>(
        &'a self,
        user_id: &'b ParticipantId,
    ) -> impl Iterator<Item = &'a Emoji> {
        self.reactions
            .iter()
            .filter(|reaction| reaction.from.contains(user_id))
            .map(|reaction| &reaction.emoji)
    }
}

impl Message {
    pub(crate) fn reducing_messages(
        messages: impl IntoIterator<Item = MessageLike>,
    ) -> Vec<Message> {
        let mut messages_map = IndexMap::new();
        let mut stanza_to_id_map = HashMap::new();
        let mut modifiers: Vec<MessageLike> = vec![];

        for msg in messages.into_iter() {
            match msg.payload {
                MessageLikePayload::Message {
                    body,
                    attachments,
                    encryption_info,
                    is_transient: is_private,
                } => {
                    let message_id = msg.id.clone();

                    let message = Message {
                        remote_id: message_id.into_original_id(),
                        server_id: msg.stanza_id,
                        from: msg.from.into(),
                        body: Body {
                            raw: body.raw,
                            html: body.html,
                        },
                        timestamp: msg.timestamp.into(),
                        is_read: false,
                        is_edited: false,
                        is_delivered: false,
                        is_transient: is_private,
                        is_encrypted: encryption_info.is_some(),
                        reactions: vec![],
                        attachments,
                        mentions: body.mentions,
                    };

                    if let Some(stanza_id) = &message.server_id {
                        stanza_to_id_map.insert(stanza_id.clone(), msg.id.id().clone());
                    };

                    messages_map.insert(msg.id.id().clone(), Some(message));
                }
                MessageLikePayload::Error { message: error } => {
                    let message_id = msg.id.clone();

                    let message = Message {
                        remote_id: message_id.into_original_id(),
                        server_id: msg.stanza_id,
                        from: msg.from.into(),
                        body: Body {
                            raw: error.clone(),
                            html: HTML::new(error),
                        },
                        timestamp: msg.timestamp.into(),
                        is_read: false,
                        is_edited: false,
                        is_delivered: false,
                        is_transient: false,
                        is_encrypted: false,
                        reactions: vec![],
                        attachments: vec![],
                        mentions: vec![],
                    };

                    if let Some(stanza_id) = &message.server_id {
                        stanza_to_id_map.insert(stanza_id.clone(), msg.id.id().clone());
                    };

                    messages_map.insert(msg.id.id().clone(), Some(message));
                }
                _ => modifiers.push(msg),
            }
        }

        for modifier in modifiers.into_iter() {
            let Some(target_id) = modifier.target else {
                error!("Missing target in modifier {:?}", modifier);
                continue;
            };

            let message_id = match target_id {
                MessageTargetId::RemoteId(ref id) => id,
                MessageTargetId::ServerId(stanza_id) => {
                    let Some(id) = stanza_to_id_map.get(&stanza_id) else {
                        info!("Could not resolve StanzaId '{stanza_id}' to a MessageId");
                        continue;
                    };
                    id
                }
            };

            let Some(Some(message)) = messages_map.get_mut(message_id) else {
                warn!("Ignoring message modifier targeting unknown message '{message_id}'.");
                continue;
            };

            match modifier.payload {
                MessageLikePayload::Correction {
                    body,
                    attachments,
                    encryption_info,
                } => {
                    message.body = Body {
                        raw: body.raw,
                        html: body.html,
                    };
                    message.mentions = body.mentions;
                    message.is_edited = true;
                    message.attachments = attachments;
                    message.is_encrypted = encryption_info.is_some()
                }
                MessageLikePayload::DeliveryReceipt => message.is_delivered = true,
                MessageLikePayload::ReadReceipt => message.is_read = true,
                MessageLikePayload::Message { .. } | MessageLikePayload::Error { .. } => {
                    unreachable!()
                }
                MessageLikePayload::Reaction { mut emojis } => {
                    let modifier_from = ParticipantId::from(modifier.from);

                    // Iterate over all existing reactions
                    'outer: for reaction in &mut message.reactions {
                        let mut idx: i32 = (emojis.len() as i32) - 1;

                        // Iterate over emojis to be applied
                        for emoji in emojis.iter().rev() {
                            // If the emoji is the same as the reaction…
                            if emoji.as_ref() == reaction.emoji.as_ref() {
                                // …add the author if needed
                                if !reaction.from.contains(&modifier_from) {
                                    reaction.from.push(modifier_from.clone())
                                }
                                // Remove the applied emoji from the list of emojis
                                emojis.remove(idx as usize);
                                // Continue with next reaction
                                continue 'outer;
                            }
                            idx -= 1;
                        }

                        // We couldn't find an emoji for this reaction, so remove our author
                        // from it (if needed)…
                        reaction.from.retain(|from| from != &modifier_from);
                    }

                    // Remove all empty reactions
                    message
                        .reactions
                        .retain(|reaction| !reaction.from.is_empty());

                    // For each of the remaining emojis…
                    for emoji in emojis.into_iter() {
                        // …add a new reaction
                        message.reactions.push(Reaction {
                            emoji: emoji.into_inner().into(),
                            from: vec![modifier_from.clone()],
                        })
                    }
                }
                MessageLikePayload::Retraction => {
                    messages_map.insert(message_id.clone(), None);
                }
            }
        }

        messages_map.into_values().filter_map(|msg| msg).collect()
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use pretty_assertions::assert_eq;

    use prose_xmpp::bare;

    use crate::domain::messaging::models::MessageLikeBody;
    use crate::domain::shared::models::UserId;
    use crate::test::MessageBuilder;
    use crate::user_id;

    use super::*;

    #[test]
    fn test_toggle_reaction() {
        let mut message = MessageBuilder::new_with_index(1).build_message();
        assert!(message.reactions.is_empty());

        message.toggle_reaction(&user_id!("a@prose.org").into(), "🎉".into());
        assert_eq!(
            message.reactions,
            vec![Reaction {
                emoji: "🎉".into(),
                from: vec![user_id!("a@prose.org").into()]
            }]
        );

        message.toggle_reaction(&user_id!("b@prose.org").into(), "🎉".into());
        assert_eq!(
            message.reactions,
            vec![Reaction {
                emoji: "🎉".into(),
                from: vec![
                    user_id!("a@prose.org").into(),
                    user_id!("b@prose.org").into()
                ]
            }]
        );

        message.toggle_reaction(&user_id!("b@prose.org").into(), "✅".into());
        assert_eq!(
            message.reactions,
            vec![
                Reaction {
                    emoji: "🎉".into(),
                    from: vec![
                        user_id!("a@prose.org").into(),
                        user_id!("b@prose.org").into()
                    ]
                },
                Reaction {
                    emoji: "✅".into(),
                    from: vec![user_id!("b@prose.org").into()]
                }
            ]
        );

        message.toggle_reaction(&user_id!("a@prose.org").into(), "🎉".into());
        assert_eq!(
            message.reactions,
            vec![
                Reaction {
                    emoji: "🎉".into(),
                    from: vec![user_id!("b@prose.org").into()]
                },
                Reaction {
                    emoji: "✅".into(),
                    from: vec![user_id!("b@prose.org").into()]
                }
            ]
        );

        message.toggle_reaction(&user_id!("b@prose.org").into(), "🎉".into());
        assert_eq!(
            message.reactions,
            vec![
                Reaction {
                    emoji: "🎉".into(),
                    from: vec![]
                },
                Reaction {
                    emoji: "✅".into(),
                    from: vec![user_id!("b@prose.org").into()]
                }
            ]
        );
    }

    #[test]
    fn test_reactions_for_user() {
        let mut message = MessageBuilder::new_with_index(1).build_message();
        message.reactions = vec![
            Reaction {
                emoji: "🎉".into(),
                from: vec![
                    user_id!("a@prose.org").into(),
                    user_id!("b@prose.org").into(),
                ],
            },
            Reaction {
                emoji: "✅".into(),
                from: vec![user_id!("b@prose.org").into()],
            },
        ];

        assert_eq!(
            message
                .reactions_from(&user_id!("a@prose.org").into())
                .cloned()
                .collect::<Vec<Emoji>>(),
            vec!["🎉".into()]
        );
        assert_eq!(
            message
                .reactions_from(&user_id!("b@prose.org").into())
                .cloned()
                .collect::<Vec<Emoji>>(),
            vec!["🎉".into(), "✅".into()]
        );
    }

    #[test]
    fn test_reduces_emojis() {
        let messages = [
            MessageLike {
                id: "1".into(),
                stanza_id: Some("stanza-id-1".into()),
                target: None,
                to: Some(bare!("a@prose.org")),
                from: user_id!("b@prose.org").into(),
                timestamp: Utc
                    .with_ymd_and_hms(2023, 04, 07, 16, 00, 00)
                    .unwrap()
                    .into(),
                payload: MessageLikePayload::Message {
                    body: MessageLikeBody {
                        raw: "Hello World".to_string(),
                        html: String::from("Hello World").into(),
                        mentions: vec![],
                    },
                    attachments: vec![],
                    encryption_info: None,
                    is_transient: false,
                },
            },
            MessageLike {
                id: "2".into(),
                stanza_id: None,
                target: Some(MessageTargetId::RemoteId("1".into())),
                to: Some(bare!("a@prose.org")),
                from: user_id!("b@prose.org").into(),
                timestamp: Utc
                    .with_ymd_and_hms(2023, 04, 07, 16, 00, 01)
                    .unwrap()
                    .into(),
                payload: MessageLikePayload::Reaction {
                    emojis: vec!["👍".into()],
                },
            },
            MessageLike {
                id: "3".into(),
                stanza_id: None,
                target: Some(MessageTargetId::RemoteId("1".into())),
                to: Some(bare!("a@prose.org")),
                from: user_id!("c@prose.org").into(),
                timestamp: Utc
                    .with_ymd_and_hms(2023, 04, 07, 16, 00, 02)
                    .unwrap()
                    .into(),
                payload: MessageLikePayload::Reaction {
                    emojis: vec!["👍".into()],
                },
            },
            MessageLike {
                id: "4".into(),
                stanza_id: None,
                target: Some(MessageTargetId::RemoteId("1".into())),
                to: Some(bare!("a@prose.org")),
                from: user_id!("b@prose.org").into(),
                timestamp: Utc
                    .with_ymd_and_hms(2023, 04, 07, 16, 00, 03)
                    .unwrap()
                    .into(),
                payload: MessageLikePayload::Reaction {
                    emojis: vec!["👍".into(), "📼".into(), "🍿".into(), "☕️".into()],
                },
            },
            MessageLike {
                id: "5".into(),
                stanza_id: None,
                target: Some(MessageTargetId::ServerId("stanza-id-1".into())),
                to: Some(bare!("a@prose.org")),
                from: user_id!("b@prose.org").into(),
                timestamp: Utc
                    .with_ymd_and_hms(2023, 04, 07, 16, 00, 04)
                    .unwrap()
                    .into(),
                payload: MessageLikePayload::Reaction {
                    emojis: vec!["📼".into(), "🍿".into()],
                },
            },
        ];

        let reduced_message = Message::reducing_messages(messages).pop().unwrap();
        assert_eq!(
            Message {
                remote_id: Some("1".into()),
                server_id: Some("stanza-id-1".into()),
                from: user_id!("b@prose.org").into(),
                body: Body {
                    raw: "Hello World".to_string(),
                    html: "Hello World".to_string().into(),
                },
                timestamp: Utc
                    .with_ymd_and_hms(2023, 04, 07, 16, 00, 00)
                    .unwrap()
                    .into(),
                is_read: false,
                is_edited: false,
                is_delivered: false,
                is_transient: false,
                is_encrypted: false,
                reactions: vec![
                    Reaction {
                        emoji: "👍".into(),
                        from: vec![user_id!("c@prose.org").into()]
                    },
                    Reaction {
                        emoji: "📼".into(),
                        from: vec![user_id!("b@prose.org").into()]
                    },
                    Reaction {
                        emoji: "🍿".into(),
                        from: vec![user_id!("b@prose.org").into()]
                    }
                ],
                attachments: vec![],
                mentions: vec![]
            },
            reduced_message,
        )
    }
}
