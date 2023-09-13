// prose-core-client/prose-core-client
//
// Copyright: 2023, Marc Bauer <mb@nesium.com>
// License: Mozilla Public License v2.0 (MPL v2.0)

use jid::BareJid;
use prose_xmpp::Client as XMPPClient;
use xmpp_parsers::muc;

#[derive(Debug)]
pub(super) struct AbstractRoom {
    pub jid: BareJid,
    pub name: Option<String>,
    pub description: Option<String>,
    pub client: XMPPClient,
    pub occupants: Vec<Occupant>,
}

#[derive(Debug)]
pub(super) struct Occupant {
    pub affiliation: muc::user::Affiliation,
    pub occupant_id: Option<String>,
}
