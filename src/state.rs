//! Persistent, non-secret state: known peers and crash-recovery information.
//!
//! Tokens never live here.  They are fetched from [`crate::secrets`] by peer
//! device id when needed.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::types::{SessionMode, SessionStatus};
use crate::util::{atomic_write, now_unix, state_dir};

pub const STATE_FILE: &str = "state.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct State {
    pub peers: HashMap<String, PeerRecord>,
    pub active_session: Option<PersistedSession>,
    pub last_session: Option<PersistedSession>,
    pub version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PeerRecord {
    pub device_id: String,
    pub name: String,
    pub addresses: Vec<String>,
    pub control_port: u16,
    pub platform: String,
    pub version: String,
    pub paired_at: u64,
    pub last_seen: u64,
    /// Whether Moonlight on the peer has completed Sunshine pairing.
    pub sunshine_paired: bool,
    pub last_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PersistedSession {
    pub session_id: String,
    pub role: String,
    pub mode: SessionMode,
    pub peer_id: String,
    pub peer_name: String,
    pub started_at: u64,
    pub virtual_pid: Option<u32>,
    pub virtual_output: String,
    pub snapshot_path: String,
    pub audio: bool,
    pub started_sunshine: bool,
    pub status: SessionStatus,
}

pub struct StateStore {
    path: PathBuf,
    inner: Mutex<State>,
}

impl StateStore {
    pub fn load() -> Result<Self> {
        Self::load_from(state_dir().join(STATE_FILE))
    }

    pub fn load_from(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create state directory {}", parent.display()))?;
        }
        let state = if path.exists() {
            match std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| serde_json::from_str::<State>(&text).ok())
            {
                Some(mut state) => {
                    state.version = 1;
                    state
                }
                None => {
                    // Keep the damaged file for inspection instead of silently
                    // discarding it, then start clean.
                    let backup = path.with_extension(format!("corrupt-{}", now_unix()));
                    std::fs::rename(&path, &backup).ok();
                    tracing::warn!("state file was unreadable; moved to {}", backup.display());
                    State {
                        version: 1,
                        ..Default::default()
                    }
                }
            }
        } else {
            State {
                version: 1,
                ..Default::default()
            }
        };
        Ok(Self {
            path,
            inner: Mutex::new(state),
        })
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn get(&self) -> State {
        self.inner.lock().expect("state lock poisoned").clone()
    }

    pub fn update<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut State) -> T,
    {
        let mut state = self.inner.lock().expect("state lock poisoned");
        let result = f(&mut state);
        let text = serde_json::to_string_pretty(&*state).context("serialize state")?;
        atomic_write(&self.path, text.as_bytes(), Some(0o600))?;
        Ok(result)
    }

    pub fn save(&self) -> Result<()> {
        self.update(|_| ())
    }

    pub fn peer(&self, device_id: &str) -> Option<PeerRecord> {
        self.inner.lock().ok()?.peers.get(device_id).cloned()
    }

    pub fn peers(&self) -> Vec<PeerRecord> {
        let mut peers: Vec<_> = self
            .inner
            .lock()
            .map(|s| s.peers.values().cloned().collect())
            .unwrap_or_default();
        peers.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        peers
    }

    pub fn upsert_peer(&self, peer: PeerRecord) -> Result<()> {
        self.update(|state| {
            state.peers.insert(peer.device_id.clone(), peer);
        })
    }

    pub fn touch_peer(&self, device_id: &str, addresses: &[String], name: &str) -> Result<()> {
        self.update(|state| {
            if let Some(peer) = state.peers.get_mut(device_id) {
                peer.last_seen = now_unix();
                if !name.is_empty() {
                    peer.name = name.to_string();
                }
                // Replace (not append) so stale addresses from a previous IP are
                // dropped once discovery reports the current ones.
                peer.addresses = addresses.to_vec();
            }
        })
    }

    pub fn set_sunshine_paired(&self, device_id: &str, paired: bool) -> Result<()> {
        self.update(|state| {
            if let Some(peer) = state.peers.get_mut(device_id) {
                peer.sunshine_paired = paired;
            }
        })
    }

    pub fn remove_peer(&self, device_id: &str) -> Result<()> {
        self.update(|state| {
            state.peers.remove(device_id);
        })
    }

    pub fn set_active_session(&self, session: PersistedSession) -> Result<()> {
        self.update(|state| {
            state.active_session = Some(session);
        })
    }

    pub fn clear_active_session(&self) -> Result<()> {
        self.update(|state| {
            if let Some(session) = state.active_session.take() {
                state.last_session = Some(session);
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "kdefusion-state-{}-{name}.json",
            crate::util::random_hex(4)
        ))
    }

    #[test]
    fn state_roundtrip() {
        let path = temp_path("roundtrip");
        let store = StateStore::load_from(&path).unwrap();
        store
            .upsert_peer(PeerRecord {
                device_id: "abc".into(),
                name: "Other PC".into(),
                addresses: vec!["192.168.1.9".into()],
                control_port: 47811,
                paired_at: 1,
                ..Default::default()
            })
            .unwrap();
        let reloaded = StateStore::load_from(&path).unwrap().peer("abc").unwrap();
        assert_eq!(reloaded.name, "Other PC");
        assert_eq!(reloaded.addresses, vec!["192.168.1.9"]);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn corrupt_state_is_backed_up() {
        let path = temp_path("corrupt");
        std::fs::write(&path, b"not json").unwrap();
        let store = StateStore::load_from(&path).unwrap();
        assert!(store.peers().is_empty());
        assert!(!path.exists());
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn active_session_moves_to_last_session() {
        let path = temp_path("session");
        let store = StateStore::load_from(&path).unwrap();
        store
            .set_active_session(PersistedSession {
                session_id: "s1".into(),
                ..Default::default()
            })
            .unwrap();
        store.clear_active_session().unwrap();
        let state = store.get();
        assert!(state.active_session.is_none());
        assert_eq!(state.last_session.unwrap().session_id, "s1");
        std::fs::remove_file(path).ok();
    }
}
