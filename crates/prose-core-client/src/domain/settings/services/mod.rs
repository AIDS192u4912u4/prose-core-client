// prose-core-client/prose-core-client
//
// Copyright: 2024, Marc Bauer <mb@nesium.com>
// License: Mozilla Public License v2.0 (MPL v2.0)

pub use synced_room_settings_service::SyncedRoomSettingsService;

mod synced_room_settings_service;

#[cfg(feature = "test")]
pub mod mocks {
    pub use super::synced_room_settings_service::MockSyncedRoomSettingsService;
}
