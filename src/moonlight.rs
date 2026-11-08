//! Moonlight command line integration.
//!
//! Moonlight Qt understands `pair`, `stream`, `quit` and a stable set of long
//! options.  Arguments are always passed as separate argv entries, never
//! through a shell.

use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::util::{command_exists, command_path, home_dir};

/// Flatpak application id used when Moonlight is not installed natively.
/// Moonlight is distributed as a Flatpak on Fedora and Debian, where the
/// distribution repositories do not carry `moonlight-qt`.
pub const FLATPAK_APP: &str = "com.moonlight_stream.Moonlight";

#[derive(Debug, Clone)]
pub struct Installation {
    pub available: bool,
    /// `native`, `flatpak` or `none`.
    pub kind: &'static str,
    pub detail: String,
}

/// How Moonlight is available on this machine, if at all.
pub fn installation() -> Installation {
    if let Some(path) = command_path("moonlight") {
        return Installation {
            available: true,
            kind: "native",
            detail: path.display().to_string(),
        };
    }
    if flatpak_installed() {
        return Installation {
            available: true,
            kind: "flatpak",
            detail: format!("Flatpak {FLATPAK_APP}"),
        };
    }
    Installation {
        available: false,
        kind: "none",
        detail: "not found (native binary or Flatpak)".into(),
    }
}

fn flatpak_installed() -> bool {
    command_exists("flatpak")
        && std::process::Command::new("flatpak")
            .args(["info", FLATPAK_APP])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
}

/// Build the command prefix used for every Moonlight invocation.  A native
/// install is preferred; otherwise the Flatpak build is launched through
/// `flatpak run`, which forwards the same CLI arguments.
fn base_command() -> Option<Command> {
    if command_exists("moonlight") {
        return Some(Command::new("moonlight"));
    }
    if flatpak_installed() {
        let mut command = Command::new("flatpak");
        command.args(["run", FLATPAK_APP]);
        return Some(command);
    }
    None
}

fn missing_message() -> String {
    "Moonlight is not installed. Install moonlight-qt (Arch), the Flatpak \
     com.moonlight_stream.Moonlight (Fedora/Debian), or run Diagnostics for instructions."
        .to_string()
}

#[derive(Debug, Clone)]
pub struct MoonlightSpec {
    pub host: String,
    pub app: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    /// auto | H.264 | HEVC | AV1
    pub codec: String,
    pub audio: bool,
    pub display_mode: String,
    pub absolute_mouse: bool,
    pub capture_system_keys: String,
    pub keep_awake: bool,
    pub performance_overlay: bool,
}

impl MoonlightSpec {
    pub fn validate(&self) -> Result<()> {
        if self.host.trim().is_empty() || self.host.starts_with('-') {
            bail!("invalid Moonlight host");
        }
        if self.host.chars().any(|c| c.is_whitespace()) {
            bail!("Moonlight host must not contain whitespace");
        }
        if self.app.trim().is_empty() || self.app.len() > 128 {
            bail!("invalid Moonlight app name");
        }
        if self.width < 320 || self.height < 200 {
            bail!("invalid Moonlight resolution");
        }
        if !(10..=480).contains(&self.fps) {
            bail!("invalid Moonlight FPS");
        }
        if !(500..=500_000).contains(&self.bitrate_kbps) {
            bail!("invalid Moonlight bitrate");
        }
        if !matches!(self.codec.as_str(), "auto" | "H.264" | "HEVC" | "AV1") {
            bail!("invalid Moonlight codec");
        }
        if !matches!(
            self.display_mode.as_str(),
            "fullscreen" | "windowed" | "borderless"
        ) {
            bail!("invalid Moonlight display mode");
        }
        if !matches!(
            self.capture_system_keys.as_str(),
            "never" | "fullscreen" | "always"
        ) {
            bail!("invalid Moonlight capture-system-keys mode");
        }
        Ok(())
    }

    pub fn args(&self) -> Vec<String> {
        let mut args = vec![
            "stream".to_string(),
            self.host.clone(),
            self.app.clone(),
            "--display-mode".to_string(),
            self.display_mode.clone(),
            "--video-decoder".to_string(),
            "hardware".to_string(),
            "--resolution".to_string(),
            format!("{}x{}", self.width, self.height),
            "--fps".to_string(),
            self.fps.to_string(),
            "--bitrate".to_string(),
            self.bitrate_kbps.to_string(),
            "--video-codec".to_string(),
            self.codec.clone(),
            "--capture-system-keys".to_string(),
            self.capture_system_keys.clone(),
            "--quit-after".to_string(),
        ];
        args.push(if self.absolute_mouse {
            "--absolute-mouse".to_string()
        } else {
            "--no-absolute-mouse".to_string()
        });
        if self.audio {
            args.push("--audio-config".to_string());
            args.push("stereo".to_string());
        }
        args.push(if self.keep_awake {
            "--keep-awake".to_string()
        } else {
            "--no-keep-awake".to_string()
        });
        if self.performance_overlay {
            args.push("--performance-overlay".to_string());
        }
        args
    }
}

