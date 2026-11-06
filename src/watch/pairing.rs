//! Paired watches (`Pairing.swift`): each has its own bearer token and a
//! 32-byte E2EE key. A pairing Wristper made without a key is listed but
//! never served; it must pair again. The names and times live in Wristper's `paired-devices.json`; the token and key live
//! only in memory and in a `SecretStore` (the login Keychain in the app, a
//! map in tests). Typing the pairing code on the Mac is the approval: the
//! watch the relay names in `claim_ok` may send one pairing request within
//! 120 s, and its result stays readable for 300 s so a lost reply never
//! forces a second pairing.

mod devices_file;
#[cfg(target_os = "macos")]
pub(crate) mod keychain;
pub(crate) mod requests;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use subtle::ConstantTimeEq;

use super::e2ee::SharedKey;
use super::{PeerInfo, Timestamp};

/// How long the typed code admits the claimed watch.
pub(crate) const APPROVAL_WINDOW: Duration = Duration::from_secs(120);
/// Unreadable secrets are retried at most this often
/// (`PairedDevices.retryInterval`).
pub(crate) const RETRY_INTERVAL: Duration = Duration::from_secs(30);
/// How long a pairing result stays readable (`PairingKey.requestLifetime`).
pub(crate) const REQUEST_LIFETIME: Duration = Duration::from_secs(300);
/// `last_seen` is rewritten at most this often.
pub(crate) const TOUCH_INTERVAL: Duration = Duration::from_secs(60);
pub(crate) const FILE_NAME: &str = "paired-devices.json";

pub(crate) const ENCRYPTION_FAILED: &str = "encryption_failed";
/// Keychain locked or refusing: shown on the Mac; the watch still reads
/// `encryption_failed`.
pub(crate) const KEYCHAIN_UNAVAILABLE: &str = "keychain_unavailable";

pub(crate) fn watch_facing_failure(failure: &str) -> &str {
    if failure == KEYCHAIN_UNAVAILABLE {
        ENCRYPTION_FAILED
    } else {
        failure
    }
}

/// Python's `secrets.token_urlsafe(32)`: 32 random bytes, base64url, no
/// padding.
pub(crate) fn new_token() -> Option<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).ok()?;
    Some(URL_SAFE_NO_PAD.encode(bytes))
}

/// Constant-time; two empty strings are not equal, so a watch without a
/// token never matches a device without one.
pub(crate) fn tokens_equal(presented: &str, expected: &str) -> bool {
    !presented.is_empty()
        && presented.len() == expected.len()
        && bool::from(presented.as_bytes().ct_eq(expected.as_bytes()))
}

/// A device's secrets as the store keeps them (`DeviceSecret`).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DeviceSecret {
    pub(crate) token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) e2ee_key: Option<String>,
}

impl fmt::Debug for DeviceSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceSecret")
            .field("e2ee", &self.e2ee_key.is_some())
            .finish_non_exhaustive()
    }
}

/// Where device secrets live. `Err` means "could not tell right now" (a
/// locked Keychain): callers answer 503 and retry, never 403.
pub(crate) trait SecretStore {
    fn load(&self, device_id: &str) -> Result<Option<DeviceSecret>, String>;
    fn save(&self, device_id: &str, secret: &DeviceSecret) -> Result<(), String>;
    /// A missing entry is not an error.
    fn delete(&self, device_id: &str) -> Result<(), String>;
    /// Whether this Mac already moved the secrets out of
    /// `paired-devices.json`. Kept in the store, not the file, because any
    /// program of the same user can edit the file. No marker is `false`.
    fn migration_completed(&self) -> Result<bool, String>;
    fn mark_migration_completed(&self) -> Result<(), String>;
}

/// The in-memory store for tests and `--test-mode`.
#[derive(Default)]
pub(crate) struct MemorySecretStore {
    entries: std::cell::RefCell<BTreeMap<String, DeviceSecret>>,
    migrated: std::cell::Cell<bool>,
    /// A locked file Keychain: an item that does not exist reads as
    /// missing (`errSecItemNotFound`), and only an existing item's data, a
    /// write, or deleting an existing item fails.
    pub(crate) locked: std::cell::Cell<bool>,
    /// Every call fails: a Keychain that answers nothing at all.
    pub(crate) unreachable: std::cell::Cell<bool>,
    /// Items another app owns: reading or writing them fails, as a Keychain
    /// item whose access list excludes Hex.
    pub(crate) foreign: std::cell::RefCell<BTreeSet<String>>,
    /// Items whose deletion is refused.
    pub(crate) undeletable: std::cell::RefCell<BTreeSet<String>>,
}

impl MemorySecretStore {
    fn exists(&self, device_id: &str) -> bool {
        self.entries.borrow().contains_key(device_id) || self.foreign.borrow().contains(device_id)
    }

    fn reachable(&self) -> Result<(), String> {
        if self.unreachable.get() {
            return Err("keychain unreachable".to_owned());
        }
        Ok(())
    }

    fn refuse(&self, device_id: &str) -> Result<(), String> {
        if self.locked.get() {
            return Err("keychain locked".to_owned());
        }
        if self.foreign.borrow().contains(device_id) {
            return Err(format!("{device_id} is not ours"));
        }
        Ok(())
    }
}

impl SecretStore for MemorySecretStore {
    fn load(&self, device_id: &str) -> Result<Option<DeviceSecret>, String> {
        self.reachable()?;
        if !self.exists(device_id) {
            return Ok(None);
        }
        self.refuse(device_id)?;
        Ok(self.entries.borrow().get(device_id).cloned())
    }

    fn save(&self, device_id: &str, secret: &DeviceSecret) -> Result<(), String> {
        self.reachable()?;
        self.refuse(device_id)?;
        self.entries
            .borrow_mut()
            .insert(device_id.to_owned(), secret.clone());
        Ok(())
    }

    fn delete(&self, device_id: &str) -> Result<(), String> {
        self.reachable()?;
        if !self.exists(device_id) {
            return Ok(());
        }
        if self.locked.get() || self.undeletable.borrow().contains(device_id) {
            return Err(format!("deleting {device_id} refused"));
        }
        self.foreign.borrow_mut().remove(device_id);
        self.entries.borrow_mut().remove(device_id);
        Ok(())
    }

    fn migration_completed(&self) -> Result<bool, String> {
        self.reachable()?;
        match (self.migrated.get(), self.locked.get()) {
            (false, _) => Ok(false),
            (true, true) => Err("keychain locked".to_owned()),
            (true, false) => Ok(true),
        }
    }

    fn mark_migration_completed(&self) -> Result<(), String> {
        self.reachable()?;
        if self.locked.get() {
            return Err("keychain locked".to_owned());
        }
        self.migrated.set(true);
        Ok(())
    }
}

/// One store shared by a `PairedDevices` and the test that checks it.
#[cfg(test)]
impl SecretStore for std::rc::Rc<MemorySecretStore> {
    fn load(&self, device_id: &str) -> Result<Option<DeviceSecret>, String> {
        (**self).load(device_id)
    }
    fn save(&self, device_id: &str, secret: &DeviceSecret) -> Result<(), String> {
        (**self).save(device_id, secret)
    }
    fn delete(&self, device_id: &str) -> Result<(), String> {
        (**self).delete(device_id)
    }
    fn migration_completed(&self) -> Result<bool, String> {
        (**self).migration_completed()
    }
    fn mark_migration_completed(&self) -> Result<(), String> {
        (**self).mark_migration_completed()
    }
}

