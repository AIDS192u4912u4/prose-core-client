// prose-core-client/prose-core-client
//
// Copyright: 2024, Marc Bauer <mb@nesium.com>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::collections::HashSet;
use std::time::SystemTime;

use aes_gcm::aead::Aead;
use aes_gcm::{AeadCore, Aes128Gcm, KeyInit};
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use futures::future::join_all;
use parking_lot::Mutex;
use rand::prelude::SliceRandom;
use tracing::{error, info, warn};

use prose_proc_macros::DependenciesStruct;
use prose_xmpp::TimeProvider;

use crate::app::deps::{
    DynAppContext, DynEncryptionKeysRepository, DynEncryptionService, DynMessagesRepository,
    DynMessagingService, DynRngProvider, DynSessionRepository, DynTimeProvider,
    DynUserDeviceIdProvider, DynUserDeviceRepository, DynUserDeviceService,
};
use crate::domain::encryption::models::{Device, DeviceId, DeviceInfo, DeviceList, PreKeyBundle};
use crate::domain::encryption::services::encryption_domain_service::{
    DecryptionError, EncryptionError,
};
use crate::domain::messaging::models::MessageLikePayload;
use crate::domain::messaging::models::{EncryptedPayload, KeyTransportPayload};
use crate::domain::shared::models::UserId;
use crate::dtos::{EncryptionKey, MessageId, PreKeyId, RoomId};

use super::super::EncryptionDomainService as EncryptionDomainServiceTrait;

#[derive(DependenciesStruct)]
pub struct EncryptionDomainService {
    ctx: DynAppContext,
    encryption_keys_repo: DynEncryptionKeysRepository,
    encryption_service: DynEncryptionService,
    message_repo: DynMessagesRepository,
    messaging_service: DynMessagingService,
    rng_provider: DynRngProvider,
    session_repo: DynSessionRepository,
    time_provider: DynTimeProvider,
    user_device_id_provider: DynUserDeviceIdProvider,
    user_device_repo: DynUserDeviceRepository,
    user_device_service: DynUserDeviceService,

    unpublish_device_attempts: Mutex<HashSet<DeviceId>>,
    repair_session_attempts: Mutex<HashSet<(UserId, DeviceId)>>,
}

const KEY_SIZE: usize = 16;
const MAC_SIZE: usize = 16;

#[cfg_attr(target_arch = "wasm32", async_trait(? Send))]
#[async_trait]
impl EncryptionDomainServiceTrait for EncryptionDomainService {
    /// Generates the local device bundle and publishes it if needed.
    async fn initialize(&self) -> Result<()> {
        self.unpublish_device_attempts.lock().clear();
        self.repair_session_attempts.lock().clear();

        // Initialize local bundle if needed…
        let bundle = match self
            .encryption_keys_repo
            .get_local_device_bundle()
            .await
            .context("Failed to load local device bundle.")?
        {
            Some(bundle) => bundle,
            None => {
                let local_encryption_bundle = self
                    .encryption_service
                    .generate_local_encryption_bundle(self.user_device_id_provider.new_id())
                    .await
                    .context("Failed to generate local encryption bundle.")?;

                self.encryption_keys_repo
                    .put_local_encryption_bundle(&local_encryption_bundle)
                    .await
                    .context("Failed to save local encryption bundle")?;

                local_encryption_bundle.into_device_bundle()
            }
        };

        let user_id = self.ctx.connected_id()?.into_user_id();

        let mut devices = self.user_device_repo.get_all(&user_id).await?;
        // Add our device to our device list if needed…
        if !devices
            .iter()
            .find(|device| device.id == bundle.device_id)
            .is_some()
        {
            info!(
                "Adding our device {} the list of devices…",
                bundle.device_id
            );
            devices.push(Device {
                id: bundle.device_id.clone(),
                label: Some(self.build_local_device_label()),
            });
            self.user_device_service
                .publish_device_list(DeviceList { devices })
                .await
                .context("Failed to publish our device list")?;
        }

        let published_bundle = self
            .user_device_service
            .load_device_bundle(&user_id, &bundle.device_id)
            .await
            .context("Failed to load our device bundle")?;

        // … and publish our device bundle…
        if published_bundle.is_none() {
            info!("Publishing our device bundle…");
            self.user_device_service
                .publish_device_bundle(bundle)
                .await
                .context("Failed to publish our device bundle")?;
        }

        Ok(())
    }

