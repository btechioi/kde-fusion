//! Configuration file handling.
//!
//! The file lives at `$XDG_CONFIG_HOME/kdefusion/config.toml`.  It contains
//! no secrets: tokens and the generated Sunshine password live in the Secret
//! Service (with a 0600 fallback file, see [`crate::secrets`]).

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use crate::util::{atomic_write, config_dir, hostname, random_hex};

pub const CONFIG_FILE: &str = "config.toml";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub device: DeviceConfig,
    pub control: ControlConfig,
    pub api: ApiConfig,
    pub sunshine: SunshineConfig,
    pub display: DisplayConfig,
    pub audio: AudioConfig,
    pub streaming: StreamingConfig,
    pub pairing: PairingConfig,
    pub ui: UiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceConfig {
    pub name: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ControlConfig {
    pub enabled: bool,
    pub bind: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiConfig {
    pub bind: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SunshineConfig {
    pub manage: bool,
    pub host: String,
    pub port: u16,
    pub username: String,
    /// Optional override, normally detected from `vainfo`.
    pub encoder: String,
    /// Optional render node override, for example `/dev/dri/renderD128`.
    pub adapter_name: String,
    /// Capture backend written to sunshine.conf.
    pub capture: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    /// Passed to `krfb-virtualmonitor --name`; the KDE output is `Virtual-<name>`.
    pub virtual_name: String,
    /// Optional physical output to mirror or extend from. Empty means automatic.
    pub physical_output: String,
    /// right | left | top | bottom
    pub extend_position: String,
    /// 0 lets the daemon pick a free port.
    pub vnc_port: u16,
    /// Time to wait for KWin to publish the virtual output.
    pub output_timeout_secs: u64,
    /// Restore the previous KScreen layout when a session stops.
    pub restore_on_stop: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    pub default_on_mirror: bool,
    pub default_on_extend: bool,
    pub default_on_headless: bool,
    /// Reserved for v1.1: mute local speakers while streaming.
    pub silence_host: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamingConfig {
    pub fps: u32,
    pub bitrate_kbps: u32,
    /// auto | H.264 | HEVC | AV1
    pub codec: String,
    /// Empty means use the client/host resolution detected at runtime.
    pub resolution: String,
    pub absolute_mouse: bool,
    pub capture_system_keys: String,
    pub keep_awake: bool,
    pub performance_overlay: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PairingConfig {
    pub auto_approve: bool,
    pub approval_timeout_secs: u64,
    pub max_pending: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub close_to_tray: bool,
    pub start_minimized: bool,
    pub tray_enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            device: DeviceConfig::default(),
            control: ControlConfig::default(),
            api: ApiConfig::default(),
            sunshine: SunshineConfig::default(),
            display: DisplayConfig::default(),
            audio: AudioConfig::default(),
            streaming: StreamingConfig::default(),
            pairing: PairingConfig::default(),
            ui: UiConfig::default(),
        }
    }
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            name: hostname(),
            id: random_hex(16),
        }
    }
}

impl Default for ControlConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bind: "0.0.0.0".to_string(),
            port: 47811,
        }
    }
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".to_string(),
            port: 47812,
        }
    }
}

impl Default for SunshineConfig {
    fn default() -> Self {
        Self {
            manage: true,
            host: "127.0.0.1".to_string(),
            port: 47990,
            username: "kdefusion".to_string(),
            encoder: "vaapi".to_string(),
            adapter_name: String::new(),
            capture: "kwin".to_string(),
        }
    }
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            virtual_name: "KDEFusion".to_string(),
            physical_output: String::new(),
            extend_position: "right".to_string(),
            vnc_port: 0,
            output_timeout_secs: 25,
            restore_on_stop: true,
        }
    }
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            default_on_mirror: true,
            default_on_extend: false,
            default_on_headless: true,
            silence_host: false,
        }
    }
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            fps: 60,
            bitrate_kbps: 40_000,
            codec: "auto".to_string(),
            resolution: String::new(),
            absolute_mouse: true,
            capture_system_keys: "fullscreen".to_string(),
            keep_awake: true,
            performance_overlay: false,
        }
    }
}

impl Default for PairingConfig {
    fn default() -> Self {
        Self {
            auto_approve: false,
            approval_timeout_secs: 120,
            max_pending: 32,
        }
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            close_to_tray: true,
            start_minimized: false,
            tray_enabled: false,
        }
    }
}

impl Config {
    pub fn output_name(&self) -> String {
        format!("Virtual-{}", self.display.virtual_name)
    }

