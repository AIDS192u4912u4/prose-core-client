// prose-core-client
//
// Copyright: 2022, Marc Bauer <mb@nesium.com>
// License: Mozilla Public License v2.0 (MPL v2.0)

// -- Modules --

use jid::BareJid;
use std::sync::Mutex;

use prose_core_client::client::{ProseClient, ProseClientBuilder, ProseClientOrigin};

#[derive(Debug, thiserror::Error)]
pub enum InitializationError {
    #[error("ProseClient was initialized already.")]
    AlreadyInitialized,
}

#[derive(Debug, thiserror::Error)]
pub enum LoginError {
    #[error("An unknown error occurred.")]
    Unknown,
}

struct Client<'cl, 'cb> {
    client: Mutex<ProseClient<'cl, 'cb>>,
}

unsafe impl<'cl, 'cb> Send for Client<'cl, 'cb> {}
unsafe impl<'cl, 'cb> Sync for Client<'cl, 'cb> {}

impl<'cl, 'cb> Client<'cl, 'cb> {
    pub fn new(origin: ProseClientOrigin) -> Self {
        Self {
            client: Mutex::new(
                ProseClientBuilder::new()
                    .app(origin)
                    .build()
                    .expect("client built")
                    .bind()
                    .expect("client bound"),
            ),
        }
    }

    pub fn connect(&self, jid: &str, password: &str) -> Result<BareJid, LoginError> {
        // For now we convert these fancy nested errors into an obfuscated mess until we
        // have a proper error handling system. We'll probably need root-level flat error enums.
        let mut client = self.client.lock().unwrap();
        (*client)
            .add(jid, password)
            .map_err(|_| LoginError::Unknown)
    }
}

uniffi_macros::include_scaffolding!("ProseCoreClientFFI");