/// One entry of `paired-devices.json`. Unknown keys (Wristper's `push`
/// destination, anything newer) round-trip untouched.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct PairedDevice {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) paired_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_paired_at: Option<String>,
    #[serde(default)]
    pub(crate) last_seen: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) requires_e2ee: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_protocol: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_app_version: Option<String>,
    #[serde(flatten)]
    pub(crate) extra: Map<String, Value>,
}

/// Whether a paired watch can be served.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeviceStatus {
    /// It has a token and a valid E2EE key.
    Ready,
    /// The store has refused its secret for less than `RETRY_INTERVAL`.
    SecretsUnavailable,
    /// Pair it again from the Mac: it has no key (a pairing from before
    /// encryption), no secret at all, or a secret the store has refused for
    /// longer than `RETRY_INTERVAL`. Only the last kind is a guess: its
    /// requests keep answering 503 and recover by themselves once the
    /// store reads it, because a locked Keychain looks the same.
    NeedsRepair,
}

/// What Settings shows about a watch; never its secrets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeviceSummary {
    pub(crate) id: String,
    pub(crate) name: String,
    /// When it last paired (a watch paired again keeps its first
    /// `paired_at`), so the window tells a new pairing from the old one.
    pub(crate) last_paired_at: String,
    pub(crate) last_seen: String,
    pub(crate) last_protocol: u32,
    pub(crate) last_app_version: String,
    pub(crate) status: DeviceStatus,
}

/// The paired watches and their secrets. One owner at a time; the caller
/// serializes access (a mutex in the service).
pub(crate) struct PairedDevices {
    path: PathBuf,
    devices: Vec<PairedDevice>,
    secrets: BTreeMap<String, DeviceSecret>,
    store: Box<dyn SecretStore>,
    /// Devices whose secret stays in the file: the import could not put it
    /// in the store. Served from memory and written back on every save
    /// until a later launch imports it.
    in_file: BTreeSet<String>,
    /// The file's secrets while the migration marker is unreadable: neither
    /// imported nor dropped, and written back unchanged on every save.
    pending_file_secrets: Option<BTreeMap<String, DeviceSecret>>,
    /// Devices whose secret the store refused, and since when. While any
    /// is here, a token that matches nothing answers 503, never 403
    /// (`secretsUnavailable` in Swift).
    unreadable: BTreeMap<String, Instant>,
    last_retry: Instant,
    clock: Box<dyn Fn() -> Instant>,
    /// Entries of the file Hex could not parse, written back unchanged.
    unparsed: Vec<Value>,
    /// Why the file is not a device list Hex can rewrite; while set, every
    /// save fails rather than replace it.
    unwritable: Option<String>,
    /// What happened while loading, for the process log.
    pub(crate) notes: Vec<String>,
}

impl PairedDevices {
    pub(crate) fn load(path: &Path, store: Box<dyn SecretStore>) -> Self {
        Self::load_with_clock(path, store, Box::new(Instant::now))
    }

    /// Reads the file and each device's secret. Secrets Wristper left in
    /// the file are imported once per Mac (`reconcileSecrets`): written to
    /// the store and read back before the file loses them, so an
    /// interruption at any step converges on the next launch.
    pub(crate) fn load_with_clock(
        path: &Path,
        store: Box<dyn SecretStore>,
        clock: Box<dyn Fn() -> Instant>,
    ) -> Self {
        let mut this = Self {
            path: path.to_path_buf(),
            devices: Vec::new(),
            secrets: BTreeMap::new(),
            store,
            in_file: BTreeSet::new(),
            pending_file_secrets: None,
            unreadable: BTreeMap::new(),
            last_retry: clock(),
            clock,
            unparsed: Vec::new(),
            unwritable: None,
            notes: Vec::new(),
        };
        match devices_file::read(path) {
            Ok(file) => {
                if !file.unparsed.is_empty() {
                    this.note(format!(
                        "{} entries of {FILE_NAME} unreadable, kept as they are",
                        file.unparsed.len()
                    ));
                }
                this.devices = file.devices;
                this.unparsed = file.unparsed;
            }
            Err(error) => {
                this.note(format!("{FILE_NAME} left untouched: {error}"));
                this.unwritable = Some(error);
            }
        }
        let mut file_secrets = BTreeMap::new();
        let mut had_fields = BTreeSet::new();
        for device in &mut this.devices {
            // Wristper 0.1 wrote `sas_verified`; it is dropped on read.
            device.extra.remove("sas_verified");
            let token = device.extra.remove("token");
            let key = device.extra.remove("e2ee_key");
            if token.is_some() || key.is_some() {
                had_fields.insert(device.id.clone());
            }
            let text = |value: Option<Value>| {
                value
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .filter(|text| !text.is_empty())
            };
            if let Some(token) = text(token)
                && !device.id.is_empty()
            {
                file_secrets.insert(
                    device.id.clone(),
                    DeviceSecret {
                        token,
                        e2ee_key: text(key),
                    },
                );
            }
        }
        if !this.settle_migration(&file_secrets) {
            let now = (this.clock)();
            for id in file_secrets.keys() {
                this.mark_unreadable(id, now);
            }
            this.pending_file_secrets = Some(file_secrets);
        }
        this.load_all();
        if this.pending_file_secrets.is_none() && !had_fields.is_subset(&this.in_file) {
            let _ = this.save();
        }
        this
    }

    fn note(&mut self, message: String) {
        self.notes.push(message);
    }

    /// `false` while the migration marker cannot be read; retried later.
    fn settle_migration(&mut self, file_secrets: &BTreeMap<String, DeviceSecret>) -> bool {
        let completed = match self.store.migration_completed() {
            Ok(completed) => completed,
            Err(error) => {
                self.note(format!(
                    "migration marker unreadable, the file's secrets stay as they are: {error}"
                ));
                return false;
            }
        };
        if completed {
            if !file_secrets.is_empty() {
                let ids: Vec<&str> = file_secrets.keys().map(String::as_str).collect();
                self.note(format!(
                    "migration already done; ignoring and removing the file's secrets for {}",
                    ids.join(", ")
                ));
            }
            return true;
        }
        for (id, secret) in file_secrets {
            match self.store.load(id) {
                Ok(Some(existing)) => {
                    if existing != *secret {
                        self.note(format!("the store's secret for {id} wins over the file's"));
                    }
                    continue;
                }
                Ok(None) => {}
                Err(error) => {
                    self.keep_in_file(id, secret, &error);
                    continue;
                }
            }
            if let Err(error) = self.write_to_store(id, secret) {
                self.keep_in_file(id, secret, &error);
            }
        }
        if self.in_file.is_empty()
            && let Err(error) = self.store.mark_migration_completed()
        {
            self.note(format!(
                "could not mark the migration done, next launch retries: {error}"
            ));
        }
        true
    }

    fn keep_in_file(&mut self, id: &str, secret: &DeviceSecret, reason: &str) {
        if self.position(id).is_none() {
            return;
        }
        self.secrets.insert(id.to_owned(), secret.clone());
        self.in_file.insert(id.to_owned());
        self.note(format!("{id}'s secret stays in the file for now: {reason}"));
    }