pub struct MoonlightProcess {
    child: Child,
    pub host: String,
    pub app: String,
    pub pid: u32,
}

impl MoonlightProcess {
    pub async fn launch(spec: &MoonlightSpec) -> Result<Self> {
        spec.validate()?;
        let mut command = base_command().ok_or_else(|| anyhow::anyhow!(missing_message()))?;
        let args = spec.args();
        tracing::info!("launching Moonlight: {}", args.join(" "));
        let mut child = command
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true)
            .spawn()
            .context("start moonlight")?;
        let pid = child.id().unwrap_or(0);
        if let Some(stdout) = child.stdout.take() {
            spawn_log_reader(stdout, "moonlight stdout");
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_log_reader(stderr, "moonlight stderr");
        }
        Ok(Self {
            child,
            host: spec.host.clone(),
            app: spec.app.clone(),
            pid,
        })
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub async fn stop(mut self) {
        tracing::info!("stopping Moonlight for {}", self.host);
        // Ask Moonlight to close the session cleanly first.  The CLI can open a
        // modal dialog and never exit (same behaviour as `pair`), so give it its
        // own process group and tear it down after a short grace period.
        if let Some(mut command) = base_command() {
            if let Ok(mut child) = command
                .args(["quit", &self.host])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0)
                .kill_on_drop(true)
                .spawn()
            {
                let quit_pid = child.id().unwrap_or(0);
                if tokio::time::timeout(Duration::from_secs(3), child.wait())
                    .await
                    .is_err()
                {
                    kill_process_group(quit_pid);
                    let _ = tokio::time::timeout(Duration::from_secs(1), child.wait()).await;
                }
            }
        }
        if matches!(self.child.try_wait(), Ok(None)) {
            // A fullscreen Flatpak stream nests `moonlight` under one or more
            // `bwrap` wrappers, so killing only the direct child would orphan
            // the actual stream.  The child leads its own process group; kill
            // the whole group.
            kill_process_group(self.pid);
            let _ = self.child.start_kill();
        }
        let _ = tokio::time::timeout(Duration::from_secs(3), self.child.wait()).await;
        // Already reaped: prevent `Drop` from signalling this PID again (it may
        // have been reused by an unrelated process group).
        self.pid = 0;
    }
}

impl Drop for MoonlightProcess {
    fn drop(&mut self) {
        kill_process_group(self.pid);
        let _ = self.child.start_kill();
    }
}

fn spawn_log_reader<R>(reader: R, label: &'static str)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if !line.trim().is_empty() {
                tracing::debug!("{label}: {line}");
            }
        }
    });
}

/// Directory Moonlight uses for its QSettings file under the platform config
/// directory.  The same name is used inside the Flatpak sandbox.
const CONFIG_DIR_NAME: &str = "Moonlight Game Streaming Project";

fn moonlight_config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(dir) = dirs::config_dir() {
        paths.push(dir.join(CONFIG_DIR_NAME).join("Moonlight.conf"));
    }
    let home = home_dir();
    paths.push(
        home.join(".config")
            .join(CONFIG_DIR_NAME)
            .join("Moonlight.conf"),
    );
    paths.push(
        home.join(".var/app")
            .join(FLATPAK_APP)
            .join("config")
            .join(CONFIG_DIR_NAME)
            .join("Moonlight.conf"),
    );
    paths.dedup();
    paths
}

/// Look for a paired host in a Moonlight config file.  QSettings stores host
/// entries as `<index>\<key>=<value>` inside `[hosts]`; a host is paired once
/// its block carries an `srvcert`.
fn config_has_paired_host(text: &str, host: &str) -> bool {
    let host = host.trim();
    if host.is_empty() {
        return false;
    }
    let mut index = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('[') || line.starts_with(';') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some((prefix, field)) = key.split_once('\\') else {
            continue;
        };
        if matches!(field, "localaddress" | "manualaddress" | "hostname")
            && value.trim() == host
        {
            index = Some(prefix.to_string());
            break;
        }
    }
    let Some(index) = index else {
        return false;
    };
    let needle = format!("{index}\\srvcert=");
    text.lines()
        .any(|line| line.trim_start().starts_with(&needle))
}

fn moonlight_has_paired_host(host: &str) -> bool {
    moonlight_config_paths().iter().any(|path| {
        std::fs::read_to_string(path)
            .map(|text| config_has_paired_host(&text, host))
            .unwrap_or(false)
    })
}

fn spawn_capture<R>(reader: R) -> Arc<Mutex<String>>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let sink = Arc::new(Mutex::new(String::new()));
    let buffer = sink.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let mut guard = buffer.lock().unwrap();
            if guard.len() < 8192 {
                guard.push_str(&line);
                guard.push('\n');
            }
        }
    });
    sink
}

