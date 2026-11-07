//! Moonlight command line integration.
//!
//! Moonlight Qt understands `pair`, `stream`, `quit` and a stable set of long
//! options.  Arguments are always passed as separate argv entries, never
//! through a shell.

use anyhow::{bail, Context, Result};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::util::{command_exists, command_path};

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
        // Ask Moonlight to close the session cleanly first.
        if let Some(mut command) = base_command() {
            let _ = tokio::time::timeout(
                Duration::from_secs(3),
                command
                    .args(["quit", &self.host])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status(),
            )
            .await;
        }
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.start_kill();
        }
        let _ = tokio::time::timeout(Duration::from_secs(3), self.child.wait()).await;
    }
}

impl Drop for MoonlightProcess {
    fn drop(&mut self) {
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

pub async fn pair(host: &str, pin: &str) -> Result<String> {
    let mut command = base_command().ok_or_else(|| anyhow::anyhow!(missing_message()))?;
    if host.trim().is_empty() || host.starts_with('-') {
        bail!("invalid pairing host");
    }
    if pin.len() != 4 || !pin.chars().all(|c| c.is_ascii_digit()) {
        bail!("pairing PIN must be exactly four digits");
    }
    tracing::info!("pairing Moonlight with {host} using a supplied PIN");
    let output = tokio::time::timeout(
        Duration::from_secs(45),
        command
            .args(["pair", host, "--pin", pin])
            .stdin(Stdio::null())
            .output(),
    )
    .await
    .context("moonlight pair timed out")?
    .context("start moonlight pair")?;
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !output.status.success() {
        bail!("moonlight pair failed ({}): {}", output.status, text.trim());
    }
    Ok(text)
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
    fn validates_pin() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert!(rt.block_on(pair("127.0.0.1", "12")).is_err());
        assert!(rt.block_on(pair("127.0.0.1", "abcd")).is_err());
    }
}