    async fn encrypt_message(
        &self,
        recipient_id: &UserId,
        message: String,
    ) -> Result<EncryptedPayload, EncryptionError> {
        let current_user_id = self.ctx.connected_id()?.into_user_id();

        let local_device = self
            .encryption_keys_repo
            .get_local_device()
            .await?
            .ok_or(anyhow!("Missing local encryption bundle"))?;

        match self
            .start_sessions_if_needed(
                &current_user_id,
                self.user_device_repo
                    .get_all(&current_user_id)
                    .await?
                    .into_iter()
                    .filter(|device| device.id != local_device.device_id),
            )
            .await
        {
            Ok(_) => (),
            Err(err) => {
                error!(
                    "Failed to start OMEMO session for our other devices. {}",
                    err.to_string()
                );
            }
        }

        match self
            .start_sessions_if_needed(
                recipient_id,
                self.user_device_repo.get_all(recipient_id).await?,
            )
            .await
        {
            Ok(_) => (),
            Err(err) => {
                error!(
                    "Failed to start OMEMO session with {recipient_id}. {}",
                    err.to_string()
                );
            }
        }

        let their_sessions = self
            .session_repo
            .get_all_sessions(&recipient_id)
            .await?
            .into_iter()
            .filter(|session| session.is_active)
            .collect::<Vec<_>>();

        if their_sessions.is_empty() {
            return Err(EncryptionError::NoDevices);
        }

        let their_active_device_ids = their_sessions
            .into_iter()
            .filter_map(|session| {
                session
                    .is_trusted_or_undecided()
                    .then_some((recipient_id, session.device_id))
            })
            .collect::<Vec<_>>();

        if their_active_device_ids.is_empty() {
            return Err(EncryptionError::NoDevices);
        }

        let nonce = Aes128Gcm::generate_nonce(self.rng_provider.rng());
        let dek = Aes128Gcm::generate_key(self.rng_provider.rng());
        let cipher = Aes128Gcm::new(&dek);

        let payload = cipher
            .encrypt(&nonce, message.as_bytes())
            .map_err(|err| anyhow!("{err}"))?;

        let mut dek_and_mac = [0u8; KEY_SIZE + MAC_SIZE];
        dek_and_mac[..KEY_SIZE].copy_from_slice(&dek);
        dek_and_mac[KEY_SIZE..KEY_SIZE + MAC_SIZE].copy_from_slice(&payload[message.len()..]);

        let now = SystemTime::from(self.time_provider.now());

        // Instead of encrypting the message for all the user's devices we'll only encrypt it
        // for devices which we have an active session with, i.e. devices that are actually trusted.
        // Otherwise, libsignal will choke later on.
        let our_active_device_ids = self
            .get_active_and_trusted_device_ids(&current_user_id)
            .await?;

        let encrypt_message_futures = our_active_device_ids
            .into_iter()
            .filter(|device_id| device_id != &local_device.device_id)
            .map(|device_id| (&current_user_id, device_id))
            .chain(their_active_device_ids)
            .map(|(user_id, device_id)| async move {
                self.encryption_service
                    .encrypt_key(user_id, &device_id, &dek_and_mac, &now)
                    .await
            });

        let messages = join_all(encrypt_message_futures)
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;

        let payload = EncryptedPayload {
            device_id: local_device.device_id,
            iv: nonce.as_slice().into(),
            keys: messages,
            payload: payload[..message.len()].into(),
        };

        Ok(payload)
    }

    async fn decrypt_message(
        &self,
        sender_id: &UserId,
        message_id: Option<&MessageId>,
        payload: EncryptedPayload,
    ) -> Result<String, DecryptionError> {
        // First try to decrypt the message. If that succeeds, great!
        let error = match self.decrypt_payload(sender_id, payload).await {
            Ok(message) => return Ok(message),
            Err(error) => error,
        };

        // If it failed we'll have a look at our message cache to see if
        // we may have decrypted the message in the past.
        let Some(message_id) = message_id else {
            return Err(error);
        };

        let Ok(messages) = self
            .message_repo
            .get(&RoomId::User(sender_id.clone()), message_id)
            .await
        else {
            return Err(error);
        };

        let Some(message) = messages.first() else {
            return Err(error);
        };

        let MessageLikePayload::Message { body, .. } = &message.payload else {
            return Err(error);
        };

        Ok(body.to_string())
    }