    /// Writes and reads back the same value.
    fn write_to_store(&self, id: &str, secret: &DeviceSecret) -> Result<(), String> {
        self.store.save(id, secret)?;
        match self.store.load(id)? {
            Some(stored) if stored == *secret => Ok(()),
            _ => Err("the secret did not read back".to_owned()),
        }
    }

    fn load_all(&mut self) {
        let ids: Vec<String> = self
            .devices
            .iter()
            .map(|device| device.id.clone())
            .filter(|id| {
                !id.is_empty()
                    && !self.in_file.contains(id)
                    && !self
                        .pending_file_secrets
                        .as_ref()
                        .is_some_and(|pending| pending.contains_key(id))
            })
            .collect();
        for id in ids {
            self.load_from_store(&id);
        }
    }

    fn load_from_store(&mut self, id: &str) {
        match self.store.load(id) {
            Ok(Some(secret)) => {
                self.secrets.insert(id.to_owned(), secret);
                self.unreadable.remove(id);
            }
            Ok(None) => {
                self.unreadable.remove(id);
                self.note(format!("no secret for {id}; pair that watch again"));
            }
            Err(error) => {
                let now = (self.clock)();
                if self.mark_unreadable(id, now) {
                    self.note(format!("secret for {id} unreadable, will retry: {error}"));
                }
            }
        }
    }

    fn mark_unreadable(&mut self, id: &str, now: Instant) -> bool {
        if self.unreadable.contains_key(id) {
            return false;
        }
        self.unreadable.insert(id.to_owned(), now);
        true
    }

    /// At most once per `RETRY_INTERVAL` (`retryUnreadableLocked`).
    fn retry_unreadable(&mut self) {
        let now = (self.clock)();
        if self.unreadable.is_empty() || now.duration_since(self.last_retry) < RETRY_INTERVAL {
            return;
        }
        self.last_retry = now;
        if let Some(pending) = self.pending_file_secrets.clone() {
            if !self.settle_migration(&pending) {
                return;
            }
            self.pending_file_secrets = None;
            for id in pending.keys() {
                self.unreadable.remove(id);
            }
            self.load_all();
            let _ = self.save();
        }
        for id in self.unreadable.keys().cloned().collect::<Vec<_>>() {
            if self.position(&id).is_some() {
                self.load_from_store(&id);
            } else {
                self.unreadable.remove(&id);
            }
        }
    }

    /// A token that matches no device is not proof of "unpaired" while
    /// any secret is unreadable: the service answers 503 then.
    pub(crate) fn secrets_unavailable(&mut self) -> bool {
        self.retry_unreadable();
        !self.unreadable.is_empty()
    }

    pub(crate) fn secrets_unavailable_for(&mut self, id: &str) -> bool {
        self.retry_unreadable();
        self.unreadable.contains_key(id)
    }

    fn position(&self, id: &str) -> Option<usize> {
        self.devices.iter().position(|device| device.id == id)
    }

    /// Issues a new token and key for `id`, new or re-paired. Typing the
    /// code on the Mac is the consent to replace whatever the store holds
    /// for `id`, even an item Hex cannot read: it is deleted and added
    /// again. `None` means this pairing is void: the store refused the
    /// delete or the new secret, or the list of a new watch could not be
    /// written.
    pub(crate) fn enroll(
        &mut self,
        id: &str,
        name: &str,
        e2ee_key: &SharedKey,
        now: SystemTime,
    ) -> Option<String> {
        let token = new_token()?;
        let secret = DeviceSecret {
            token: token.clone(),
            e2ee_key: Some(e2ee_key.to_base64()),
        };
        let is_new = self.position(id).is_none();
        // Delete, then add: an update could overwrite an item Hex may not
        // read back. A refused delete leaves the old item as it was.
        if let Err(error) = self.store.delete(id) {
            self.note(format!(
                "could not replace {id}'s secret, pairing void, old one kept: {error}"
            ));
            return None;
        }
        // The old item is gone: no copy of its secret may outlive it, or a
        // failure below would leave the old watch authenticating.
        self.forget_secret(id);
        if let Err(error) = self.write_to_store(id, &secret) {
            let _ = self.store.delete(id);
            if !is_new {
                let _ = self.save();
            }
            self.note(format!(
                "could not store {id}'s new secret, pairing void; pair it again: {error}"
            ));
            return None;
        }
        let stamp = Timestamp::string(now);
        match self.position(id) {
            Some(index) => {
                let device = &mut self.devices[index];
                // A new key is a new pairing: the ratchet belonged to the
                // old one, and so did any push destination.
                device.requires_e2ee = None;
                device.extra.remove("push");
                device.last_paired_at = Some(stamp);
                if !name.is_empty() {
                    device.name = name.to_owned();
                }
            }
            None => self.devices.push(PairedDevice {
                id: id.to_owned(),
                name: if name.is_empty() { id } else { name }.to_owned(),
                paired_at: stamp.clone(),
                last_paired_at: Some(stamp),
                last_seen: String::new(),
                requires_e2ee: None,
                last_protocol: None,
                last_app_version: None,
                extra: Map::new(),
            }),
        }
        self.secrets.insert(id.to_owned(), secret);
        if self.save().is_err() && is_new {
            self.devices.retain(|device| device.id != id);
            self.secrets.remove(id);
            let _ = self.store.delete(id);
            self.note(format!(
                "could not write the paired list, pairing of {id} void"
            ));
            return None;
        }
        Some(token)
    }

    /// Writes the list without `id` first; only once it is written does
    /// Hex forget the watch in memory and delete the store's item. A list
    /// Hex could not write changes nothing: the watch stays listed and
    /// served, its item stays, and `Err` says why, so Remove can try again.
    /// An item left behind once the list is written is harmless, since only
    /// listed devices load secrets.
    pub(crate) fn revoke(&mut self, id: &str) -> Result<bool, String> {
        let index = self.position(id);
        if let Some(index) = index {
            let device = self.devices.remove(index);
            if let Err(error) = self.save() {
                self.devices.insert(index, device);
                return Err(format!("could not write {FILE_NAME}: {error}"));
            }
            self.forget_secret(id);
        }
        if let Err(error) = self.store.delete(id) {
            self.note(format!(
                "revoked {id} but its secret stayed in the keychain: {error}"
            ));
        }
        Ok(index.is_some())
    }

    /// Drops every copy of `id`'s secret Hex holds outside the store.
    fn forget_secret(&mut self, id: &str) {
        self.secrets.remove(id);
        self.unreadable.remove(id);
        self.in_file.remove(id);
        if let Some(pending) = &mut self.pending_file_secrets {
            pending.remove(id);
        }
    }

    /// The token and key of a watch that can be served. A pairing without
    /// a valid 32-byte key has neither: it must pair again.
    fn credentials(&self, id: &str) -> Option<(&str, SharedKey)> {
        let secret = self.secrets.get(id)?;
        let key = SharedKey::from_base64(secret.e2ee_key.as_deref()?)?;
        Some((secret.token.as_str(), key))
    }

    pub(crate) fn token_for(&mut self, id: &str) -> Option<&str> {
        self.retry_unreadable();
        self.credentials(id).map(|(token, _)| token)
    }

    pub(crate) fn e2ee_key_for(&mut self, id: &str) -> Option<SharedKey> {
        self.retry_unreadable();
        self.credentials(id).map(|(_, key)| key)
    }

