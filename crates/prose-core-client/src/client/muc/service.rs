use crate::client::muc::room_config::RoomConfig;
use anyhow::Result;
use jid::BareJid;
use prose_xmpp::mods::muc::RoomConfigResponse;
use prose_xmpp::mods::{muc, MUC};
use prose_xmpp::Client as XMPPClient;

pub struct Service {
    pub jid: BareJid,
    pub user_jid: BareJid,
    pub(in crate::client) client: XMPPClient,
}

impl Service {
    pub async fn load_public_rooms(&self) -> Result<Vec<muc::Room>> {
        let muc = self.client.get_mod::<MUC>();
        muc.load_public_rooms(&self.jid).await
    }

    pub async fn create_public_channel(&self, channel_name: impl AsRef<str>) -> Result<()> {
        let muc = self.client.get_mod::<MUC>();
        let room_name = channel_name.as_ref().to_string();
        let nickname = self.user_jid.node_str().unwrap_or("unknown");

        muc.create_reserved_room(&self.jid, room_name.clone(), nickname, |form| async move {
            Ok(RoomConfigResponse::Submit(
                RoomConfig::public_channel(room_name).populate_form(&form)?,
            ))
        })
        .await
    }

    // pub async fn create_group_chat(&self) -> Result<()> {
    //     let muc = self.client.get_mod::<MUC>();
    //
    //     muc.create_reserved_room(&self.jid, "new_room", |form| async move {
    //         Ok(RoomConfigResponse::Submit(
    //             RoomConfig::group_chat().populate_form(&form)?,
    //         ))
    //     })
    //     .await
    // }
}
