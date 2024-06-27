// prose-core-client/prose-core-client
//
// Copyright: 2023, Marc Bauer <mb@nesium.com>
// License: Mozilla Public License v2.0 (MPL v2.0)

use anyhow::bail;
use xmpp_parsers::muc::user::Status;
use xmpp_parsers::presence::Presence;

use prose_xmpp::ns;
use prose_xmpp::stanza::muc::MucUser;

use crate::domain::rooms::models::RoomSessionParticipant;
use crate::domain::user_info::models::{Avatar, AvatarSource};
use crate::dtos::{OccupantId, ParticipantId, UserId};
use crate::infra::xmpp::util::PresenceExt;

impl TryFrom<Presence> for RoomSessionParticipant {
    type Error = anyhow::Error;

    fn try_from(mut value: Presence) -> Result<Self, Self::Error> {
        let anon_occupant_id = value.anon_occupant_id();
        let availability = value.availability();

        let Some(from) = value.from.take().and_then(|from| from.try_into_full().ok()) else {
            bail!("Expected FullJid in MUC presence.")
        };

        let Some(muc_user) = value
            .payloads
            .iter()
            .find(|p| p.is("x", ns::MUC_USER))
            .cloned()
        else {
            bail!("Missing 'x' element in MUC presence");
        };

        let muc_user = MucUser::try_from(muc_user)?;

        let Some(item) = muc_user.items.first() else {
            bail!("Missing 'item' element in MUC presence");
        };

        let occupant_id = OccupantId::from(from);
        let real_id = item.jid.clone().map(|jid| UserId::from(jid.into_bare()));

        let is_self = muc_user.status.contains(&Status::SelfPresence);

        let avatar = value.avatar_id().map(|avatar_id| Avatar {
            id: avatar_id,
            source: AvatarSource::Vcard,
            owner: ParticipantId::Occupant(occupant_id.clone()),
        });

        Ok(RoomSessionParticipant {
            id: occupant_id,
            is_self,
            anon_id: anon_occupant_id,
            real_id,
            affiliation: item.affiliation.clone().into(),
            availability,
            avatar,
        })
    }
}
