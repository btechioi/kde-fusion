//! Secret storage.
//!
//! KDE Plasma provides a Secret Service implementation (`ksecretd` with
//! KWallet).  We use `secret-tool`, which is part of libsecret and available
//! on CachyOS.  If the Secret Service is unavailable (for example in a TTY or
//! in an automated test) we fall back to a 0600 JSON file under the state dir.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use crate::util::{atomic_write, command_exists, state_dir};

pub const SUNSHINE_PASSWORD: &str = "sunshine-password";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    SecretService,
    File,
}

#[derive(Serialize, Deserialize, Default)]
struct FileSecrets {
    #[serde(default)]
    values: HashMap<String, String>,
}

pub struct SecretStore {
    backend: Backend,
    file_path: PathBuf,
    cache: Mutex<HashMap<String, String>>,
}

impl SecretStore {
    pub fn new() -> Self {
        let file_path = state_dir().join("secrets.json");
        let force_file = std::env::var("KDEFUSION_SECRET_BACKEND")
            .map(|value| value.eq_ignore_ascii_case("file"))
            .unwrap_or(false);
        let backend = if !force_file && command_exists("secret-tool") {
            Backend::SecretService
        } else {
            Backend::File
        };
        Self {
            backend,
            file_path,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Always use the file backend.  Useful for tests and for users who
    /// explicitly do not want secrets in KWallet.
    pub fn file_only() -> Self {
        Self {
            backend: Backend::File,
            file_path: state_dir().join("secrets.json"),
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn file_only_at(path: impl Into<PathBuf>) -> Self {
        Self {
            backend: Backend::File,
            file_path: path.into(),
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }

    pub fn get(&self, key: &str) -> Option<String> {
        if let Some(value) = self.cache.lock().ok()?.get(key).cloned() {
            return Some(value);
        }
        let value = match self.backend {
            Backend::SecretService => self.lookup_secret_service(key),
            Backend::File => self.read_file().ok()?.values.get(key).cloned(),
        }?;
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(key.to_string(), value.clone());
        }
        Some(value)
    }

    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        match self.backend {
            Backend::SecretService => self.store_secret_service(key, value)?,
            Backend::File => self.set_file(key, value)?,
        }
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(key.to_string(), value.to_string());
        }
        Ok(())
    }

    pub fn delete(&self, key: &str) -> Result<()> {
        match self.backend {
            Backend::SecretService => {
                let _ = Command::new("secret-tool")
                    .args(["clear", "service", "kdefusion", "key", key])
                    .output();
            }
            Backend::File => {
                let mut data = self.read_file().unwrap_or_default();
                data.values.remove(key);
                self.write_file(&data)?;
            }
        }
        if let Ok(mut cache) = self.cache.lock() {
            cache.remove(key);
        }
        Ok(())
    }

    pub fn sunshine_password(&self) -> Option<String> {
        self.get(SUNSHINE_PASSWORD)
    }

    pub fn set_sunshine_password(&self, password: &str) -> Result<()> {
        self.set(SUNSHINE_PASSWORD, password)
    }

    pub fn peer_token(&self, device_id: &str) -> Option<String> {
        self.get(&peer_token_key(device_id))
    }

    pub fn set_peer_token(&self, device_id: &str, token: &str) -> Result<()> {
        self.set(&peer_token_key(device_id), token)
    }

    pub fn delete_peer_token(&self, device_id: &str) -> Result<()> {
        self.delete(&peer_token_key(device_id))
    }

    fn lookup_secret_service(&self, key: &str) -> Option<String> {
        let output = Command::new("secret-tool")
            .args(["lookup", "service", "kdefusion", "key", key])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let value = String::from_utf8_lossy(&output.stdout)
            .trim_end_matches(['\r', '\n'])
            .to_string();
        (!value.is_empty()).then_some(value)
    }

    fn store_secret_service(&self, key: &str, value: &str) -> Result<()> {
        let mut child = Command::new("secret-tool")
            .args([
                "store",
                "--label=KDE Fusion secret",
                "service",
                "kdefusion",
                "key",
                key,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("start secret-tool")?;
        child
            .stdin
            .as_mut()
            .context("secret-tool stdin")?
            .write_all(value.as_bytes())
            .context("write secret to secret-tool")?;
        let output = child.wait_with_output().context("wait for secret-tool")?;
        if !output.status.success() {
            anyhow::bail!(
                "secret-tool store failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    fn read_file(&self) -> Result<FileSecrets> {
        if !self.file_path.exists() {
            return Ok(FileSecrets::default());
        }
        let text = std::fs::read_to_string(&self.file_path)
            .with_context(|| format!("read secrets {}", self.file_path.display()))?;
        serde_json::from_str(&text)
            .with_context(|| format!("parse secrets {}", self.file_path.display()))
    }

    fn write_file(&self, data: &FileSecrets) -> Result<()> {
        let text = serde_json::to_vec_pretty(data).context("serialize secrets")?;
        atomic_write(&self.file_path, &text, Some(0o600))
    }

    fn set_file(&self, key: &str, value: &str) -> Result<()> {
        let mut data = self.read_file().unwrap_or_default();
        data.values.insert(key.to_string(), value.to_string());
        self.write_file(&data)
    }
}

impl Default for SecretStore {
    fn default() -> Self {
        Self::new()
    }
}

pub fn peer_token_key(device_id: &str) -> String {
    format!("peer-token-{device_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_backend_roundtrip() {
        let path = std::env::temp_dir().join(format!(
            "kdefusion-secret-{}.json",
            crate::util::random_hex(4)
        ));
        let store = SecretStore::file_only_at(path.clone());
        assert!(store.get("missing").is_none());
        store.set("alpha", "one").unwrap();
        assert_eq!(store.get("alpha").as_deref(), Some("one"));
        store.set("alpha", "two").unwrap();
        assert_eq!(store.get("alpha").as_deref(), Some("two"));
        store.delete("alpha").unwrap();
        assert!(store.get("alpha").is_none());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("two"));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn peer_token_key_is_stable() {
        assert_eq!(peer_token_key("abc"), "peer-token-abc");
    }
}