    /// Which watch presented `token`; constant-time per device. A watch
    /// without a key never matches, so it reads 403 and pairs again.
    pub(crate) fn device_for_token(&mut self, token: &str) -> Option<String> {
        self.retry_unreadable();
        self.devices
            .iter()
            .find(|device| {
                self.credentials(&device.id)
                    .is_some_and(|(expected, _)| tokens_equal(token, expected))
            })
            .map(|device| device.id.clone())
    }

    pub(crate) fn requires_e2ee(&self, id: &str) -> bool {
        self.position(id)
            .is_some_and(|index| self.devices[index].requires_e2ee == Some(true))
    }

    /// One way: only `enroll` clears it. Returns whether anything changed.
    pub(crate) fn require_e2ee(&mut self, id: &str) -> bool {
        let Some(index) = self.position(id) else {
            return false;
        };
        if self.devices[index].requires_e2ee == Some(true) {
            return false;
        }
        self.devices[index].requires_e2ee = Some(true);
        let _ = self.save();
        true
    }

    /// Records a call from `id`: `last_seen` at most once a minute, and the
    /// protocol and app version whenever they change.
    pub(crate) fn touch(&mut self, id: &str, peer: Option<&PeerInfo>, now: SystemTime) {
        let Some(index) = self.position(id) else {
            return;
        };
        let device = &mut self.devices[index];
        let app_version = peer
            .and_then(|peer| peer.app_version.clone())
            .unwrap_or_default();
        let version_changed = peer.is_some_and(|peer| {
            device.last_protocol != Some(peer.protocol_version)
                || device.last_app_version.as_deref().unwrap_or("") != app_version
        });
        if !version_changed
            && let Some(seen) = Timestamp::parse(&device.last_seen)
            && now
                .duration_since(seen)
                .is_ok_and(|age| age < TOUCH_INTERVAL)
        {
            return;
        }
        device.last_seen = Timestamp::string(now);
        if let Some(peer) = peer {
            device.last_protocol = Some(peer.protocol_version);
            device.last_app_version = Some(app_version);
        }
        let _ = self.save();
    }

    pub(crate) fn last_protocol(&self, id: &str) -> Option<u32> {
        self.position(id)
            .and_then(|index| self.devices[index].last_protocol)
    }

    pub(crate) fn summary(&mut self) -> Vec<DeviceSummary> {
        self.retry_unreadable();
        let now = (self.clock)();
        self.devices
            .iter()
            .map(|device| DeviceSummary {
                id: device.id.clone(),
                name: device.name.clone(),
                last_paired_at: device
                    .last_paired_at
                    .clone()
                    .unwrap_or_else(|| device.paired_at.clone()),
                last_seen: device.last_seen.clone(),
                last_protocol: device.last_protocol.unwrap_or(super::MINIMUM_PEER_PROTOCOL),
                last_app_version: device.last_app_version.clone().unwrap_or_default(),
                status: if let Some(since) = self.unreadable.get(&device.id) {
                    if now.duration_since(*since) < RETRY_INTERVAL {
                        DeviceStatus::SecretsUnavailable
                    } else {
                        DeviceStatus::NeedsRepair
                    }
                } else if self.credentials(&device.id).is_some() {
                    DeviceStatus::Ready
                } else {
                    DeviceStatus::NeedsRepair
                },
            })
            .collect()
    }

    /// Makes every save fail with `why`, as a full disk would.
    #[cfg(test)]
    pub(crate) fn refuse_saves(&mut self, why: &str) {
        self.unwritable = Some(why.to_owned());
    }

    /// Undoes [`Self::refuse_saves`].
    #[cfg(test)]
    pub(crate) fn allow_saves(&mut self) {
        self.unwritable = None;
    }

    pub(crate) fn devices(&self) -> &[PairedDevice] {
        &self.devices
    }

    /// Writes the list owner-only, without secrets except the ones the
    /// import has not settled yet (`in_file`, `pending_file_secrets`).
    fn save(&mut self) -> io::Result<()> {
        let devices = self
            .devices
            .iter()
            .map(|device| {
                let kept = self
                    .pending_file_secrets
                    .as_ref()
                    .and_then(|pending| pending.get(&device.id))
                    .or_else(|| {
                        self.in_file
                            .contains(&device.id)
                            .then(|| self.secrets.get(&device.id))
                            .flatten()
                    });
                let mut device = device.clone();
                if let Some(secret) = kept {
                    device
                        .extra
                        .insert("token".into(), Value::from(secret.token.clone()));
                    if let Some(key) = &secret.e2ee_key {
                        device
                            .extra
                            .insert("e2ee_key".into(), Value::from(key.clone()));
                    }
                }
                device
            })
            .collect();
        let written = match &self.unwritable {
            Some(reason) => Err(io::Error::other(reason.clone())),
            None => devices_file::write(&self.path, devices, &self.unparsed),
        };
        if let Err(error) = &written {
            self.note(format!("could not write {FILE_NAME}: {error}"));
        }
        written
    }
}

#[cfg(test)]
mod tests {
    use super::requests::{PairingRequest, PairingRequests, PairingResult};
    use super::*;
    use crate::watch::e2ee::{self, open_pairing_result};
    use p256::SecretKey;
    use p256::elliptic_curve::sec1::ToSec1Point;
    use std::cell::Cell;
    use std::rc::Rc;

    /// A store whose writes fail while `refusing` is set; reads and
    /// deletes go through.
    struct SavesRefused(Rc<MemorySecretStore>, Rc<Cell<bool>>);

    impl SecretStore for SavesRefused {
        fn load(&self, id: &str) -> Result<Option<DeviceSecret>, String> {
            self.0.load(id)
        }
        fn save(&self, id: &str, secret: &DeviceSecret) -> Result<(), String> {
            if self.1.get() {
                return Err("write refused".into());
            }
            self.0.save(id, secret)
        }
        fn delete(&self, id: &str) -> Result<(), String> {
            self.0.delete(id)
        }
        fn migration_completed(&self) -> Result<bool, String> {
            self.0.migration_completed()
        }
        fn mark_migration_completed(&self) -> Result<(), String> {
            self.0.mark_migration_completed()
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("hex-watch-pairing-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn load(dir: &Path, store: Rc<MemorySecretStore>) -> PairedDevices {
        PairedDevices::load(&dir.join(FILE_NAME), Box::new(store))
    }

    /// A clock the test moves by hand.
    fn load_at(
        dir: &Path,
        store: Rc<MemorySecretStore>,
        clock: Rc<Cell<Instant>>,
    ) -> PairedDevices {
        PairedDevices::load_with_clock(
            &dir.join(FILE_NAME),
            Box::new(store),
            Box::new(move || clock.get()),
        )
    }

    fn advance(clock: &Cell<Instant>, by: Duration) {
        clock.set(clock.get() + by);
    }

    fn key() -> SharedKey {
        SharedKey::from_base64("w9fFiEsXKqe7xFZ8w+SKs5nTn9bYRhMYXEOB17J8uO0=").unwrap()
    }

    fn keyed(token: &str) -> DeviceSecret {
        DeviceSecret {
            token: token.into(),
            e2ee_key: Some(key().to_base64()),
        }
    }

    fn file_text(dir: &Path) -> String {
        std::fs::read_to_string(dir.join(FILE_NAME)).unwrap()
    }

    fn status(paired: &mut PairedDevices, id: &str) -> DeviceStatus {
        paired
            .summary()
            .into_iter()
            .find(|device| device.id == id)
            .unwrap()
            .status
    }

    const ONE_DEVICE: &[u8] =
        br#"{"devices":[{"id":"w1","name":"W","paired_at":"","last_seen":""}]}"#;
    const LATER: SystemTime = SystemTime::UNIX_EPOCH;

    fn watch_public_key() -> String {
        let secret = SecretKey::from_slice(&[9u8; 32]).unwrap();
        base64::engine::general_purpose::STANDARD
            .encode(secret.public_key().to_sec1_point(false).as_bytes())
    }

    const NOW: SystemTime = SystemTime::UNIX_EPOCH;

    fn req<'a>(
        claimed: &'a str,
        device: &'a str,
        name: &'a str,
        key: &'a str,
    ) -> PairingRequest<'a> {
        PairingRequest {
            claimed_device_id: claimed,
            device_id: device,
            name,
            watch_public_key: key,
        }
    }