    async fn load_device_infos(&self, user_id: &UserId) -> Result<Vec<DeviceInfo>> {
        let this_device_id = if &self.ctx.connected_id()?.into_user_id() == user_id {
            self.encryption_keys_repo
                .get_local_device()
                .await?
                .map(|device| device.device_id)
        } else {
            None
        };

        let device_infos = self
            .session_repo
            .get_all_sessions(user_id)
            .await?
            .into_iter()
            .filter_map(|session| {
                let Some(identity) = session.identity else {
                    return None;
                };

                let is_this_device = Some(&session.device_id) == this_device_id.as_ref();

                let info = DeviceInfo {
                    id: session.device_id,
                    identity,
                    trust: session.trust,
                    is_active: session.is_active,
                    is_this_device,
                };
                Some(info)
            })
            .collect();

        Ok(device_infos)
    }

    async fn delete_device(&self, device_id: &DeviceId) -> Result<()> {
        let user_id = self.ctx.connected_id()?.into_user_id();

        let mut devices = self.user_device_repo.get_all(&user_id).await?;
        let num_devices = devices.len();
        devices.retain(|device| &device.id != device_id);

        if devices.len() == num_devices {
            return Ok(());
        }

        self.user_device_repo
            .set_all(&user_id, devices.clone())
            .await?;

        self.user_device_service
            .publish_device_list(DeviceList { devices })
            .await?;

        self.user_device_service
            .delete_device_bundle(device_id)
            .await?;

        Ok(())
    }

    async fn disable_omemo(&self) -> Result<()> {
        let devices = self
            .user_device_repo
            .get_all(&self.ctx.connected_id()?.into_user_id())
            .await?;

        self.user_device_service.delete_device_list().await?;

        for device in devices {
            _ = self
                .user_device_service
                .delete_device_bundle(&device.id)
                .await
        }

        Ok(())
    }

    async fn handle_received_key_transport_message(
        &self,
        sender_id: &UserId,
        payload: KeyTransportPayload,
    ) -> Result<()> {
        let local_device = self
            .encryption_keys_repo
            .get_local_device()
            .await?
            .ok_or(anyhow!("Missing local encryption bundle"))?;

        let key = payload.get_key(&local_device.device_id).ok_or(anyhow!(
            "KeyTransportMessage was not encrypted for current device."
        ))?;

        self.decrypt_key(&key, sender_id, &payload.device_id)
            .await?;

        if key.is_pre_key {
            self.did_receive_pre_key_message(&local_device.device_id, sender_id, &payload.device_id)
                .await
        }

        Ok(())
    }

    async fn handle_received_device_list(
        &self,
        user_id: &UserId,
        device_list: DeviceList,
    ) -> Result<()> {
        // Did we just receive our own PubSub node?
        if user_id != &self.ctx.connected_id()?.into_user_id() {
            self.user_device_repo
                .set_all(user_id, device_list.devices)
                .await?;
            return Ok(());
        }

        self.user_device_repo
            .set_all(user_id, device_list.devices.clone())
            .await?;

        let Some(current_device) = self.encryption_keys_repo.get_local_device().await? else {
            return Ok(());
        };

        // … This step presents the risk of introducing a race condition: Two devices might
        // simultaneously try to announce themselves, unaware of the other's existence. The second
        // device would overwrite the first one. To mitigate this, devices MUST check that their
        // own device id is contained in the list whenever they receive a PEP update from their own
        // account. If they have been removed, they MUST reannounce themselves.
        //
        // https://xmpp.org/extensions/xep-0384.html#devices

        if device_list
            .devices
            .iter()
            .find(|device| device.id == current_device.device_id)
            .is_some()
        {
            return Ok(());
        }

        let mut updated_device_list = device_list;
        updated_device_list.devices.push(Device {
            id: current_device.device_id,
            label: Some(self.build_local_device_label()),
        });

        self.user_device_service
            .publish_device_list(updated_device_list)
            .await
            .context("Failed to publish our updated device list")?;

        Ok(())
    }

    async fn clear_cache(&self) -> Result<()> {
        self.user_device_repo.clear_cache().await?;
        self.encryption_keys_repo.clear_cache().await?;
        self.session_repo.clear_cache().await?;
        Ok(())
    }
}

