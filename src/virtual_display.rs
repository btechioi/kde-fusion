//! Lifecycle of the KDE virtual output.
//!
//! `krfb-virtualmonitor` creates a real KWin output plus a password-protected
//! VNC listener.  We only need the output; Sunshine captures it through KWin.
//! The VNC listener gets a random 8 character password and an ephemeral port.

use anyhow::{bail, Context, Result};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::kscreen::{format_scale, Kscreen};
use crate::util::{command_exists, find_free_port, is_wayland, random_hex};

#[derive(Debug, Clone)]
pub struct VirtualDisplaySpec {
    /// Passed as `--name`; KWin exposes it as `Virtual-<name>`.
    pub name: String,
    pub logical_width: u32,
    pub logical_height: u32,
    pub scale: f64,
    pub vnc_port: u16,
    pub timeout: Duration,
}

impl VirtualDisplaySpec {
    pub fn new(
        name: impl Into<String>,
        logical_width: u32,
        logical_height: u32,
        scale: f64,
    ) -> Result<Self> {
        let name = name.into();
        if logical_width < 320 || logical_height < 240 {
            bail!("virtual display resolution is too small");
        }
        if !(0.5..=4.0).contains(&scale) {
            bail!("virtual display scale must be between 0.5 and 4.0");
        }
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            bail!("virtual display name contains unsupported characters");
        }
        Ok(Self {
            name,
            logical_width,
            logical_height,
            scale,
            vnc_port: 0,
            timeout: Duration::from_secs(25),
        })
    }

    pub fn expected_output_name(&self) -> String {
        format!("Virtual-{}", self.name)
    }

    /// KWin screencast/stream pixel size.
    pub fn pixel_size(&self) -> (u32, u32) {
        (
            ((self.logical_width as f64) * self.scale).round() as u32,
            ((self.logical_height as f64) * self.scale).round() as u32,
        )
    }
}

pub struct VirtualDisplay {
    child: Child,
    pub pid: u32,
    pub output_name: String,
    pub spec: VirtualDisplaySpec,
    pub vnc_port: u16,
}

