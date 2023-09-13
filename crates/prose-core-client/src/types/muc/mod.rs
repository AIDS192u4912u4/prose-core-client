// prose-core-client/prose-xmpp
//
// Copyright: 2023, Marc Bauer <mb@nesium.com>
// License: Mozilla Public License v2.0 (MPL v2.0)

pub(crate) use bookmark_metadata::{BookmarkMetadata, RoomType};
pub(crate) use room::Room;
pub(crate) use room_config::RoomConfig;
pub(crate) use room_info::RoomInfo;
pub(crate) use service::{CreateRoomResult, Service};

mod bookmark_metadata;
mod room;
mod room_config;
mod room_info;
mod rooms;
mod service;