impl EncryptionDomainService {
    async fn generate_and_publish_missing_pre_keys(&self) -> Result<()> {
        let pre_keys = self
            .encryption_keys_repo
            .get_all_pre_keys()
            .await
            .context("Failed to load local PreKeys")?;

        // Collect existing PreKey ids…
        let pre_key_ids = pre_keys
            .iter()
            .map(|pre_key| pre_key.id.as_ref())
            .collect::<HashSet<_>>();
        // Check if any IDs between 1 and 100 are missing…
        let missing_pre_key_ids = (1..=100)
            .filter_map(|idx| {
                if pre_key_ids.contains(&idx) {
                    return None;
                }
                return Some(PreKeyId::from(idx));
            })
            .collect::<Vec<_>>();

        // No missing IDs, nothing to do…
        if missing_pre_key_ids.is_empty() {
            return Ok(());
        }

        info!("Generating {} new PreKeys…", missing_pre_key_ids.len());
        let missing_pre_keys = self
            .encryption_service
            .generate_pre_keys_with_ids(missing_pre_key_ids)
            .await
            .context("Failed to re-generate deleted PreKeys")?;

        info!("Saving new PreKeys…");
        self.encryption_keys_repo
            .put_pre_keys(missing_pre_keys.as_slice())
            .await
            .context("Failed to save re-generated PreKeys…")?;

        info!("Publishing bundle with new PreKeys…");
        let mut bundle = self
            .encryption_keys_repo
            .get_local_device_bundle()
            .await?
            .ok_or(anyhow!("Missing own device bundle"))?;
        bundle.pre_keys.sort_by_key(|key| key.id);

        self.user_device_service
            .publish_device_bundle(bundle)
            .await
            .context("Failed to publish device bundle with re-generated PreKeys")?;

        Ok(())
    }

    async fn decrypt_key(
        &self,
        key: &EncryptionKey,
        sender_id: &UserId,
        sender_device_id: &DeviceId,
    ) -> Result<Box<[u8]>> {
        let dek_and_mac = self
            .encryption_service
            .decrypt_key(
                sender_id,
                &sender_device_id,
                &key.data.as_ref(),
                key.is_pre_key,
            )
            .await?;

        if dek_and_mac.len() != MAC_SIZE + KEY_SIZE {
            bail!("Invalid DEK and MAC size");
        }

        Ok(dek_and_mac)
    }

    async fn decrypt_payload(
        &self,
        sender_id: &UserId,
        payload: EncryptedPayload,
    ) -> Result<String, DecryptionError> {
        let local_device = self
            .encryption_keys_repo
            .get_local_device()
            .await?
            .ok_or(anyhow!("Missing local encryption bundle"))?;

        let key = payload
            .get_key(&local_device.device_id)
            .ok_or(DecryptionError::NotEncryptedForThisDevice)?;

        let dek_and_mac = match self.decrypt_key(&key, sender_id, &payload.device_id).await {
            Ok(data) => data,
            Err(err) => {
                // While we would usually only try to repair a session for certain error types,
                // i.e. InvalidMessageException and NoSessionException, there's no way to get
                // a typed error out of the outdated libsignal-protocol-javascript. This is
                // something to improve after we switch to WASI and can share the native libsignal
                // library between web and native.
                if self
                    .repair_session_attempts
                    .lock()
                    .insert((sender_id.clone(), payload.device_id.clone()))
                {
                    _ = self
                        .start_session_with_device(sender_id, payload.device_id)
                        .await;
                }
                return Err(err.into());
            }
        };

        let dek = aes_gcm::Key::<Aes128Gcm>::from_slice(&dek_and_mac[..KEY_SIZE]);
        let mac = &dek_and_mac[KEY_SIZE..KEY_SIZE + MAC_SIZE];
        let mut payload_and_mac = Vec::with_capacity(payload.payload.len() + mac.len());
        payload_and_mac.extend_from_slice(payload.payload.as_ref());
        payload_and_mac.extend(mac);

        let cipher = Aes128Gcm::new(&dek);
        let nonce =
            aes_gcm::Nonce::<<Aes128Gcm as AeadCore>::NonceSize>::from_slice(payload.iv.as_ref());
        let message = String::from_utf8(
            cipher
                .decrypt(nonce, payload_and_mac.as_slice())
                .map_err(|err| anyhow!("{err}"))?,
        )
        .map_err(|err| anyhow!(err))?;

        if key.is_pre_key {
            self.did_receive_pre_key_message(&local_device.device_id, sender_id, &payload.device_id)
                .await
        }

        Ok(message)
    }

    async fn did_receive_pre_key_message(
        &self,
        local_device_id: &DeviceId,
        sender_id: &UserId,
        sender_device_id: &DeviceId,
    ) {
        if let Err(err) = self.generate_and_publish_missing_pre_keys().await {
            error!("Failed to generate missing prekeys. {}", err.to_string())
        }

        if let Err(err) = self
            .complete_session(&local_device_id, sender_id, sender_device_id)
            .await
        {
            error!(
                "Failed to complete session with {sender_id}. {}",
                err.to_string()
            )
        }
    }

