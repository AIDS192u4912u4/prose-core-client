// prose-core-client
//
// Copyright: 2022, Valerian Saliou <valerian@valeriansaliou.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

// -- Imports --

use libstrophe::{Connection, ConnectionEvent, ConnectionFlags, Context, Stanza};

// -- Structures --

pub struct ProseClientEvent;

// -- Implementations --

impl ProseClientEvent {
    pub fn connection(context: &Context, connection: &mut Connection, event: ConnectionEvent) {
        match event {
            ConnectionEvent::RawConnect => {
                log::trace!("[event] connected (raw)");

                // Nothing done here (as we never use raw connections)
            }
            ConnectionEvent::Connect => {
                log::trace!("[event] connected");

                // Bind stanza handlers
                connection.handler_add(Self::stanza_presence, None, Some("presence"), None);
                connection.handler_add(Self::stanza_message, None, Some("message"), None);
                connection.handler_add(Self::stanza_iq, None, Some("iq"), None);

                // Announce first presence
                // TODO: this should not be done from here, right?
                let presence = Stanza::new_presence();

                connection.send(&presence);
            }
            ConnectionEvent::Disconnect(err) => {
                log::trace!("[event] disconnected: {:?}", err);

                context.stop();
            }
        }
    }

    pub fn stanza_presence(
        context: &Context,
        connection: &mut Connection,
        stanza: &Stanza,
    ) -> bool {
        log::trace!("[event] presence from: {}", stanza.from().unwrap_or("--"));

        // TODO

        true
    }

    pub fn stanza_message(context: &Context, connection: &mut Connection, stanza: &Stanza) -> bool {
        log::trace!("[event] message from: {}", stanza.from().unwrap_or("--"));

        // TODO

        true
    }

    pub fn stanza_iq(context: &Context, connection: &mut Connection, stanza: &Stanza) -> bool {
        log::trace!("[event] iq from: {}", stanza.from().unwrap_or("--"));

        // TODO

        true
    }
}