    pub fn default_audio_for(&self, mode: &str) -> bool {
        match mode {
            "extend" => self.audio.default_on_extend,
            "headless" => self.audio.default_on_headless,
            _ => self.audio.default_on_mirror,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.device.name.trim().is_empty() {
            bail!("device name must not be empty");
        }
        if self.device.id.trim().is_empty() {
            bail!("device id must not be empty");
        }
        if self.control.port < 1024 {
            bail!("control port must be 1024 or higher");
        }
        if self.api.port < 1024 {
            bail!("API port must be 1024 or higher");
        }
        if self.control.port == self.api.port {
            bail!("control port and API port must differ");
        }
        if self.display.virtual_name.trim().is_empty() {
            bail!("virtual output name must not be empty");
        }
        if !self
            .display
            .virtual_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            bail!("virtual output name may only contain letters, digits, '-' and '_'");
        }
        if !matches!(
            self.display.extend_position.as_str(),
            "right" | "left" | "top" | "bottom"
        ) {
            bail!("extend_position must be right, left, top or bottom");
        }
        if !(10..=480).contains(&self.streaming.fps) {
            bail!("fps must be between 10 and 480");
        }
        if !(500..=500_000).contains(&self.streaming.bitrate_kbps) {
            bail!("bitrate must be between 500 and 500000 Kbps");
        }
        if !matches!(
            self.streaming.codec.as_str(),
            "auto" | "H.264" | "HEVC" | "AV1"
        ) {
            bail!("codec must be auto, H.264, HEVC or AV1");
        }
        Ok(())
    }
}

/// Thread-safe configuration holder.  Reads clone the small config struct;
/// writes validate and persist before swapping the in-memory value.
pub struct ConfigStore {
    path: PathBuf,
    inner: RwLock<Config>,
}

impl ConfigStore {
    pub fn load() -> Result<Self> {
        Self::load_from(config_dir().join(CONFIG_FILE))
    }

    pub fn load_from(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create config directory {}", parent.display()))?;
        }
        let mut config = if path.exists() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("read config {}", path.display()))?;
            toml::from_str::<Config>(&text)
                .with_context(|| format!("parse config {}", path.display()))?
        } else {
            Config::default()
        };
        let mut dirty = !path.exists();
        if config.device.id.trim().is_empty() {
            config.device.id = random_hex(16);
            dirty = true;
        }
        if config.device.name.trim().is_empty() {
            config.device.name = hostname();
            dirty = true;
        }
        config.validate().context("invalid configuration")?;
        let store = Self {
            path: path.clone(),
            inner: RwLock::new(config),
        };
        if dirty {
            store.save()?;
        }
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self) -> Config {
        self.inner.read().expect("config lock poisoned").clone()
    }

    pub fn set(&self, config: Config) -> Result<()> {
        config.validate()?;
        let text = toml::to_string_pretty(&config).context("serialize configuration")?;
        atomic_write(&self.path, text.as_bytes(), Some(0o600))?;
        *self.inner.write().expect("config lock poisoned") = config;
        Ok(())
    }

    pub fn update<F>(&self, f: F) -> Result<Config>
    where
        F: FnOnce(&mut Config),
    {
        let mut config = self.get();
        f(&mut config);
        self.set(config.clone())?;
        Ok(config)
    }

    /// Persist the current value, used after small automatic fixups.
    pub fn save(&self) -> Result<()> {
        let config = self.get();
        let text = toml::to_string_pretty(&config).context("serialize configuration")?;
        atomic_write(&self.path, text.as_bytes(), Some(0o600))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        let cfg = Config::default();
        cfg.validate().unwrap();
        assert_eq!(cfg.output_name(), "Virtual-KDEFusion");
        assert!(cfg.default_audio_for("mirror"));
        assert!(!cfg.default_audio_for("extend"));
    }

    #[test]
    fn config_roundtrip_preserves_values() {
        let dir = std::env::temp_dir().join(format!("kdefusion-test-{}", random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let store = ConfigStore::load_from(&path).unwrap();
        let mut cfg = store.get();
        cfg.device.name = "Test PC".to_string();
        cfg.control.port = 49001;
        store.set(cfg).unwrap();
        let reloaded = ConfigStore::load_from(&path).unwrap().get();
        assert_eq!(reloaded.device.name, "Test PC");
        assert_eq!(reloaded.control.port, 49001);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn rejects_injection_characters_in_output_name() {
        let mut cfg = Config::default();
        cfg.display.virtual_name = "foo;bar".to_string();
        assert!(cfg.validate().is_err());
    }
}