    #[test]
    fn tokens_are_32_random_bytes_base64url_without_padding() {
        let token = new_token().unwrap();
        assert_eq!(token.len(), 43);
        assert!(
            token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
        assert_ne!(token, new_token().unwrap());
        assert_eq!(URL_SAFE_NO_PAD.decode(&token).unwrap().len(), 32);
    }

    #[test]
    fn token_comparison_needs_equal_nonempty_values() {
        assert!(tokens_equal("abc", "abc"));
        assert!(!tokens_equal("abc", "abd"));
        assert!(!tokens_equal("abc", "abcd"));
        assert!(!tokens_equal("", ""));
        assert!(!tokens_equal("", "abc"));
    }

    #[test]
    fn wristpers_file_loads_and_is_written_back_without_secrets() {
        let dir = temp_dir("file");
        let store = Rc::new(MemorySecretStore::default());
        store
            .save(
                "w1",
                &DeviceSecret {
                    token: "tok1".into(),
                    e2ee_key: Some(
                        SharedKey::from_base64("w9fFiEsXKqe7xFZ8w+SKs5nTn9bYRhMYXEOB17J8uO0=")
                            .unwrap()
                            .to_base64(),
                    ),
                },
            )
            .unwrap();
        std::fs::write(
            dir.join(FILE_NAME),
            br#"{"devices":[{"id":"w1","name":"Apple Watch","paired_at":"2026-10-01T00:00:00.000000+00:00","last_paired_at":"2026-10-01T00:00:00.000000+00:00","last_seen":"2026-10-02T00:00:00.000000+00:00","requires_e2ee":true,"last_protocol":4,"last_app_version":"1.4","push":{"platform":"watchos","token":"abc"}}]}"#,
        )
        .unwrap();
        let mut paired = load(&dir, store.clone());
        assert_eq!(paired.devices().len(), 1);
        assert_eq!(paired.token_for("w1"), Some("tok1"));
        assert!(paired.e2ee_key_for("w1").is_some());
        assert!(paired.requires_e2ee("w1"));
        assert_eq!(paired.last_protocol("w1"), Some(4));
        assert_eq!(paired.device_for_token("tok1").as_deref(), Some("w1"));
        assert_eq!(paired.device_for_token("tok2"), None);
        assert!(!paired.secrets_unavailable());
        paired.touch(
            "w1",
            Some(&PeerInfo {
                protocol_version: 4,
                app_version: Some("1.5".into()),
            }),
            NOW + Duration::from_secs(1_800_000_000),
        );
        let written: Value =
            serde_json::from_slice(&std::fs::read(dir.join(FILE_NAME)).unwrap()).unwrap();
        let device = &written["devices"][0];
        assert_eq!(
            device["push"]["token"], "abc",
            "unknown keys survive a rewrite"
        );
        assert_eq!(device["last_app_version"], "1.5");
        assert!(device.get("token").is_none());
        assert!(device.get("e2ee_key").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn secrets_left_in_the_file_move_into_the_store_once() {
        let dir = temp_dir("import");
        std::fs::write(
            dir.join(FILE_NAME),
            br#"{"devices":[{"id":"old","name":"Old Watch","token":"legacy-token","e2ee_key":"w9fFiEsXKqe7xFZ8w+SKs5nTn9bYRhMYXEOB17J8uO0=","paired_at":"","last_seen":"","sas_verified":true}]}"#,
        )
        .unwrap();
        let store = Rc::new(MemorySecretStore::default());
        let mut paired = load(&dir, store.clone());
        assert_eq!(paired.token_for("old"), Some("legacy-token"));
        assert_eq!(store.load("old").unwrap().unwrap().token, "legacy-token");
        assert_eq!(store.migration_completed(), Ok(true));
        let written = std::fs::read_to_string(dir.join(FILE_NAME)).unwrap();
        assert!(!written.contains("legacy-token"), "{written}");
        assert!(!written.contains("sas_verified"), "{written}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A locked file Keychain still says "not found" for missing items, so
    /// answering other reads proves nothing about the paired watch: it must
    /// keep reading 503, never the 403 that makes the watch forget Hex.
    #[test]
    fn a_locked_keychain_never_answers_403_for_a_paired_watch() {
        let dir = temp_dir("locked");
        std::fs::write(
            dir.join(FILE_NAME),
            br#"{"devices":[{"id":"w1","name":"W","paired_at":"","last_seen":""},{"id":"w2","name":"W2","paired_at":"","last_seen":""}]}"#,
        )
        .unwrap();
        let store = Rc::new(MemorySecretStore::default());
        store.save("w1", &keyed("t")).unwrap();
        store.mark_migration_completed().unwrap();
        store.locked.set(true);
        assert_eq!(
            store.load("w2"),
            Ok(None),
            "the mock answers like a locked Keychain"
        );
        let clock = Rc::new(Cell::new(Instant::now()));
        let mut paired = load_at(&dir, store.clone(), clock.clone());
        assert_eq!(status(&mut paired, "w1"), DeviceStatus::SecretsUnavailable);
        for _ in 0..20 {
            assert_eq!(paired.device_for_token("t"), None);
            assert!(
                paired.secrets_unavailable(),
                "an unmatched token while w1 is unreadable answers 503, not 403"
            );
            assert!(paired.secrets_unavailable_for("w1"));
            advance(&clock, RETRY_INTERVAL);
        }
        assert_eq!(
            status(&mut paired, "w1"),
            DeviceStatus::NeedsRepair,
            "Settings suggests pairing again; requests still answer 503"
        );
        assert!(paired.secrets_unavailable());
        store.locked.set(false);
        advance(&clock, RETRY_INTERVAL);
        assert!(!paired.secrets_unavailable());
        assert_eq!(paired.device_for_token("t").as_deref(), Some("w1"));
        assert_eq!(status(&mut paired, "w1"), DeviceStatus::Ready);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn enroll_replaces_the_token_clears_the_ratchet_and_revoke_removes_both() {
        let dir = temp_dir("enroll");
        let store = Rc::new(MemorySecretStore::default());
        let mut paired = load(&dir, store.clone());
        let key = key();
        let first = paired.enroll("w1", "Apple Watch", &key, NOW).unwrap();
        assert!(paired.require_e2ee("w1"));
        assert!(!paired.require_e2ee("w1"), "already on");
        assert!(paired.requires_e2ee("w1"));
        let second = paired.enroll("w1", "", &key, NOW).unwrap();
        assert_ne!(first, second);
        assert_eq!(paired.token_for("w1"), Some(second.as_str()));
        assert_eq!(
            paired.device_for_token(&first),
            None,
            "the old token is dead"
        );
        assert!(
            !paired.requires_e2ee("w1"),
            "a new pairing starts without the ratchet"
        );
        assert_eq!(
            paired.devices()[0].name,
            "Apple Watch",
            "an empty name keeps the old one"
        );
        assert_eq!(paired.devices().len(), 1);
        assert_eq!(paired.revoke("w1"), Ok(true));
        assert_eq!(paired.revoke("w1"), Ok(false));
        assert_eq!(store.load("w1").unwrap(), None);
        assert!(paired.devices().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn enroll_is_void_when_the_store_refuses() {
        let dir = temp_dir("void");
        let store = Rc::new(MemorySecretStore::default());
        let mut paired = load(&dir, store.clone());
        store.locked.set(true);
        assert_eq!(paired.enroll("w1", "W", &key(), NOW), None);
        assert!(paired.devices().is_empty());
        assert!(!dir.join(FILE_NAME).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn typing_the_code_approves_the_claimed_watch_once_for_120_seconds() {
        let dir = temp_dir("approve");
        let store = Rc::new(MemorySecretStore::default());
        let mut paired = load(&dir, store.clone());
        let mut requests = PairingRequests::new();
        let start = Instant::now();
        let key = watch_public_key();
        assert_eq!(
            requests.admit(
                &req("w1", "w1", "Apple Watch", &key),
                &mut paired,
                start,
                NOW
            ),
            None,
            "no approval yet"
        );
        requests.approve("w1", start);
        assert_eq!(requests.window_remaining(start), APPROVAL_WINDOW);
        assert_eq!(
            requests.admit(&req("w9", "w9", "Other", &key), &mut paired, start, NOW),
            None,
            "only the claimed watch"
        );
        assert!(
            requests.window_remaining(start + Duration::from_secs(1)) > Duration::ZERO,
            "a wrong watch does not spend it"
        );
        let late = start + APPROVAL_WINDOW;
        assert_eq!(
            requests.admit(
                &req("w1", "w1", "Apple Watch", &key),
                &mut paired,
                late,
                NOW
            ),
            None,
            "expired"
        );
        requests.approve("w1", start);
        let request_id = requests
            .admit(
                &req("w1", "w1", "Apple Watch", &key),
                &mut paired,
                start + Duration::from_secs(5),
                NOW,
            )
            .unwrap();
        assert_eq!(request_id.len(), 32);
        assert_eq!(
            requests.window_remaining(start + Duration::from_secs(5)),
            Duration::ZERO,
            "single use"
        );
        assert_eq!(
            requests.admit(
                &req("w1", "w1", "Apple Watch", &key),
                &mut paired,
                start + Duration::from_secs(6),
                NOW
            ),
            None
        );
        assert_eq!(requests.failure(&request_id, start), None);
        match requests.result(&request_id, start + Duration::from_secs(10)) {
            PairingResult::Sealed {
                sealed,
                mac_public_key,
            } => {
                let watch_secret = SecretKey::from_slice(&[9u8; 32]).unwrap();
                let agreement = e2ee::agree_with(&watch_secret, &mac_public_key).unwrap();
                let token = open_pairing_result(&sealed, &agreement.key, &request_id).unwrap();
                assert_eq!(paired.token_for("w1"), Some(token.as_str()));
                assert_eq!(paired.e2ee_key_for("w1"), Some(agreement.key));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(requests.completed_count(), 1);
        assert!(
            matches!(
                requests.result(&request_id, start + Duration::from_secs(299)),
                PairingResult::Sealed { .. }
            ),
            "re-readable"
        );
        assert_eq!(
            requests.result(
                &request_id,
                start + REQUEST_LIFETIME + Duration::from_secs(5)
            ),
            PairingResult::Unknown
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn completion_forgets_the_result_and_counts_once() {
        let dir = temp_dir("complete");
        let store = Rc::new(MemorySecretStore::default());
        let mut paired = load(&dir, store.clone());
        let mut requests = PairingRequests::new();
        let start = Instant::now();
        requests.approve("w1", start);
        let request_id = requests
            .admit(
                &req("w1", "w1", "Watch", &watch_public_key()),
                &mut paired,
                start,
                NOW,
            )
            .unwrap();
        assert!(matches!(
            requests.result(&request_id, start),
            PairingResult::Sealed { .. }
        ));
        requests.completed("w1");
        assert_eq!(requests.result(&request_id, start), PairingResult::Unknown);
        assert_eq!(requests.completed_count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_secret_the_store_cannot_read_during_import_stays_in_the_file() {
        let dir = temp_dir("keep-in-file");
        std::fs::write(
            dir.join(FILE_NAME),
            br#"{"devices":[{"id":"w1","name":"W","token":"file-token","e2ee_key":"w9fFiEsXKqe7xFZ8w+SKs5nTn9bYRhMYXEOB17J8uO0=","paired_at":"","last_seen":""}]}"#,
        )
        .unwrap();
        let store = Rc::new(MemorySecretStore::default());
        store.foreign.borrow_mut().insert("w1".into());
        let mut paired = load(&dir, store.clone());
        assert_eq!(paired.device_for_token("file-token").as_deref(), Some("w1"));
        paired.touch("w1", None, LATER + Duration::from_secs(1_800_000_000));
        let written = file_text(&dir);
        assert!(written.contains("file-token"), "{written}");
        assert!(written.contains("e2ee_key"), "{written}");
        assert_eq!(
            store.migration_completed(),
            Ok(false),
            "an unfinished import is retried at the next launch"
        );
        store.foreign.borrow_mut().clear();
        let reloaded = load(&dir, store.clone());
        assert_eq!(store.load("w1").unwrap().unwrap().token, "file-token");
        assert!(!file_text(&dir).contains("file-token"));
        assert_eq!(store.migration_completed(), Ok(true));
        drop(reloaded);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn after_the_migration_the_files_secrets_are_never_imported() {
        let dir = temp_dir("migrated");
        std::fs::write(
            dir.join(FILE_NAME),
            br#"{"devices":[{"id":"w1","name":"W","token":"planted","e2ee_key":"w9fFiEsXKqe7xFZ8w+SKs5nTn9bYRhMYXEOB17J8uO0=","paired_at":"","last_seen":""}]}"#,
        )
        .unwrap();
        let store = Rc::new(MemorySecretStore::default());
        store.mark_migration_completed().unwrap();
        let mut paired = load(&dir, store.clone());
        assert_eq!(paired.device_for_token("planted"), None);
        assert_eq!(store.load("w1").unwrap(), None);
        assert_eq!(status(&mut paired, "w1"), DeviceStatus::NeedsRepair);
        assert!(!file_text(&dir).contains("planted"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreadable_marker_leaves_the_files_secrets_alone_until_it_reads() {
        let dir = temp_dir("pending");
        std::fs::write(
            dir.join(FILE_NAME),
            br#"{"devices":[{"id":"w1","name":"W","token":"file-token","e2ee_key":"w9fFiEsXKqe7xFZ8w+SKs5nTn9bYRhMYXEOB17J8uO0=","paired_at":"","last_seen":""}]}"#,
        )
        .unwrap();
        let store = Rc::new(MemorySecretStore::default());
        store.unreachable.set(true);
        let clock = Rc::new(Cell::new(Instant::now()));
        let mut paired = load_at(&dir, store.clone(), clock.clone());
        assert!(paired.secrets_unavailable_for("w1"));
        assert_eq!(
            paired.device_for_token("file-token"),
            None,
            "not imported yet"
        );
        paired.touch("w1", None, LATER + Duration::from_secs(1_800_000_000));
        assert!(
            file_text(&dir).contains("file-token"),
            "written back as it was"
        );
        store.unreachable.set(false);
        advance(&clock, RETRY_INTERVAL);
        assert_eq!(paired.device_for_token("file-token").as_deref(), Some("w1"));
        assert_eq!(store.load("w1").unwrap().unwrap().token, "file-token");
        assert!(!file_text(&dir).contains("file-token"));
        assert_eq!(store.migration_completed(), Ok(true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unreadable_secrets_are_retried_at_most_every_30_seconds() {
        let dir = temp_dir("retry");
        std::fs::write(dir.join(FILE_NAME), ONE_DEVICE).unwrap();
        let store = Rc::new(MemorySecretStore::default());
        store.save("w1", &keyed("t")).unwrap();
        store.mark_migration_completed().unwrap();
        store.locked.set(true);
        let clock = Rc::new(Cell::new(Instant::now()));
        let mut paired = load_at(&dir, store.clone(), clock.clone());
        store.locked.set(false);
        advance(&clock, RETRY_INTERVAL - Duration::from_millis(1));
        assert_eq!(paired.device_for_token("t"), None, "inside the window");
        advance(&clock, Duration::from_millis(1));
        assert_eq!(paired.device_for_token("t").as_deref(), Some("w1"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_item_that_stays_unreadable_keeps_503_and_settings_says_pair_again() {
        let dir = temp_dir("stays-unreadable");
        std::fs::write(
            dir.join(FILE_NAME),
            br#"{"devices":[{"id":"w1","name":"W","paired_at":"","last_seen":""},{"id":"w2","name":"W2","paired_at":"","last_seen":""}]}"#,
        )
        .unwrap();
        let store = Rc::new(MemorySecretStore::default());
        store.save("w1", &keyed("t1")).unwrap();
        store.save("w2", &keyed("t2")).unwrap();
        store.mark_migration_completed().unwrap();
        store.foreign.borrow_mut().insert("w1".into());
        let clock = Rc::new(Cell::new(Instant::now()));
        let mut paired = load_at(&dir, store.clone(), clock.clone());
        assert_eq!(paired.device_for_token("t2").as_deref(), Some("w2"));
        assert_eq!(status(&mut paired, "w1"), DeviceStatus::SecretsUnavailable);
        advance(&clock, Duration::from_secs(3_600));
        assert!(paired.secrets_unavailable(), "still 503, never 403");
        assert!(paired.secrets_unavailable_for("w1"));
        assert_eq!(paired.device_for_token("t1"), None);
        assert_eq!(status(&mut paired, "w1"), DeviceStatus::NeedsRepair);
        assert_eq!(status(&mut paired, "w2"), DeviceStatus::Ready);
        store.foreign.borrow_mut().clear();
        advance(&clock, RETRY_INTERVAL);
        assert_eq!(paired.device_for_token("t1").as_deref(), Some("w1"));
        assert!(!paired.secrets_unavailable());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pairing_again_replaces_an_item_hex_cannot_read() {
        let dir = temp_dir("overwrite");
        std::fs::write(dir.join(FILE_NAME), ONE_DEVICE).unwrap();
        let store = Rc::new(MemorySecretStore::default());
        store.save("w1", &keyed("old")).unwrap();
        store.mark_migration_completed().unwrap();
        store.foreign.borrow_mut().insert("w1".into());
        let mut paired = load(&dir, store.clone());
        let token = paired.enroll("w1", "W", &key(), NOW).unwrap();
        assert_eq!(store.load("w1").unwrap().unwrap().token, token);
        assert_eq!(paired.device_for_token(&token).as_deref(), Some("w1"));
        assert!(!paired.secrets_unavailable());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_refused_delete_voids_the_pairing_and_leaves_the_old_item_alone() {
        let dir = temp_dir("undeletable");
        std::fs::write(dir.join(FILE_NAME), ONE_DEVICE).unwrap();
        let store = Rc::new(MemorySecretStore::default());
        store.save("w1", &keyed("old")).unwrap();
        store.mark_migration_completed().unwrap();
        store.foreign.borrow_mut().insert("w1".into());
        store.undeletable.borrow_mut().insert("w1".into());
        let mut paired = load(&dir, store.clone());
        assert_eq!(paired.enroll("w1", "W", &key(), NOW), None);
        assert_eq!(paired.devices().len(), 1);
        assert!(
            paired.secrets_unavailable_for("w1"),
            "still 503 for the old pairing"
        );
        store.foreign.borrow_mut().clear();
        assert_eq!(
            store.load("w1").unwrap().unwrap().token,
            "old",
            "the old item was never written"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_bad_entry_keeps_the_others_and_is_written_back_unchanged() {
        let dir = temp_dir("bad-entry");
        let bad = r#"{"id":"w0","name":7,"token":"keep-me"}"#;
        std::fs::write(
            dir.join(FILE_NAME),
            format!(
                r#"{{"devices":[{bad},"junk",{{"id":"w1","name":"W","paired_at":"","last_seen":""}}]}}"#
            ),
        )
        .unwrap();
        let store = Rc::new(MemorySecretStore::default());
        store.save("w1", &keyed("t1")).unwrap();
        store.mark_migration_completed().unwrap();
        let mut paired = load(&dir, store.clone());
        assert_eq!(paired.devices().len(), 1);
        assert_eq!(paired.device_for_token("t1").as_deref(), Some("w1"));
        paired.touch("w1", None, LATER + Duration::from_secs(1_800_000_000));
        let written: Value = serde_json::from_str(&file_text(&dir)).unwrap();
        let entries = written["devices"].as_array().unwrap();
        assert_eq!(entries.len(), 3);
        assert!(entries.contains(&serde_json::from_str::<Value>(bad).unwrap()));
        assert!(entries.contains(&Value::from("junk")));
        let token = paired.enroll("w0", "W0", &key(), NOW).unwrap();
        let written: Value = serde_json::from_str(&file_text(&dir)).unwrap();
        let ids: Vec<&Value> = written["devices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| &entry["id"])
            .collect();
        assert_eq!(
            ids.iter().filter(|id| **id == "w0").count(),
            1,
            "pairing again replaced it"
        );
        assert!(!file_text(&dir).contains("keep-me"));
        assert_eq!(paired.device_for_token(&token).as_deref(), Some("w0"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_is_not_a_device_list_is_never_overwritten() {
        let dir = temp_dir("not-a-list");
        let original = br#"{"devices":{"w1":"not a list"}}"#;
        std::fs::write(dir.join(FILE_NAME), original).unwrap();
        let store = Rc::new(MemorySecretStore::default());
        let mut paired = load(&dir, store.clone());
        assert!(paired.devices().is_empty());
        assert_eq!(
            paired.enroll("w1", "W", &key(), NOW),
            None,
            "a new watch needs the list"
        );
        assert_eq!(std::fs::read(dir.join(FILE_NAME)).unwrap(), original);
        assert_eq!(store.load("w1"), Ok(None), "its secret was taken back");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pairing_without_a_key_needs_repair_and_never_authenticates() {
        let dir = temp_dir("keyless");
        std::fs::write(dir.join(FILE_NAME), ONE_DEVICE).unwrap();
        let store = Rc::new(MemorySecretStore::default());
        store
            .save(
                "w1",
                &DeviceSecret {
                    token: "plain".into(),
                    e2ee_key: None,
                },
            )
            .unwrap();
        store.mark_migration_completed().unwrap();
        let mut paired = load(&dir, store.clone());
        assert_eq!(paired.device_for_token("plain"), None);
        assert_eq!(paired.token_for("w1"), None);
        assert!(!paired.secrets_unavailable());
        assert_eq!(status(&mut paired, "w1"), DeviceStatus::NeedsRepair);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bad_public_key_or_locked_store_fails_the_request_but_spends_the_approval() {
        let dir = temp_dir("fail");
        let store = Rc::new(MemorySecretStore::default());
        let mut paired = load(&dir, store.clone());
        let mut requests = PairingRequests::new();
        let start = Instant::now();
        requests.approve("w1", start);
        let bad = requests
            .admit(
                &req("w1", "w1", "W", "bm90IGEga2V5"),
                &mut paired,
                start,
                NOW,
            )
            .unwrap();
        assert_eq!(requests.failure(&bad, start), Some(ENCRYPTION_FAILED));
        assert_eq!(requests.result(&bad, start), PairingResult::Unknown);
        assert!(paired.devices().is_empty());
        store.locked.set(true);
        requests.approve("w2", start);
        assert_eq!(
            requests.failure(&bad, start),
            None,
            "a new claim clears old failures"
        );
        let locked = requests
            .admit(
                &req("w2", "w2", "W", &watch_public_key()),
                &mut paired,
                start,
                NOW,
            )
            .unwrap();
        assert_eq!(requests.failure(&locked, start), Some(KEYCHAIN_UNAVAILABLE));
        assert_eq!(
            watch_facing_failure(KEYCHAIN_UNAVAILABLE),
            ENCRYPTION_FAILED
        );
        assert_eq!(requests.latest_failure(start), Some(KEYCHAIN_UNAVAILABLE));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn previous_token_reuses_the_older_entry() {
        let dir = temp_dir("previous");
        let store = Rc::new(MemorySecretStore::default());
        let mut paired = load(&dir, store.clone());
        let mut requests = PairingRequests::new();
        let start = Instant::now();
        requests.approve("w-old", start);
        let first = requests
            .admit(
                &req("w-old", "w-old", "W", &watch_public_key()),
                &mut paired,
                start,
                NOW,
            )
            .unwrap();
        let PairingResult::Sealed { .. } = requests.result(&first, start) else {
            panic!()
        };
        let old_token = paired.token_for("w-old").unwrap().to_owned();
        requests.approve("w-new", start);
        let owner = paired.device_for_token(&old_token).unwrap();
        let _ = requests
            .admit(
                &req("w-new", &owner, "W", &watch_public_key()),
                &mut paired,
                start,
                NOW,
            )
            .unwrap();
        assert_eq!(
            paired.devices().len(),
            1,
            "a reinstalled watch keeps one entry"
        );
        assert_eq!(paired.devices()[0].id, "w-old");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn paired_w1(dir: &Path, store: &Rc<MemorySecretStore>) -> (PairedDevices, String) {
        let mut paired = load(dir, store.clone());
        let token = paired.enroll("w1", "W", &key(), NOW).unwrap();
        (paired, token)
    }

    #[test]
    fn a_revoke_whose_list_cannot_be_written_changes_nothing() {
        let dir = temp_dir("revoke-unsaved");
        let store = Rc::new(MemorySecretStore::default());
        let (mut paired, token) = paired_w1(&dir, &store);
        paired.refuse_saves("disk full");
        let error = paired.revoke("w1").unwrap_err();
        assert_eq!(error, format!("could not write {FILE_NAME}: disk full"));
        assert_eq!(paired.devices().len(), 1, "the watch is still listed");
        assert_eq!(status(&mut paired, "w1"), DeviceStatus::Ready);
        assert_eq!(paired.device_for_token(&token).as_deref(), Some("w1"));
        assert_eq!(
            store.load("w1").unwrap().map(|secret| secret.token),
            Some(token.clone()),
            "the keychain item was not touched"
        );
        // Memory and disk still agree: a restart reads the same watch.
        let mut restarted = load(&dir, store.clone());
        assert_eq!(restarted.device_for_token(&token).as_deref(), Some("w1"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_revoke_tried_again_after_a_failure_removes_the_watch() {
        let dir = temp_dir("revoke-again");
        let store = Rc::new(MemorySecretStore::default());
        let (mut paired, token) = paired_w1(&dir, &store);
        paired.refuse_saves("disk full");
        assert!(paired.revoke("w1").is_err());
        paired.allow_saves();
        assert_eq!(paired.revoke("w1"), Ok(true));
        assert!(paired.devices().is_empty());
        assert_eq!(paired.device_for_token(&token), None);
        assert_eq!(store.load("w1").unwrap(), None);
        let mut restarted = load(&dir, store.clone());
        assert!(restarted.devices().is_empty());
        assert_eq!(restarted.device_for_token(&token), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_keychain_item_without_a_list_entry_never_authenticates() {
        let dir = temp_dir("revoke-orphan");
        let store = Rc::new(MemorySecretStore::default());
        let (mut paired, token) = paired_w1(&dir, &store);
        store.undeletable.borrow_mut().insert("w1".into());
        assert_eq!(paired.revoke("w1"), Ok(true));
        assert_eq!(paired.device_for_token(&token), None);
        assert_eq!(
            store.load("w1").unwrap().map(|secret| secret.token),
            Some(token.clone()),
            "the orphan item is still there"
        );
        let mut restarted = load(&dir, store.clone());
        assert_eq!(restarted.device_for_token(&token), None);
        assert_eq!(restarted.token_for("w1"), None);
        assert!(restarted.devices().is_empty());
        assert!(!restarted.secrets_unavailable());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_re_pair_that_deletes_the_old_secret_but_cannot_store_the_new_one_needs_repair() {
        let dir = temp_dir("repair-void");
        let store = Rc::new(MemorySecretStore::default());
        let refusing = Rc::new(Cell::new(false));
        let mut paired = PairedDevices::load(
            &dir.join(FILE_NAME),
            Box::new(SavesRefused(store.clone(), refusing.clone())),
        );
        let old = paired.enroll("w1", "W", &key(), NOW).unwrap();
        refusing.set(true);
        assert_eq!(paired.enroll("w1", "W", &key(), NOW), None);
        assert_eq!(paired.device_for_token(&old), None, "the old token is gone");
        assert_eq!(paired.token_for("w1"), None);
        assert_eq!(paired.e2ee_key_for("w1"), None);
        assert!(!paired.secrets_unavailable_for("w1"));
        assert_eq!(status(&mut paired, "w1"), DeviceStatus::NeedsRepair);
        let mut restarted = load(&dir, store.clone());
        assert_eq!(restarted.device_for_token(&old), None);
        assert_eq!(status(&mut restarted, "w1"), DeviceStatus::NeedsRepair);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
