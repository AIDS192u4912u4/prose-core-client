use std::cell::RefCell;
use std::rc::Rc;
use std::str::FromStr;

use anyhow::Result;
use async_trait::async_trait;
use jid::FullJid;
use minidom::Element;
use thiserror::Error;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::DomException;

use prose_xmpp::client::ConnectorProvider;
use prose_xmpp::connector::{
    Connection as ConnectionTrait, ConnectionError, ConnectionEvent, ConnectionEventHandler,
    Connector as ConnectorTrait,
};

use crate::util::Interval;

#[wasm_bindgen(typescript_custom_section)]
const TS_APPEND_CONTENT: &'static str = r#"
export interface ProseConnectionProvider {
    provideConnection(): ProseConnection
}

export interface ProseConnection {
    setEventHandler(handler: ProseConnectionEventHandler): void
    connect(jid: string, password: string): Promise<void>
    disconnect(): void
    sendStanza(stanza: string): void
}
"#;

#[wasm_bindgen(module = "/js/strophejs-connection.js")]
extern "C" {
    #[wasm_bindgen(typescript_type = "StropheJSConnectionProvider")]
    pub type JSConnectionProvider;

    #[wasm_bindgen(constructor)]
    pub fn new() -> JSConnectionProvider;

    #[wasm_bindgen(method, js_name = "provideConnection")]
    pub fn provide_connection(this: &JSConnectionProvider) -> JSConnection;
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "ProseConnection")]
    pub type JSConnection;

    #[wasm_bindgen(method, js_name = "setEventHandler")]
    fn set_event_handler(this: &JSConnection, handlers: EventHandler);

    #[wasm_bindgen(method, catch)]
    async fn connect(
        this: &JSConnection,
        jid: String,
        password: String,
    ) -> Result<(), DomException>;

    #[wasm_bindgen(method)]
    fn disconnect(this: &JSConnection);

    #[wasm_bindgen(method, catch, js_name = "sendStanza")]
    fn send_stanza(this: &JSConnection, stanza: String) -> Result<(), DomException>;
}

#[wasm_bindgen(js_name = "ProseConnectionEventHandler")]
pub struct EventHandler {
    connection: Connection,
    handler: Rc<ConnectionEventHandler>,
}

pub struct Connector {
    provider: Rc<JSConnectionProvider>,
}

impl Connector {
    pub fn provider(provider: JSConnectionProvider) -> ConnectorProvider {
        let provider = Rc::new(provider);
        Box::new(move || {
            Box::new(Connector {
                provider: provider.clone(),
            })
        })
    }
}

#[async_trait(? Send)]
impl ConnectorTrait for Connector {
    async fn connect(
        &self,
        jid: &FullJid,
        password: &str,
        event_handler: ConnectionEventHandler,
    ) -> Result<Box<dyn ConnectionTrait>, ConnectionError> {
        let client = Rc::new(self.provider.provide_connection());
        let event_handler = Rc::new(event_handler);

        let ping_interval = {
            let connection = Connection::new(client.clone());
            let event_handler = event_handler.clone();

            Interval::new(60_000, move || {
                let fut = (event_handler)(&connection, ConnectionEvent::PingTimer);
                spawn_local(async move { fut.await });
            })
        };

        let timeout_interval = {
            let connection = Connection::new(client.clone());
            let event_handler = event_handler.clone();

            Interval::new(5_000, move || {
                let fut = (event_handler)(&connection, ConnectionEvent::TimeoutTimer);
                spawn_local(async move { fut.await });
            })
        };

        let event_handler = EventHandler {
            connection: Connection::new(client.clone()),
            handler: event_handler,
        };
        client.set_event_handler(event_handler);
        client
            .connect(jid.to_string(), password.to_string())
            .await
            .map_err(|err| JSConnectionError::from(err))?;

        Ok(Box::new(Connection {
            client,
            ping_interval: RefCell::new(Some(ping_interval)),
            timeout_interval: RefCell::new(Some(timeout_interval)),
        }))
    }
}

pub struct Connection {
    client: Rc<JSConnection>,
    ping_interval: RefCell<Option<Interval>>,
    timeout_interval: RefCell<Option<Interval>>,
}

impl Connection {
    fn new(client: Rc<JSConnection>) -> Self {
        Connection {
            client,
            ping_interval: Default::default(),
            timeout_interval: Default::default(),
        }
    }
}

impl ConnectionTrait for Connection {
    fn send_stanza(&self, stanza: Element) -> Result<()> {
        self.client
            .send_stanza(String::from(&stanza))
            .map_err(|err| JSConnectionError::from(err))?;
        Ok(())
    }

    fn disconnect(&self) {
        self.ping_interval.replace(None);
        self.timeout_interval.replace(None);
        self.client.disconnect()
    }
}

#[wasm_bindgen(js_class = "ProseConnectionEventHandler")]
impl EventHandler {
    #[wasm_bindgen(js_name = "handleDisconnect")]
    pub fn handle_disconnect(&self, error: Option<String>) {
        let fut = (self.handler)(
            &self.connection,
            ConnectionEvent::Disconnected {
                error: error.map(|error| ConnectionError::Generic { msg: error }),
            },
        );
        spawn_local(async move { fut.await })
    }

    #[wasm_bindgen(js_name = "handleStanza")]
    pub fn handle_stanza(&self, stanza: String) {
        let fut = (self.handler)(
            &self.connection,
            ConnectionEvent::Stanza(
                Element::from_str(&stanza).expect("Failed to parse received stanza"),
            ),
        );
        spawn_local(async move { fut.await })
    }
}

#[derive(Error, Debug)]
pub enum JSConnectionError {
    #[error("DomException {name} ({code}): {message}")]
    DomException {
        code: u16,
        name: String,
        message: String,
    },
}

impl From<JSConnectionError> for ConnectionError {
    fn from(value: JSConnectionError) -> Self {
        ConnectionError::Generic {
            msg: value.to_string(),
        }
    }
}

impl From<DomException> for JSConnectionError {
    fn from(value: DomException) -> Self {
        JSConnectionError::DomException {
            code: value.code(),
            name: value.name(),
            message: value.message(),
        }
    }
}