impl VirtualDisplay {
    pub async fn start(spec: VirtualDisplaySpec, kscreen: &Kscreen) -> Result<Self> {
        if !is_wayland() {
            bail!("virtual outputs require a KDE Wayland session (XDG_SESSION_TYPE=wayland)");
        }
        if !command_exists("krfb-virtualmonitor") {
            bail!("krfb-virtualmonitor is missing. Install the 'krfb' package, then try again.");
        }

        let vnc_port = if spec.vnc_port == 0 {
            find_free_port()?
        } else {
            spec.vnc_port
        };
        // RFB authentication truncates passwords to 8 bytes.  Eight random
        // alphanumeric characters still give ~47 bits for the LAN listener.
        let password: String = (0..8)
            .map(|_| {
                const ALPHABET: &[u8] =
                    b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
                char::from(ALPHABET[fastrand::usize(..ALPHABET.len())])
            })
            .collect();

        tracing::info!(
            "creating virtual output {} at {}x{} scale {} (VNC port {})",
            spec.expected_output_name(),
            spec.logical_width,
            spec.logical_height,
            format_scale(spec.scale),
            vnc_port
        );

        let mut child = Command::new("krfb-virtualmonitor")
            .args([
                "--resolution",
                &format!("{}x{}", spec.logical_width, spec.logical_height),
                "--name",
                &spec.name,
                "--password",
                &password,
                "--port",
                &vnc_port.to_string(),
                "--scale",
                &format_scale(spec.scale),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("start krfb-virtualmonitor")?;

        let pid = child.id().unwrap_or(0);
        let errors = Arc::new(Mutex::new(String::new()));
        if let Some(stdout) = child.stdout.take() {
            spawn_log_reader(stdout, "krfb-virtualmonitor stdout", None);
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_log_reader(stderr, "krfb-virtualmonitor stderr", Some(errors.clone()));
        }

        let deadline = tokio::time::Instant::now() + spec.timeout;
        loop {
            if let Some(status) = child.try_wait().context("poll virtual monitor")? {
                let detail = errors.lock().map(|s| s.clone()).unwrap_or_default();
                bail!(
                    "krfb-virtualmonitor exited with {status} while creating the virtual output. {}",
                    detail.trim()
                );
            }
            match kscreen.list().await {
                Ok(snapshot) => {
                    if let Some(output) = snapshot.find_virtual(&spec.name) {
                        let output_name = output.name.clone();
                        tracing::info!(
                            "virtual output {} is available ({}x{} logical, {:.2} Hz)",
                            output_name,
                            output.logical_width,
                            output.logical_height,
                            output.current_mode().map(|m| m.refresh).unwrap_or(0.0)
                        );
                        return Ok(Self {
                            child,
                            pid,
                            output_name,
                            spec,
                            vnc_port,
                        });
                    }
                }
                Err(err) => tracing::debug!("waiting for KScreen: {err:#}"),
            }
            if tokio::time::Instant::now() >= deadline {
                let _ = child.start_kill();
                let detail = errors.lock().map(|s| s.clone()).unwrap_or_default();
                bail!(
                    "timed out waiting for {} to appear in KScreen. {}",
                    spec.expected_output_name(),
                    detail.trim()
                );
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }

    pub async fn stop(mut self) {
        tracing::info!("removing virtual output {}", self.output_name);
        let _ = self.child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(4), self.child.wait()).await;
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

fn spawn_log_reader<R>(reader: R, label: &'static str, sink: Option<Arc<Mutex<String>>>)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if !line.trim().is_empty() {
                tracing::debug!("{label}: {line}");
            }
            if let Some(sink) = &sink {
                if let Ok(mut text) = sink.lock() {
                    if text.len() < 4096 {
                        text.push_str(&line);
                        text.push('\n');
                    }
                }
            }
        }
    });
}

/// Kill a virtual monitor left behind by a previous daemon crash.  The PID is
/// verified against the process command line first so a recycled PID is never
/// killed by accident.
pub fn kill_stale_virtual(pid: u32, expected_name: &str) -> Result<bool> {
    if pid == 0 {
        return Ok(false);
    }
    let cmdline_path = format!("/proc/{pid}/cmdline");
    let cmdline = match std::fs::read(&cmdline_path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).replace('\0', " "),
        Err(_) => return Ok(false),
    };
    if !cmdline.contains("krfb-virtualmonitor") || !cmdline.contains(expected_name) {
        return Ok(false);
    }
    tracing::warn!("killing stale virtual monitor process {pid}");
    let _ = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status();
    Ok(true)
}

/// Unique marker used in daemon log messages and diagnostics.
pub fn marker() -> String {
    random_hex(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_computes_pixel_size() {
        let spec = VirtualDisplaySpec::new("KDEFusion", 1920, 1080, 1.0).unwrap();
        assert_eq!(spec.pixel_size(), (1920, 1080));
        assert_eq!(spec.expected_output_name(), "Virtual-KDEFusion");

        let spec = VirtualDisplaySpec::new("KDEFusion", 1822, 1024, 0.75).unwrap();
        assert_eq!(spec.pixel_size(), (1367, 768));
    }

    #[test]
    fn spec_rejects_bad_input() {
        assert!(VirtualDisplaySpec::new("bad;name", 1920, 1080, 1.0).is_err());
        assert!(VirtualDisplaySpec::new("x", 100, 100, 1.0).is_err());
        assert!(VirtualDisplaySpec::new("x", 1920, 1080, 9.0).is_err());
    }

    #[test]
    fn stale_pid_check_rejects_non_krfb() {
        assert!(!kill_stale_virtual(std::process::id(), "KDEFusion").unwrap());
        assert!(!kill_stale_virtual(u32::MAX, "KDEFusion").unwrap());
    }
}
