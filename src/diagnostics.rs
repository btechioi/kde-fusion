//! Dependency and environment diagnostics shown in the GUI and CLI.

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::discovery::{DiscoveredPeer, DiscoveryRegistry};
use crate::kscreen::Kscreen;
use crate::moonlight;
use crate::platform;
use crate::secrets::{Backend, SecretStore};
use crate::state::StateStore;
use crate::sunshine;
use crate::util::{command_exists, command_path, is_kde, is_wayland, now_unix, run};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Check {
    pub id: String,
    pub label: String,
    /// ok | warn | error | info
    pub status: String,
    pub detail: String,
    pub fix: String,
}

impl Check {
    fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        status: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            status: status.into(),
            detail: detail.into(),
            fix: String::new(),
        }
    }

    fn with_fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = fix.into();
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticsReport {
    pub generated_at: u64,
    pub app_version: String,
    pub distro: String,
    pub package_manager: String,
    pub session_type: String,
    pub desktop: String,
    pub wayland: bool,
    pub kde: bool,
    pub checks: Vec<Check>,
    pub ok: usize,
    pub warn: usize,
    pub error: usize,
    pub paired_peers: usize,
    pub discovered_peers: usize,
}

pub async fn run_report(
    config: &Config,
    state: &StateStore,
    secrets: &SecretStore,
    discovery: &DiscoveryRegistry,
) -> DiagnosticsReport {
    let mut checks = Vec::new();

    let session_type = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into());
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown".into());
    let wayland = is_wayland();
    let kde = is_kde();
    let distro = platform::detect();

    checks.push(
        Check::new(
            "platform",
            "Distribution",
            "info",
            format!("{}; family={}", distro.summary(), distro.family_label),
        )
        .with_fix("KDE Fusion supports the Arch, Fedora and Debian package families natively."),
    );

    checks.push(
        Check::new(
            "session",
            "KDE Plasma Wayland session",
            if wayland && kde { "ok" } else { "error" },
            format!("session={session_type}, desktop={desktop}"),
        )
        .with_fix("KDE Fusion requires a Plasma 6 Wayland session for virtual-output streaming."),
    );

    for (cmd, key, required) in [
        ("kscreen-doctor", "kscreen", true),
        ("krfb-virtualmonitor", "krfb", true),
        ("sunshine", "sunshine", true),
        ("curl", "curl", true),
        ("notify-send", "notify-send", false),
        ("secret-tool", "secret-tool", false),
        ("vainfo", "vainfo", false),
    ] {
        let present = command_exists(cmd);
        checks.push(
            Check::new(
                format!("cmd-{cmd}"),
                format!("{cmd}"),
                if present {
                    "ok"
                } else if required {
                    "error"
                } else {
                    "warn"
                },
                command_path(cmd)
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "not found in PATH".to_string()),
            )
            .with_fix(if present {
                String::new()
            } else {
                platform::install_hint(key)
            }),
        );
    }

    // Moonlight can be a native binary or the Flatpak build, which is the
    // supported installation on Fedora and Debian.
    let moonlight_install = moonlight::installation();
    checks.push(
        Check::new(
            "cmd-moonlight",
            "moonlight",
            if moonlight_install.available {
                "ok"
            } else {
                "error"
            },
            moonlight_install.detail,
        )
        .with_fix(if moonlight_install.available {
            String::new()
        } else {
            platform::install_hint("moonlight")
        }),
    );

    // Display / KScreen.
    match Kscreen::default().list().await {
        Ok(snapshot) => {
            let physical = snapshot.physical_outputs();
            let virtual_found = snapshot
                .outputs
                .iter()
                .find(|o| o.is_virtual())
                .map(|o| o.name.clone());
            checks.push(Check::new(
                "kscreen",
                "KScreen output layout",
                if physical.iter().any(|o| o.enabled) {
                    "ok"
                } else {
                    "error"
                },
                format!(
                    "{} output(s); physical: {}; virtual: {}",
                    snapshot.outputs.len(),
                    physical
                        .iter()
                        .map(|o| o.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    virtual_found.unwrap_or_else(|| "none".into())
                ),
            ));
        }
        Err(err) => checks.push(
            Check::new(
                "kscreen",
                "KScreen output layout",
                "error",
                format!("{err:#}"),
            )
            .with_fix("Install kscreen and make sure kscreen-doctor can talk to KWin."),
        ),
    }

    // Sunshine.
    let sunshine_path = command_path("sunshine")
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "not found".into());
    checks.push(Check::new(
        "sunshine-binary",
        "Sunshine binary",
        if sunshine_path == "not found" {
            "error"
        } else {
            "ok"
        },
        sunshine_path,
    ));
    let service = sunshine::active_user_service().await;
    checks.push(Check::new(
        "sunshine-service",
        "Sunshine user service",
        if service.is_some() { "ok" } else { "info" },
        service
            .clone()
            .unwrap_or_else(|| "not running; KDE Fusion will launch Sunshine for a session".into()),
    ));
    let probe = sunshine::probe_http(&config.sunshine.host, config.sunshine.port, "/api/config")
        .await
        .unwrap_or(0);
    checks.push(
        Check::new(
            "sunshine-api",
            "Sunshine API",
            match probe {
                200 => "ok",
                401 => "warn",
                0 => "info",
                _ => "warn",
            },
            format!(
                "https://{}:{} -> HTTP {}",
                config.sunshine.host, config.sunshine.port, probe
            ),
        )
        .with_fix("Start Sunshine and open its web UI once if credentials are missing."),
    );

    let paths = sunshine::paths();
    let ini = sunshine::IniFile::read(&paths.config).ok();
    let capture = ini
        .as_ref()
        .and_then(|i| i.get("capture"))
        .unwrap_or_else(|| "(not set)".into());
    let output_name = ini
        .as_ref()
        .and_then(|i| i.get("output_name"))
        .unwrap_or_else(|| "(not set)".into());
    let audio = ini
        .as_ref()
        .and_then(|i| i.get("stream_audio"))
        .unwrap_or_else(|| "(not set)".into());
    checks.push(
        Check::new(
            "sunshine-config",
            "Sunshine configuration",
            if capture == "kwin" && output_name == config.output_name() {
                "ok"
            } else {
                "warn"
            },
            format!("capture={capture}, output_name={output_name}, stream_audio={audio}"),
        )
        .with_fix("KDE Fusion writes these keys automatically when a session starts."),
    );

    let kwin_permission = sunshine::ensure_kwin_permission().await_ok();
    checks.push(Check::new(
        "kwin-permission",
        "KWin screencast permission",
        if kwin_permission.is_some() { "ok" } else { "warn" },
        kwin_permission
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "not installed".into()),
    )
    .with_fix("The daemon can create ~/.local/share/applications/dev.lizardbyte.app.Sunshine.kwin.desktop."));

    // Encoder.
    let support = sunshine::detect_encoder_support().await;
    let codec = sunshine::select_codec(&config.streaming.codec, &support);
    checks.push(
        Check::new(
            "encoder",
            "Hardware encoder",
            if support.h264 || support.hevc {
                "ok"
            } else if command_exists("vainfo") {
                "warn"
            } else {
                "warn"
            },
            format!(
                "H.264={}, HEVC={}, AV1={}; selected {codec}",
                support.h264, support.hevc, support.av1
            ),
        )
        .with_fix(platform::encoder_hint()),
    );

    // Audio.
    let audio_info = run("pactl", &["info".to_string()]).await.ok();
    let sink = audio_info
        .as_ref()
        .and_then(|out| {
            out.stdout
                .lines()
                .find(|l| l.trim_start().starts_with("Default Sink:"))
                .map(|l| l.trim().to_string())
        })
        .unwrap_or_else(|| "no default PipeWire/Pulse sink reported".into());
    checks.push(
        Check::new(
            "audio",
            "System audio",
            if sink.starts_with("Default Sink:") {
                "ok"
            } else {
                "warn"
            },
            sink,
        )
        .with_fix(platform::audio_hint()),
    );

    // Secrets.
    checks.push(Check::new(
        "secrets",
        "Secret storage",
        match secrets.backend() {
            Backend::SecretService => "ok",
            Backend::File => "warn",
        },
        match secrets.backend() {
            Backend::SecretService => String::from("KWallet / Secret Service (secret-tool)"),
            Backend::File => String::from("secret-tool unavailable; using a 0600 fallback file"),
        },
    ));

    // Peers / networking.
    let paired = state.peers();
    let discovered = discovery.list();
    checks.push(Check::new(
        "pairing",
        "Paired peers",
        if paired.is_empty() { "info" } else { "ok" },
        format!(
            "{} paired; {} discovered over mDNS",
            paired.len(),
            discovered.len()
        ),
    ));
    checks.push(Check::new(
        "control-port",
        "Control channel",
        if config.control.enabled { "ok" } else { "warn" },
        format!(
            "{}:{} (LAN-only, token-authenticated)",
            config.control.bind, config.control.port
        ),
    ));

    // GPU summary.
    if let Ok(out) = run("lspci", &[]).await {
        let gpu = out
            .stdout
            .lines()
            .filter(|l| {
                l.contains("VGA compatible controller")
                    || l.contains("Display controller")
                    || l.contains("3D controller")
            })
            .map(|l| l.split(": ").nth(1).unwrap_or(l).to_string())
            .collect::<Vec<_>>()
            .join("; ");
        if !gpu.is_empty() {
            checks.push(Check::new("gpu", "GPU", "info", gpu));
        }
    }

    if config.audio.silence_host {
        checks.push(
            Check::new(
                "silence-host",
                "Silence host speakers",
                "warn",
                "requested, but v1 keeps host audio audible",
            )
            .with_fix("Host muting via a PipeWire null sink is planned for v1.1."),
        );
    }

    let ok = checks.iter().filter(|c| c.status == "ok").count();
    let warn = checks.iter().filter(|c| c.status == "warn").count();
    let error = checks.iter().filter(|c| c.status == "error").count();
    DiagnosticsReport {
        generated_at: now_unix(),
        app_version: crate::VERSION.to_string(),
        distro: distro.summary(),
        package_manager: distro.manager.clone(),
        session_type,
        desktop,
        wayland,
        kde,
        checks,
        ok,
        warn,
        error,
        paired_peers: paired.len(),
        discovered_peers: discovered.len(),
    }
}

/// A tiny extension that turns a blocking helper into an `Option` in async
/// diagnostics without obscuring failures.
trait AwaitOk<T> {
    fn await_ok(self) -> Option<T>;
}

impl<T> AwaitOk<T> for Result<T, anyhow::Error> {
    fn await_ok(self) -> Option<T> {
        self.ok()
    }
}

pub fn discovered_json(peers: &[DiscoveredPeer]) -> serde_json::Value {
    serde_json::to_value(peers).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_serialize_for_qml() {
        let check = Check::new("x", "X", "ok", "fine").with_fix("nothing");
        let value = serde_json::to_value(check).unwrap();
        assert_eq!(value["status"], "ok");
        assert_eq!(value["fix"], "nothing");
    }
}
