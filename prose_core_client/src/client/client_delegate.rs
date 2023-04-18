use jid::BareJid;

use prose_core_domain::MessageId;
use prose_core_lib::ConnectionEvent;

pub enum ClientEvent {
    /// The status of the connection has changed.
    ConnectionStatusChanged { event: ConnectionEvent },

    /// Infos about a contact have changed.
    ContactChanged { jid: BareJid },

    /// One or many messages were either received or sent.
    MessagesAppended {
        conversation: BareJid,
        message_ids: Vec<MessageId>,
    },

    /// One or many messages were received that affected earlier messages (e.g. a reaction).
    MessagesUpdated {
        conversation: BareJid,
        message_ids: Vec<MessageId>,
    },

    /// A message was deleted.
    MessagesDeleted {
        conversation: BareJid,
        message_ids: Vec<MessageId>,
    },
}

pub trait ClientDelegate: Send + Sync {
    fn handle_event(&self, event: ClientEvent);
}