fn kill_process_group(pid: u32) {
    if pid == 0 {
        return;
    }
    // The child was started in its own process group (see `process_group`),
    // so a negative PID reaches the Flatpak/bwrap wrapper and Moonlight.
    let _ = std::process::Command::new("kill")
        .args(["-KILL", "--", &format!("-{pid}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

pub async fn pair(host: &str, pin: &str) -> Result<String> {
    let mut command = base_command().ok_or_else(|| anyhow::anyhow!(missing_message()))?;
    if host.trim().is_empty() || host.starts_with('-') {
        bail!("invalid pairing host");
    }
    if pin.len() != 4 || !pin.chars().all(|c| c.is_ascii_digit()) {
        bail!("pairing PIN must be exactly four digits");
    }

    // If Moonlight already holds a certificate for this host there is nothing
    // to do.  This also short-circuits a re-pair after a failed session.
    if moonlight_has_paired_host(host) {
        tracing::info!("Moonlight already holds a certificate for {host}");
        return Ok("already paired".to_string());
    }

    tracing::info!("pairing Moonlight with {host} using a supplied PIN");
    let mut child = command
        .args(["pair", host, "--pin", pin])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
        .context("start moonlight pair")?;
    let pid = child.id().unwrap_or(0);
    let output = child.stderr.take().map(spawn_capture);

    // The CLI never exits after a successful pair: it opens a "pairing
    // completed" dialog and waits for a click.  Waiting for the process would
    // always time out, so use the server certificate written to Moonlight's
    // config as the success signal and stop the GUI ourselves.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
    loop {
        if moonlight_has_paired_host(host) {
            kill_process_group(pid);
            let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
            return Ok("paired".to_string());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let text = output
                    .as_ref()
                    .map(|buffer| buffer.lock().unwrap().clone())
                    .unwrap_or_default();
                if status.success() {
                    return Ok(text);
                }
                bail!("moonlight pair failed ({status}): {}", text.trim());
            }
            Ok(None) => {}
            Err(err) => bail!("cannot monitor moonlight: {err}"),
        }
        if tokio::time::Instant::now() >= deadline {
            kill_process_group(pid);
            let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
            let text = output
                .as_ref()
                .map(|buffer| buffer.lock().unwrap().clone())
                .unwrap_or_default();
            bail!("moonlight pair timed out: {}", text.trim());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

pub async fn quit(host: &str) -> Result<()> {
    let Some(mut command) = base_command() else {
        return Ok(());
    };
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        command
            .args(["quit", host])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status(),
    )
    .await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> MoonlightSpec {
        MoonlightSpec {
            host: "192.168.1.9".into(),
            app: "Desktop".into(),
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_kbps: 40_000,
            codec: "HEVC".into(),
            audio: true,
            display_mode: "fullscreen".into(),
            absolute_mouse: true,
            capture_system_keys: "fullscreen".into(),
            keep_awake: true,
            performance_overlay: false,
        }
    }

    #[test]
    fn builds_expected_arguments() {
        let args = spec().args();
        assert_eq!(args[0], "stream");
        assert_eq!(args[1], "192.168.1.9");
        assert_eq!(args[2], "Desktop");
        assert!(args.contains(&"--resolution".to_string()));
        assert!(args.contains(&"1920x1080".to_string()));
        assert!(args.contains(&"--absolute-mouse".to_string()));
        assert!(args.contains(&"--audio-config".to_string()));
        assert!(args.contains(&"stereo".to_string()));
        assert!(args.contains(&"--quit-after".to_string()));
        assert!(!args.iter().any(|a| a == "--no-audio-config"));
    }

    #[test]
    fn audio_off_omits_audio_config() {
        let mut s = spec();
        s.audio = false;
        assert!(!s.args().contains(&"--audio-config".to_string()));
    }

    #[test]
    fn rejects_option_injection() {
        let mut s = spec();
        s.host = "--help".into();
        assert!(s.validate().is_err());
        s.host = "host name".into();
        assert!(s.validate().is_err());
    }

    #[test]
    fn detects_paired_host_in_config() {
        let config = "\
[General]
certificate=\"x\"
[hosts]
1\\hostname=My-Laptop
1\\localaddress=192.168.1.12
1\\manualaddress=192.168.1.12
1\\srvcert=@ByteArray(abc)
2\\hostname=Other
2\\manualaddress=192.168.1.9
";
        assert!(config_has_paired_host(config, "192.168.1.12"));
        assert!(config_has_paired_host(config, "My-Laptop"));
        // Present but without a certificate: not paired yet.
        assert!(!config_has_paired_host(config, "192.168.1.9"));
        assert!(!config_has_paired_host(config, "10.0.0.1"));
        assert!(!config_has_paired_host(config, ""));
    }

    #[test]
    fn validates_pin() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert!(rt.block_on(pair("127.0.0.1", "12")).is_err());
        assert!(rt.block_on(pair("127.0.0.1", "abcd")).is_err());
    }
}