    async fn complete_session(
        &self,
        local_device_id: &DeviceId,
        sender_id: &UserId,
        sender_device_id: &DeviceId,
    ) -> Result<()> {
        let nonce = Aes128Gcm::generate_nonce(self.rng_provider.rng());
        let dek = Aes128Gcm::generate_key(self.rng_provider.rng());

        let mut dek_and_mac = [0u8; KEY_SIZE + MAC_SIZE];
        dek_and_mac[..KEY_SIZE].copy_from_slice(&dek);

        let encrypted_key = self
            .encryption_service
            .encrypt_key(
                sender_id,
                sender_device_id,
                &dek_and_mac,
                &SystemTime::from(self.time_provider.now()),
            )
            .await?;

        self.messaging_service
            .send_key_transport_message(
                sender_id,
                KeyTransportPayload {
                    device_id: local_device_id.clone(),
                    iv: nonce.as_slice().into(),
                    keys: vec![encrypted_key],
                },
            )
            .await?;

        Ok(())
    }

    fn build_local_device_label(&self) -> String {
        self.ctx
            .software_version
            .os
            .as_ref()
            .map(|os| format!("{} ({})", self.ctx.software_version.name, os))
            .unwrap_or(self.ctx.software_version.name.clone())
    }

    async fn start_sessions_if_needed(
        &self,
        user_id: &UserId,
        devices: impl IntoIterator<Item = Device>,
    ) -> Result<()> {
        let device_ids = devices
            .into_iter()
            .map(|device| device.id)
            .collect::<Vec<_>>();

        self.session_repo
            .put_active_devices(user_id, device_ids.as_slice())
            .await?;

        join_all(device_ids.into_iter().map(|device_id| async move {
            if self
                .session_repo
                .get_session(user_id, &device_id)
                .await?
                .is_some()
            {
                return Ok(());
            }

            self.start_session_with_device(user_id, device_id.clone())
                .await
                .with_context(|| format!("Failed to start session with {user_id} ({})", device_id))
        }))
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;

        Ok(())
    }

    async fn start_session_with_device(&self, user_id: &UserId, device_id: DeviceId) -> Result<()> {
        info!("Starting OMEMO session with {user_id} ({device_id})…");

        let Some(bundle) = self
            .user_device_service
            .load_device_bundle(&user_id, &device_id)
            .await
            .with_context(|| format!("Failed to load device bundle for {user_id} ({device_id})"))?
        else {
            info!("No device bundle found for {user_id} ({device_id}).");

            if user_id == &self.ctx.connected_id()?.into_user_id()
                && self
                    .unpublish_device_attempts
                    .lock()
                    .insert(device_id.clone())
            {
                _ = self.unpublish_device(&device_id).await
            }

            return Ok(());
        };

        let pre_key_bundle = PreKeyBundle {
            device_id: device_id.clone(),
            signed_pre_key: bundle.signed_pre_key,
            identity_key: bundle.identity_key,
            pre_key: bundle
                .pre_keys
                .choose(&mut self.rng_provider.rng())
                .ok_or(anyhow!("No pre_keys available."))?
                .clone(),
        };

        match self
            .encryption_service
            .process_pre_key_bundle(&user_id, pre_key_bundle)
            .await
            .with_context(|| format!("Failed to process PreKey bundle for {user_id} ({device_id})"))
        {
            Ok(_) => (),
            Err(err) => {
                if user_id == &self.ctx.connected_id()?.into_user_id()
                    && self
                        .unpublish_device_attempts
                        .lock()
                        .insert(device_id.clone())
                {
                    _ = self.unpublish_device(&device_id).await
                }
                return Err(err);
            }
        }

        Ok(())
    }

    async fn get_active_and_trusted_device_ids(&self, user_id: &UserId) -> Result<Vec<DeviceId>> {
        Ok(self
            .session_repo
            .get_all_sessions(user_id)
            .await?
            .into_iter()
            .filter_map(|session| {
                (session.is_active && session.is_trusted_or_undecided())
                    .then_some(session.device_id)
            })
            .collect())
    }

    async fn unpublish_device(&self, device_id: &DeviceId) -> Result<()> {
        let mut devices = self
            .user_device_repo
            .get_all(&self.ctx.connected_id()?.into_user_id())
            .await?;
        let num_devices = devices.len();

        devices.retain(|device| &device.id != device_id);

        if devices.len() == num_devices {
            warn!("Could not find device {device_id} for removal.");
            return Ok(());
        }

        info!("Removing device {device_id} from our list of devices…");
        self.user_device_service
            .publish_device_list(DeviceList { devices })
            .await
            .context("Failed to publish our device list")?;

        Ok(())
    }
}
