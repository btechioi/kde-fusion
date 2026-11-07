//! `kdefusionctl`: command line control for the local KDE Fusion daemon.
//!
//! The daemon exposes a loopback-only HTTP/JSON API.  We talk to it with the
//! system `curl` binary so the CLI does not need a TLS stack (the daemon is
//! plain HTTP, but avoiding another runtime dependency keeps the package small).

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

const DEFAULT_API: &str = "http://127.0.0.1:47812";

#[derive(Parser, Debug)]
#[command(
    name = "kdefusionctl",
    version = kdefusion::VERSION,
    about = "Control the KDE Fusion daemon",
    after_help = "Examples:\n  kdefusionctl status\n  kdefusionctl peers\n  kdefusionctl pair <peer-id>\n  kdefusionctl start --peer <peer-id> --mode extend --audio on"
)]
struct Cli {
    /// Daemon API base URL (default: http://127.0.0.1:47812).
    #[arg(long, global = true, env = "KDEFUSION_API", default_value = DEFAULT_API)]
    api: String,
    /// Print raw JSON instead of a human friendly summary.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Show daemon and session status.
    Status,
    /// List paired and discovered peers.
    Peers,
    /// Ask another PC to pair with this one.
    Pair {
        /// Device id shown by `kdefusionctl peers`.
        peer_id: String,
        /// Six digit code shown on the other PC. Generated when omitted.
        #[arg(long)]
        code: Option<String>,
    },
    /// Show pairing requests waiting for approval.
    Pending,
    /// Approve a pairing request.
    Approve { request_id: String },
    /// Reject a pairing request.
    Reject { request_id: String },
    /// Remove a paired peer (revokes its token).
    Remove { peer_id: String },
    /// Start a streaming session.
    Start(StartArgs),
    /// Stop the active session.
    Stop {
        #[arg(long, default_value = "user requested")]
        reason: String,
    },
    /// Run environment and dependency diagnostics.
    Doctor,
    /// Print the effective configuration.
    Config,
    /// Change a single configuration value, for example `device.name`.
    #[command(name = "set")]
    SetConfig {
        /// Dotted key, for example `streaming.fps` or `audio.default_on_extend`.
        key: String,
        /// New value. Strings and booleans are inferred automatically.
        value: String,
    },
    /// Sunshine helper commands.
    #[command(subcommand)]
    Sunshine(SunshineCommands),
    /// Wait until the session reaches a final state.
    Wait {
        /// idle | streaming | failed
        #[arg(long, default_value = "streaming")]
        phase: String,
        #[arg(long, default_value_t = 90)]
        timeout_secs: u64,
    },
}

#[derive(Args, Debug)]
struct StartArgs {
    /// Device id of the PC that will view this desktop.
    #[arg(long)]
    peer: String,
    /// mirror, extend or headless.
    #[arg(long, default_value = "mirror")]
    mode: String,
    /// left, right, top or bottom (extend mode only).
    #[arg(long, default_value = "right")]
    layout: String,
    /// on or off. Defaults to the mode default in the configuration.
    #[arg(long)]
    audio: Option<String>,
    /// Optional resolution override, for example 1920x1080.
    #[arg(long)]
    resolution: Option<String>,
    #[arg(long)]
    fps: Option<u32>,
    #[arg(long)]
    bitrate: Option<u32>,
    /// auto, H.264, HEVC or AV1.
    #[arg(long)]
    codec: Option<String>,
}

#[derive(Subcommand, Debug)]
enum SunshineCommands {
    /// Write the Sunshine configuration and Desktop app entry.
    Prepare,
    /// Check and store Sunshine admin credentials.
    Credentials {
        #[arg(long)]
        username: String,
        #[arg(long)]
        password: String,
    },
}

struct Api {
    base: String,
}

