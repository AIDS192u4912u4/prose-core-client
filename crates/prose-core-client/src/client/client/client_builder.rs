// prose-core-client/prose-core-client
//
// Copyright: 2023, Marc Bauer <mb@nesium.com>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::sync::Arc;

use prose_xmpp::client::ConnectorProvider;
use prose_xmpp::mods::{Caps, Chat, Profile, Roster, Status, MAM};
use prose_xmpp::{
    ns, Client as XMPPClient, ClientBuilder as XMPPClientBuilder, IDProvider, SystemTimeProvider,
    TimeProvider,
};

use crate::avatar_cache::AvatarCache;
use crate::client::client::client::ClientInner;
use crate::data_cache::DataCache;
use crate::types::{Capabilities, Feature, SoftwareVersion};
use crate::{Client, ClientDelegate};

pub struct UndefinedDataCache {}
pub struct UndefinedAvatarCache {}

pub struct ClientBuilder<D, A> {
    builder: XMPPClientBuilder,
    data_cache: D,
    avatar_cache: A,
    time_provider: Arc<dyn TimeProvider>,
    software_version: SoftwareVersion,
    delegate: Option<Box<dyn ClientDelegate<D, A>>>,
}

impl ClientBuilder<UndefinedDataCache, UndefinedAvatarCache> {
    pub fn new() -> Self {
        ClientBuilder {
            builder: XMPPClient::builder(),
            data_cache: UndefinedDataCache {},
            avatar_cache: UndefinedAvatarCache {},
            time_provider: Arc::new(SystemTimeProvider::default()),
            software_version: SoftwareVersion::default(),
            delegate: None,
        }
    }
}

impl<A> ClientBuilder<UndefinedDataCache, A> {
    pub fn set_data_cache<D2: DataCache>(self, data_cache: D2) -> ClientBuilder<D2, A> {
        ClientBuilder {
            builder: self.builder,
            data_cache,
            avatar_cache: self.avatar_cache,
            time_provider: self.time_provider,
            software_version: self.software_version,
            delegate: None,
        }
    }
}

impl<D> ClientBuilder<D, UndefinedAvatarCache> {
    pub fn set_avatar_cache<A2: AvatarCache>(self, avatar_cache: A2) -> ClientBuilder<D, A2> {
        ClientBuilder {
            builder: self.builder,
            data_cache: self.data_cache,
            avatar_cache,
            time_provider: self.time_provider,
            software_version: self.software_version,
            delegate: None,
        }
    }
}

impl<D, A> ClientBuilder<D, A> {
    pub fn set_connector_provider(mut self, connector_provider: ConnectorProvider) -> Self {
        self.builder = self.builder.set_connector_provider(connector_provider);
        self
    }

    pub fn set_id_provider<P: IDProvider + 'static>(mut self, id_provider: P) -> Self {
        self.builder = self.builder.set_id_provider(id_provider);
        self
    }

    pub fn set_time_provider<T: TimeProvider + 'static>(mut self, time_provider: T) -> Self {
        self.time_provider = Arc::new(time_provider);
        self
    }

    pub fn set_software_version(mut self, software_version: SoftwareVersion) -> Self {
        self.software_version = software_version;
        self
    }
}

impl<D: DataCache, A: AvatarCache> ClientBuilder<D, A> {
    pub fn set_delegate(mut self, delegate: Option<Box<dyn ClientDelegate<D, A>>>) -> Self {
        self.delegate = delegate;
        self
    }

    pub fn build(self) -> Client<D, A> {
        let caps = Capabilities::new(
            self.software_version.name.clone(),
            "https://prose.org",
            vec![
                Feature::new(ns::JABBER_CLIENT, false),
                Feature::new(ns::AVATAR_DATA, false),
                Feature::new(ns::AVATAR_METADATA, false),
                Feature::new(ns::AVATAR_METADATA, true),
                Feature::new(ns::CHATSTATES, false),
                Feature::new(ns::DISCO_INFO, false),
                Feature::new(ns::RSM, false),
                Feature::new(ns::CAPS, false),
                Feature::new(ns::PING, false),
                Feature::new(ns::PUBSUB, false),
                Feature::new(ns::PUBSUB, true),
                Feature::new(ns::PUBSUB_EVENT, false),
                Feature::new(ns::ROSTER, false),
                Feature::new(ns::REACTIONS, false),
                Feature::new(ns::RECEIPTS, false),
                Feature::new(ns::CHAT_MARKERS, false),
                Feature::new(ns::MESSAGE_CORRECT, false),
                Feature::new(ns::RETRACT, false),
                Feature::new(ns::FASTEN, false),
                Feature::new(ns::DELAY, false),
                Feature::new(ns::FALLBACK, false),
                Feature::new(ns::HINTS, false),
                Feature::new(ns::MAM, false),
                Feature::new(ns::TIME, false),
                Feature::new(ns::VERSION, false),
                Feature::new(ns::LAST_ACTIVITY, false),
                Feature::new(ns::USER_ACTIVITY, false),
                Feature::new(ns::USER_ACTIVITY, true),
                Feature::new(ns::VCARD4, false),
                Feature::new(ns::VCARD4, true),
            ],
        );

        let inner = Arc::new(ClientInner {
            caps,
            data_cache: self.data_cache,
            avatar_cache: self.avatar_cache,
            time_provider: self.time_provider.clone(),
            software_version: self.software_version,
            delegate: self.delegate,
        });

        let event_inner = inner.clone();

        let client = self
            .builder
            .add_mod(Caps::default())
            .add_mod(MAM::default())
            .add_mod(Chat::default())
            .add_mod(Profile::default())
            .add_mod(Roster::default())
            .add_mod(Status::default())
            .set_time_provider(self.time_provider)
            .set_event_handler(Box::new(move |xmpp_client, event| {
                let client = Client {
                    client: xmpp_client,
                    inner: event_inner.clone(),
                };
                async move { client.handle_event(event).await }
            }))
            .build();

        Client { client, inner }
    }
}