impl Api {
    fn new(base: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
        }
    }

    fn request(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
        let url = format!("{}{}", self.base, path);
        let mut args: Vec<String> = vec![
            "-sS".into(),
            "--connect-timeout".into(),
            "3".into(),
            "--max-time".into(),
            "120".into(),
            "-X".into(),
            method.into(),
            "--url".into(),
            url,
            "--write-out".into(),
            "\\n%{http_code}".into(),
        ];
        if body.is_some() {
            args.push("-H".into());
            args.push("Content-Type: application/json".into());
        }
        let mut child = Command::new("curl")
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("start curl (is curl installed?)")?;
        if let Some(body) = body {
            let text = serde_json::to_vec(body)?;
            child
                .stdin
                .as_mut()
                .context("curl stdin")?
                .write_all(&text)?;
        }
        let output = child.wait_with_output().context("wait for curl")?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let (body, code) = match stdout.rsplit_once('\n') {
            Some((body, code)) => (body.to_string(), code.trim().parse::<u16>().unwrap_or(0)),
            None => (stdout, 0),
        };
        if code == 0 {
            bail!(
                "the KDE Fusion daemon is not reachable at {}. Start it with: systemctl --user start kdefusiond\n{}",
                self.base,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let value: Value = serde_json::from_str(&body)
            .with_context(|| format!("daemon returned non-JSON (HTTP {code}): {body}"))?;
        if !(200..=299).contains(&code) || value.get("ok").and_then(Value::as_bool) == Some(false) {
            let message = value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("request failed");
            bail!("HTTP {code}: {message}");
        }
        Ok(value)
    }

    fn get(&self, path: &str) -> Result<Value> {
        self.request("GET", path, None)
    }

    fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.request("POST", path, Some(&body))
    }
}

fn main() {
    if let Err(err) = run() {
        eprintln!("kdefusionctl: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let api = Api::new(&cli.api);
    match cli.command {
        Commands::Status => {
            let value = api.get("/api/status")?;
            if cli.json {
                print_json(&value)?;
            } else {
                print_status(&value);
            }
        }
        Commands::Peers => {
            let value = api.get("/api/peers")?;
            if cli.json {
                print_json(&value)?;
            } else {
                print_peers(&value);
            }
        }
        Commands::Pair { peer_id, code } => {
            let mut body = json!({ "peer_id": peer_id });
            if let Some(code) = code {
                body["code"] = json!(code);
            }
            let value = api.post("/api/peers/pair", body)?;
            if cli.json {
                print_json(&value)?;
            } else {
                println!(
                    "Pairing request sent to {}.\nCode: {}\n{}",
                    value["peer_name"].as_str().unwrap_or(&peer_id),
                    value["code"].as_str().unwrap_or(""),
                    value["detail"].as_str().unwrap_or("")
                );
            }
        }
        Commands::Pending => {
            let value = api.get("/api/pairing/pending")?;
            if cli.json {
                print_json(&value)?;
            } else {
                let pending = value["pending"].as_array().cloned().unwrap_or_default();
                if pending.is_empty() {
                    println!("No pending pairing requests.");
                }
                for item in pending {
                    println!(
                        "{}  {} ({})  code {}  from {}",
                        item["id"].as_str().unwrap_or(""),
                        item["name"].as_str().unwrap_or(""),
                        item["device_id"].as_str().unwrap_or(""),
                        item["code"].as_str().unwrap_or(""),
                        item["remote_addr"].as_str().unwrap_or("")
                    );
                }
            }
        }
        Commands::Approve { request_id } => {
            let value = api.post("/api/pairing/approve", json!({ "request_id": request_id }))?;
            if cli.json {
                print_json(&value)?;
            } else {
                println!(
                    "Approved {}.",
                    value["paired"]["name"].as_str().unwrap_or("peer")
                );
            }
        }
        Commands::Reject { request_id } => {
            let value = api.post("/api/pairing/reject", json!({ "request_id": request_id }))?;
            if cli.json {
                print_json(&value)?;
            } else {
                println!("Rejected the pairing request.");
            }
        }
        Commands::Remove { peer_id } => {
            api.post("/api/peers/remove", json!({ "peer_id": peer_id }))?;
            println!("Peer removed and token revoked.");
        }
        Commands::Start(args) => {
            let audio = match args.audio.as_deref() {
                None => None,
                Some("on" | "true" | "enabled" | "yes") => Some(true),
                Some("off" | "false" | "disabled" | "no") => Some(false),
                Some(other) => bail!("--audio must be 'on' or 'off', not '{other}'"),
            };
            let body = json!({
                "peer_id": args.peer,
                "mode": args.mode,
                "layout": args.layout,
                "audio": audio,
                "resolution": args.resolution.unwrap_or_default(),
                "fps": args.fps,
                "bitrate_kbps": args.bitrate,
                "codec": args.codec,
            });
            let value = api.post("/api/session/start", body)?;
            if cli.json {
                print_json(&value)?;
            } else {
                println!(
                    "{} Watch progress with: kdefusionctl status",
                    value["detail"].as_str().unwrap_or("Session requested.")
                );
            }
        }
        Commands::Stop { reason } => {
            let value = api.post("/api/session/stop", json!({ "reason": reason }))?;
            if cli.json {
                print_json(&value)?;
            } else {
                println!("Stopping the session...");
            }
        }
        Commands::Doctor => {
            let value = api.get("/api/diagnostics")?;
            if cli.json {
                print_json(&value)?;
            } else {
                print_diagnostics(&value["report"]);
            }
        }
        Commands::Config => {
            let value = api.get("/api/config")?;
            print_json(&value["config"])?;
        }
        Commands::SetConfig { key, value } => {
            let patch = build_patch(&key, &value)?;
            let result = api.post("/api/config", patch)?;
            println!(
                "{}",
                result["detail"].as_str().unwrap_or("Configuration saved.")
            );
        }
        Commands::Sunshine(SunshineCommands::Prepare) => {
            api.post("/api/sunshine/prepare", json!({}))?;
            println!("Sunshine configuration and Desktop app entry are ready.");
        }
        Commands::Sunshine(SunshineCommands::Credentials { username, password }) => {
            let result = api.post(
                "/api/sunshine/credentials",
                json!({ "username": username, "password": password }),
            )?;
            println!("{}", result["detail"].as_str().unwrap_or("Saved."));
        }
        Commands::Wait {
            phase,
            timeout_secs,
        } => {
            let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs.max(1));
            loop {
                let value = api.get("/api/session/status")?;
                let current = value["status"]["phase"].as_str().unwrap_or("unknown");
                if current == phase {
                    if cli.json {
                        print_json(&value)?;
                    } else {
                        println!("Session reached phase '{phase}'.");
                    }
                    return Ok(());
                }
                if current == "failed" && phase != "failed" {
                    bail!(
                        "session failed: {}",
                        value["status"]["error"].as_str().unwrap_or("unknown error")
                    );
                }
                if std::time::Instant::now() >= deadline {
                    bail!("timed out waiting for phase '{phase}' (last phase: {current})");
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
    Ok(())
}

fn print_json(value: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn print_status(value: &Value) {
    let session = &value["session"];
    println!(
        "Device:   {} ({})",
        value["device"]["name"].as_str().unwrap_or("?"),
        value["device"]["id"].as_str().unwrap_or("?")
    );
    println!(
        "Version:  {}    Peers: {} paired, {} discovered",
        value["version"].as_str().unwrap_or("?"),
        value["paired_peers"].as_u64().unwrap_or(0),
        value["discovered_peers"].as_u64().unwrap_or(0)
    );
    println!("Phase:    {}", session["phase"].as_str().unwrap_or("idle"));
    if let Some(peer) = session["peer_name"].as_str() {
        if !peer.is_empty() {
            println!(
                "Peer:     {}    Mode: {}    Audio: {}    Size: {}x{}",
                peer,
                session["mode"].as_str().unwrap_or("?"),
                if session["audio"].as_bool().unwrap_or(false) {
                    "on"
                } else {
                    "off"
                },
                session["width"].as_u64().unwrap_or(0),
                session["height"].as_u64().unwrap_or(0)
            );
        }
    }
    if let Some(detail) = session["detail"].as_str() {
        if !detail.is_empty() {
            println!("Detail:   {detail}");
        }
    }
    if let Some(error) = session["error"].as_str() {
        if !error.is_empty() {
            println!("Error:    {error}");
        }
    }
}

fn print_peers(value: &Value) {
    let peers = value["peers"].as_array().cloned().unwrap_or_default();
    if peers.is_empty() {
        println!("No peers yet. Make sure KDE Fusion runs on the other PC.");
        return;
    }
    println!(
        "{:<20} {:<24} {:<8} {:<8} {}",
        "NAME", "DEVICE ID", "ONLINE", "PAIRED", "ADDRESS"
    );
    for peer in peers {
        println!(
            "{:<20} {:<24} {:<8} {:<8} {}",
            truncate(peer["name"].as_str().unwrap_or("?"), 20),
            truncate(peer["device_id"].as_str().unwrap_or("?"), 24),
            if peer["online"].as_bool().unwrap_or(false) {
                "yes"
            } else {
                "no"
            },
            if peer["paired"].as_bool().unwrap_or(false) {
                "yes"
            } else {
                "no"
            },
            peer["addresses"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default()
        );
    }
}

fn print_diagnostics(report: &Value) {
    println!(
        "KDE Fusion {} on {} / {} ({} ok, {} warnings, {} errors)",
        report["app_version"].as_str().unwrap_or("?"),
        report["desktop"].as_str().unwrap_or("?"),
        report["session_type"].as_str().unwrap_or("?"),
        report["ok"].as_u64().unwrap_or(0),
        report["warn"].as_u64().unwrap_or(0),
        report["error"].as_u64().unwrap_or(0)
    );
    for check in report["checks"].as_array().cloned().unwrap_or_default() {
        let marker = match check["status"].as_str().unwrap_or("info") {
            "ok" => "OK  ",
            "warn" => "WARN",
            "error" => "FAIL",
            _ => "INFO",
        };
        println!(
            "[{marker}] {:<28} {}",
            check["label"].as_str().unwrap_or(""),
            check["detail"].as_str().unwrap_or("")
        );
        if let Some(fix) = check["fix"].as_str() {
            if !fix.is_empty() {
                println!("       fix: {fix}");
            }
        }
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// Build a nested JSON patch from `a.b.c` + a scalar value.  The value is
/// interpreted as a boolean, number or string in that order.
fn build_patch(key: &str, raw: &str) -> Result<Value> {
    if key.is_empty() || key.split('.').any(|part| part.is_empty()) {
        bail!("configuration key must look like section.option");
    }
    let value = if raw.eq_ignore_ascii_case("true") {
        json!(true)
    } else if raw.eq_ignore_ascii_case("false") {
        json!(false)
    } else if let Ok(number) = raw.parse::<i64>() {
        json!(number)
    } else if let Ok(number) = raw.parse::<f64>() {
        json!(number)
    } else {
        json!(raw)
    };
    let mut root = Value::Object(Default::default());
    let mut cursor = &mut root;
    let parts: Vec<&str> = key.split('.').collect();
    for (index, part) in parts.iter().enumerate() {
        if index + 1 == parts.len() {
            cursor[part] = value.clone();
        } else {
            cursor[part] = Value::Object(Default::default());
            cursor = cursor.get_mut(part).expect("object inserted above");
        }
    }
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patches_are_nested() {
        let patch = build_patch("streaming.fps", "120").unwrap();
        assert_eq!(patch["streaming"]["fps"], 120);
        let patch = build_patch("device.name", "Living Room").unwrap();
        assert_eq!(patch["device"]["name"], "Living Room");
        let patch = build_patch("audio.silence_host", "true").unwrap();
        assert_eq!(patch["audio"]["silence_host"], true);
        assert!(build_patch("bad..key", "x").is_err());
    }

    #[test]
    fn truncation_is_unicode_safe() {
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("äöüß", 4), "äöüß");
    }
}
