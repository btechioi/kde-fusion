//! Sunshine integration: configuration, first-run bootstrap, HTTPS API client,
//! pairing, encoder detection and process supervision.
//!
//! Sunshine's web API is HTTPS with a self-signed certificate.  We deliberately
//! use the system `curl` binary instead of pulling a full TLS stack into the
//! daemon.  `curl` is a hard dependency of the Sunshine package on CachyOS.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

use crate::config::Config;
use crate::secrets::SecretStore;
use crate::util::{
    atomic_write, command_path, curl_config_escape, first_nonempty, poll_until, random_token, run,
};

pub const DESKTOP_APP: &str = "Desktop";
pub const SERVICE_UNITS: [&str; 2] = [
    "sunshine.service",
    "app-dev.lizardbyte.app.Sunshine.service",
];

#[derive(Debug, Clone)]
pub struct SunshinePaths {
    pub dir: PathBuf,
    pub config: PathBuf,
    pub apps: PathBuf,
    pub permission_user: PathBuf,
}

pub fn paths() -> SunshinePaths {
    let dir = dirs::config_dir()
        .unwrap_or_else(|| crate::util::home_dir().join(".config"))
        .join("sunshine");
    let permission_user = dirs::data_dir()
        .unwrap_or_else(|| crate::util::home_dir().join(".local/share"))
        .join("applications")
        .join("dev.lizardbyte.app.Sunshine.kwin.desktop");
    SunshinePaths {
        config: dir.join("sunshine.conf"),
        apps: dir.join("apps.json"),
        dir,
        permission_user,
    }
}

/// KWin only allows `zkde_screencast_unstable_v1` for executables with a
/// matching desktop file.  The Sunshine package usually installs one, but if
/// it does not (or if Sunshine lives in `/usr/local/bin`) we create a user-level
/// permission file.  This avoids a portal dialog and works without root.
pub fn ensure_kwin_permission() -> Result<PathBuf> {
    let system_candidates = [
        PathBuf::from("/usr/share/applications/dev.lizardbyte.app.Sunshine.kwin.desktop"),
        PathBuf::from("/usr/share/applications/sunshine-kwin.desktop"),
        PathBuf::from("/usr/local/share/applications/dev.lizardbyte.app.Sunshine.kwin.desktop"),
    ];
    if let Some(found) = system_candidates.iter().find(|p| p.exists()) {
        return Ok(found.clone());
    }
    let p = paths();
    if p.permission_user.exists() {
        return Ok(p.permission_user);
    }
    let exec = command_path("sunshine")
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "/usr/bin/sunshine".to_string());
    let content = format!(
        "[Desktop Entry]\n\
         Exec={exec}\n\
         X-KDE-Wayland-Interfaces=zkde_screencast_unstable_v1\n\
         Type=Application\n\
         Name=Sunshine KWin Wayland Permission\n\
         Comment=Allows Sunshine to use KWin's native screencast protocol\n\
         NoDisplay=true\n"
    );
    atomic_write(&p.permission_user, content.as_bytes(), Some(0o644))?;
    tracing::info!(
        "installed KWin screencast permission at {}",
        p.permission_user.display()
    );
    Ok(p.permission_user)
}

#[derive(Debug, Clone)]
pub struct IniFile {
    pub path: PathBuf,
    lines: Vec<String>,
}

impl IniFile {
    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let lines = if path.exists() {
            std::fs::read_to_string(&path)
                .with_context(|| format!("read {}", path.display()))?
                .lines()
                .map(ToString::to_string)
                .collect()
        } else {
            Vec::new()
        };
        Ok(Self { path, lines })
    }

    pub fn get(&self, key: &str) -> Option<String> {
        for line in self.lines.iter().rev() {
            let trimmed = line.trim();
            if trimmed.starts_with('#') || trimmed.starts_with(';') {
                continue;
            }
            if let Some((k, v)) = trimmed.split_once('=') {
                if k.trim() == key {
                    return Some(v.trim().trim_matches('"').to_string());
                }
            }
        }
        None
    }

    pub fn set(&mut self, key: &str, value: &str) {
        let new_line = format!("{key} = {value}");
        for line in self.lines.iter_mut() {
            let trimmed = line.trim_start();
            if trimmed.starts_with('#') || trimmed.starts_with(';') {
                continue;
            }
            if let Some((k, _)) = trimmed.split_once('=') {
                if k.trim() == key {
                    *line = new_line;
                    return;
                }
            }
        }
        self.lines.push(new_line);
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let backup = self.path.with_extension("conf.kdefusion.bak");
        if self.path.exists() && !backup.exists() {
            std::fs::copy(&self.path, &backup).ok();
        }
        let mut text = self.lines.join("\n");
        text.push('\n');
        atomic_write(&self.path, text.as_bytes(), Some(0o600))
    }
}

pub fn write_config(config: &Config, audio: bool) -> Result<()> {
    let p = paths();
    std::fs::create_dir_all(&p.dir).context("create Sunshine config directory")?;
    let mut ini = IniFile::read(&p.config)?;
    ini.set("capture", &config.sunshine.capture);
    ini.set("output_name", &config.output_name());
    if !config.sunshine.encoder.trim().is_empty() {
        ini.set("encoder", config.sunshine.encoder.trim());
    }
    if !config.sunshine.adapter_name.trim().is_empty() {
        ini.set("adapter_name", config.sunshine.adapter_name.trim());
    }
    ini.set("stream_audio", if audio { "enabled" } else { "disabled" });
    // Keep the web UI local.  The daemon talks to it over loopback.
    ini.set("origin_web_ui_allowed", "pc");
    ini.save()?;
    ensure_desktop_app(&p.apps)?;
    ensure_kwin_permission()?;
    Ok(())
}

pub fn ensure_desktop_app(path: &Path) -> Result<()> {
    let mut root: Value = if path.exists() {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("read Sunshine apps {}", path.display()))?;
        serde_json::from_str(&text).unwrap_or_else(|_| json!({ "apps": [] }))
    } else {
        json!({ "apps": [] })
    };
    if !root.get("apps").map(Value::is_array).unwrap_or(false) {
        root["apps"] = json!([]);
    }
    let apps = root["apps"].as_array_mut().expect("apps array");
    if !apps
        .iter()
        .any(|app| app.get("name").and_then(Value::as_str) == Some(DESKTOP_APP))
    {
        apps.push(json!({
            "name": DESKTOP_APP,
            "cmd": "",
            "working-dir": "",
            "image-path": "",
            "auto-detach": true,
            "wait-all": true,
            "exit-timeout": 5,
            "exclude-global-prep-cmd": false,
            "elevated": false
        }));
        let backup = path.with_extension("json.kdefusion.bak");
        if path.exists() && !backup.exists() {
            std::fs::copy(path, &backup).ok();
        }
        atomic_write(
            path,
            serde_json::to_vec_pretty(&root)?.as_slice(),
            Some(0o600),
        )?;
        tracing::info!("added Desktop app to {}", path.display());
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinRequest {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub address: String,
}

#[derive(Debug, Clone)]
pub struct HttpResult {
    pub status: u16,
    pub body: String,
}

impl HttpResult {
    pub fn json(&self) -> Result<Value> {
        serde_json::from_str(&self.body)
            .with_context(|| format!("parse Sunshine response: {}", truncate(&self.body, 300)))
    }
}

#[derive(Debug, Clone)]
pub struct SunshineClient {
    host: String,
    port: u16,
    username: String,
    password: String,
}

impl SunshineClient {
    pub fn new(
        host: impl Into<String>,
        port: u16,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            host: host.into(),
            port,
            username: username.into(),
            password: password.into(),
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("https://{}:{}{}", self.host, self.port, path)
    }

    async fn http(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        max_time: u64,
    ) -> Result<HttpResult> {
        let mut args: Vec<String> = vec![
            "-sk".into(),
            "--connect-timeout".into(),
            "3".into(),
            "--max-time".into(),
            max_time.to_string(),
            "-K".into(),
            "-".into(),
            "-X".into(),
            method.into(),
            "--url".into(),
            self.url(path),
            "--write-out".into(),
            "\\n%{http_code}".into(),
        ];
        if body.is_some() {
            args.push("-H".into());
            args.push("Content-Type: application/json".into());
        }
        if let Some(body) = body {
            args.push("--data-binary".into());
            args.push(serde_json::to_string(body)?);
        }

        let mut child = Command::new("curl")
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("start curl (required for Sunshine API access)")?;
        if !self.username.is_empty() && !self.password.is_empty() {
            let cfg = format!(
                "user = \"{}:{}\"\n",
                curl_config_escape(&self.username),
                curl_config_escape(&self.password)
            );
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(cfg.as_bytes()).await?;
            }
        } else if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(b"");
        }
        let output = child.wait_with_output().await.context("wait for curl")?;
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        let (body, code) = match text.rsplit_once('\n') {
            Some((body, code)) => (body.to_string(), code.trim().parse::<u16>().unwrap_or(0)),
            None => (text, 0),
        };
        if code == 0 {
            bail!(
                "Sunshine API is not reachable at https://{}:{}. {}",
                self.host,
                self.port,
                first_nonempty(
                    &String::from_utf8_lossy(&output.stderr),
                    "Is Sunshine running?"
                )
            );
        }
        Ok(HttpResult { status: code, body })
    }

    pub async fn get_config(&self) -> Result<HttpResult> {
        self.http("GET", "/api/config", None, 10).await
    }

    pub async fn get_apps(&self) -> Result<HttpResult> {
        self.http("GET", "/api/apps", None, 10).await
    }

    pub async fn pending_pairings(&self) -> Result<Vec<PinRequest>> {
        let result = self.http("GET", "/api/pin", None, 10).await?;
        if result.status != 200 {
            bail!(
                "Sunshine /api/pin returned HTTP {}: {}",
                result.status,
                truncate(&result.body, 200)
            );
        }
        let value = result.json()?;
        let pairings = value
            .get("pairings")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::new();
        for item in pairings {
            out.push(PinRequest {
                id: item
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                name: item
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                address: item
                    .get("address")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            });
        }
        Ok(out)
    }

    pub async fn submit_pin(&self, pairing_id: &str, pin: &str, name: &str) -> Result<bool> {
        let body = json!({
            "pairing_id": pairing_id,
            "pin": pin,
            "name": name,
        });
        let result = self.http("POST", "/api/pin", Some(&body), 45).await?;
        if result.status != 200 {
            bail!(
                "Sunshine rejected the PIN (HTTP {}): {}",
                result.status,
                truncate(&result.body, 300)
            );
        }
        let value = result.json()?;
        Ok(value
            .get("status")
            .and_then(Value::as_bool)
            .unwrap_or(false))
    }

    /// Poll for Moonlight's pairing request and approve it with the PIN that
    /// Moonlight was launched with.  This is the two-sided automation that
    /// removes the web UI step.
    pub async fn poll_and_submit_pin(
        &self,
        pin: &str,
        client_name: &str,
        timeout: Duration,
    ) -> Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut last_error = None;
        loop {
            if tokio::time::Instant::now() >= deadline {
                bail!(
                    "timed out waiting for Moonlight to request pairing. {}",
                    last_error.unwrap_or_default()
                );
            }
            match self.pending_pairings().await {
                Ok(pairings) if !pairings.is_empty() => {
                    for pairing in pairings {
                        tracing::info!(
                            "approving Moonlight pairing request from {} ({})",
                            pairing.name,
                            pairing.address
                        );
                        match self.submit_pin(&pairing.id, pin, client_name).await {
                            Ok(true) => return Ok(()),
                            Ok(false) => {
                                last_error = Some("Sunshine refused the PIN".to_string());
                            }
                            Err(err) => last_error = Some(format!("{err:#}")),
                        }
                    }
                }
                Ok(_) => {}
                Err(err) => last_error = Some(format!("{err:#}")),
            }
            tokio::time::sleep(Duration::from_millis(350)).await;
        }
    }

    pub async fn health(&self) -> bool {
        matches!(self.get_config().await, Ok(r) if r.status == 200)
    }
}

pub async fn probe_http(host: &str, port: u16, path: &str) -> Result<u16> {
    let output = Command::new("curl")
        .args([
            "-sk",
            "--connect-timeout",
            "2",
            "--max-time",
            "4",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "--url",
            &format!("https://{host}:{port}{path}"),
        ])
        .output()
        .await
        .context("run curl probe")?;
    let code = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u16>()
        .unwrap_or(0);
    Ok(code)
}

pub async fn wait_ready(host: &str, port: u16, timeout: Duration) -> Result<()> {
    let code = poll_until(timeout, Duration::from_millis(300), || async {
        match probe_http(host, port, "/api/config").await {
            Ok(code) if code == 200 || code == 401 => Some(code),
            _ => None,
        }
    })
    .await;
    match code {
        Some(_) => Ok(()),
        None => bail!(
            "Sunshine did not become ready at https://{host}:{port} within {} seconds",
            timeout.as_secs()
        ),
    }
}

/// Ensure we have working admin credentials.  On a fresh Sunshine install the
/// first-run API allows creating them; afterwards the password comes from
/// KWallet/Secret Service.
pub async fn ensure_admin(config: &Config, secrets: &SecretStore) -> Result<SunshineClient> {
    let host = config.sunshine.host.clone();
    let port = config.sunshine.port;
    wait_ready(&host, port, Duration::from_secs(20)).await?;

    if let Some(password) = secrets.sunshine_password() {
        let client = SunshineClient::new(&host, port, &config.sunshine.username, &password);
        if client.health().await {
            return Ok(client);
        }
        tracing::warn!("stored Sunshine password was rejected");
    }

    // Try to bootstrap only when no credentials exist.  If Sunshine already
    // has an admin user, this returns 401/403 and we ask the user to log in.
    let generated = random_token();
    let body = json!({
        "currentUsername": "",
        "currentPassword": "",
        "newUsername": config.sunshine.username,
        "newPassword": generated,
        "confirmNewPassword": generated,
    });
    let anonymous = SunshineClient::new(&host, port, "", "");
    let result = anonymous
        .http("POST", "/api/password", Some(&body), 15)
        .await?;
    if result.status == 200 {
        secrets.set_sunshine_password(&generated)?;
        tracing::info!("created Sunshine admin credentials for KDE Fusion");
        let client = SunshineClient::new(&host, port, &config.sunshine.username, &generated);
        if client.health().await {
            return Ok(client);
        }
        bail!("Sunshine accepted bootstrap but the new credentials do not work");
    }
    bail!(
        "Sunshine already has admin credentials. Add them in KDE Fusion Settings \
         (they are stored in KWallet), then retry. Server said HTTP {}: {}",
        result.status,
        truncate(&result.body, 300)
    )
}

/// Check whether the provided credentials work, for the Settings page.
pub async fn check_credentials(config: &Config, username: &str, password: &str) -> Result<bool> {
    let client = SunshineClient::new(
        &config.sunshine.host,
        config.sunshine.port,
        username,
        password,
    );
    Ok(client.health().await)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EncoderSupport {
    pub h264: bool,
    pub hevc: bool,
    pub av1: bool,
    pub raw: String,
}

pub async fn detect_encoder_support() -> EncoderSupport {
    if !crate::util::command_exists("vainfo") {
        return EncoderSupport::default();
    }
    let out = match run("vainfo", &[]).await {
        Ok(out) => out,
        Err(_) => return EncoderSupport::default(),
    };
    let text = format!("{}\n{}", out.stdout, out.stderr);
    let mut support = EncoderSupport {
        raw: text.clone(),
        ..Default::default()
    };
    // vainfo prints profile blocks followed by entrypoint lines.  A small state
    // machine avoids matching a decode entrypoint as an encode capability.
    let mut profile = String::new();
    for line in text.lines() {
        if let Some(idx) = line.find("VAProfile") {
            profile = line[idx..].to_string();
        }
        if line.contains("VAEntrypointEncSlice") || line.contains("VAEntrypointEncSliceLP") {
            if profile.contains("H264") {
                support.h264 = true;
            }
            if profile.contains("HEVC") {
                support.hevc = true;
            }
            if profile.contains("AV1") {
                support.av1 = true;
            }
        }
    }
    support
}

pub fn select_codec(requested: &str, support: &EncoderSupport) -> String {
    match requested {
        "HEVC" if support.hevc => "HEVC".to_string(),
        "HEVC" => {
            tracing::warn!("HEVC encode is not available; falling back to H.264");
            "H.264".to_string()
        }
        "AV1" if support.av1 => "AV1".to_string(),
        "AV1" => {
            tracing::warn!("AV1 encode is not available; falling back to H.264");
            "H.264".to_string()
        }
        "H.264" => "H.264".to_string(),
        _ if support.hevc => "HEVC".to_string(),
        _ => "H.264".to_string(),
    }
}

pub async fn active_user_service() -> Option<String> {
    for unit in SERVICE_UNITS {
        if let Ok(out) = run(
            "systemctl",
            &[
                "--user".to_string(),
                "is-active".to_string(),
                unit.to_string(),
            ],
        )
        .await
        {
            if out.stdout.trim() == "active" {
                return Some(unit.to_string());
            }
        }
    }
    None
}

pub struct SunshineRuntime {
    pub started_by_us: bool,
    pub unit: Option<String>,
    child: Option<Child>,
    audio: bool,
}

impl SunshineRuntime {
    pub async fn start(config: &Config, audio: bool) -> Result<Self> {
        if !config.sunshine.manage {
            let unit = active_user_service()
                .await
                .context("Sunshine management is disabled and no active user service was found")?;
            wait_ready(
                &config.sunshine.host,
                config.sunshine.port,
                Duration::from_secs(10),
            )
            .await?;
            return Ok(Self {
                started_by_us: false,
                unit: Some(unit),
                child: None,
                audio,
            });
        }

        if !crate::util::command_exists("sunshine") {
            bail!("Sunshine is not installed. Run Diagnostics for distribution-specific install instructions.");
        }

        let old_audio = IniFile::read(paths().config)
            .ok()
            .and_then(|ini| ini.get("stream_audio"));
        write_config(config, audio)?;

        if let Some(unit) = active_user_service().await {
            let audio_changed =
                old_audio.as_deref() != Some(if audio { "enabled" } else { "disabled" });
            if audio_changed {
                tracing::info!("restarting {unit} so the audio setting takes effect");
                let _ = run(
                    "systemctl",
                    &["--user".to_string(), "restart".to_string(), unit.clone()],
                )
                .await;
            }
            wait_ready(
                &config.sunshine.host,
                config.sunshine.port,
                Duration::from_secs(15),
            )
            .await?;
            return Ok(Self {
                started_by_us: false,
                unit: Some(unit),
                child: None,
                audio,
            });
        }

        // Run Sunshine as a child of the user session so KWin screencast
        // permission and PipeWire access are available.
        let config_path = paths().config;
        let mut child = Command::new("sunshine")
            .arg(&config_path)
            .env("XDG_SESSION_TYPE", "wayland")
            .env("XDG_CURRENT_DESKTOP", "KDE")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("start sunshine")?;
        if let Some(stdout) = child.stdout.take() {
            spawn_log_reader(stdout, "sunshine stdout", None);
        }
        let stderr_sink = Arc::new(Mutex::new(String::new()));
        if let Some(stderr) = child.stderr.take() {
            spawn_log_reader(stderr, "sunshine stderr", Some(stderr_sink.clone()));
        }
        tracing::info!("started Sunshine as a user process");
        if let Err(err) = wait_ready(
            &config.sunshine.host,
            config.sunshine.port,
            Duration::from_secs(25),
        )
        .await
        {
            let detail = stderr_sink.lock().map(|s| s.clone()).unwrap_or_default();
            let _ = child.start_kill();
            return Err(err).context(format!("Sunshine stderr:\n{}", detail.trim()));
        }
        Ok(Self {
            started_by_us: true,
            unit: None,
            child: Some(child),
            audio,
        })
    }

    pub fn audio(&self) -> bool {
        self.audio
    }

    pub async fn client(&self, config: &Config, secrets: &SecretStore) -> Result<SunshineClient> {
        let _ = self;
        ensure_admin(config, secrets).await
    }

    pub async fn stop(mut self) {
        if self.started_by_us {
            if let Some(mut child) = self.child.take() {
                tracing::info!("stopping Sunshine process started by KDE Fusion");
                let _ = child.start_kill();
                let _ = tokio::time::timeout(Duration::from_secs(4), child.wait()).await;
            }
        }
    }
}

impl Drop for SunshineRuntime {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.start_kill();
        }
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
                    if text.len() < 8192 {
                        text.push_str(&line);
                        text.push('\n');
                    }
                }
            }
        }
    });
}

/// Format an address for Moonlight and Sunshine.
pub fn stream_host(config: &Config) -> String {
    if config.sunshine.host == "127.0.0.1" || config.sunshine.host == "localhost" {
        crate::util::local_ipv4()
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| config.sunshine.host.clone())
    } else {
        config.sunshine.host.clone()
    }
}

pub fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ini_updates_without_duplicates() {
        let dir =
            std::env::temp_dir().join(format!("kdefusion-ini-{}", crate::util::random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sunshine.conf");
        std::fs::write(&path, "# comment\ncapture = portal\nmin_log_level = info\n").unwrap();
        let mut ini = IniFile::read(&path).unwrap();
        ini.set("capture", "kwin");
        ini.set("stream_audio", "enabled");
        ini.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("capture =").count(), 1);
        assert!(text.contains("capture = kwin"));
        assert!(text.contains("stream_audio = enabled"));
        let ini = IniFile::read(&path).unwrap();
        assert_eq!(ini.get("capture").as_deref(), Some("kwin"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn desktop_app_is_added_once() {
        let dir =
            std::env::temp_dir().join(format!("kdefusion-apps-{}", crate::util::random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("apps.json");
        ensure_desktop_app(&path).unwrap();
        ensure_desktop_app(&path).unwrap();
        let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let apps = value["apps"].as_array().unwrap();
        assert_eq!(apps.iter().filter(|a| a["name"] == DESKTOP_APP).count(), 1);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn codec_selection_falls_back() {
        let support = EncoderSupport {
            h264: true,
            hevc: false,
            ..Default::default()
        };
        assert_eq!(select_codec("auto", &support), "H.264");
        assert_eq!(select_codec("HEVC", &support), "H.264");
        let support = EncoderSupport {
            h264: true,
            hevc: true,
            ..Default::default()
        };
        assert_eq!(select_codec("auto", &support), "HEVC");
    }

    #[test]
    fn stream_host_is_never_loopback() {
        let mut config = Config::default();
        config.sunshine.host = "127.0.0.1".into();
        let host = stream_host(&config);
        assert_ne!(host, "127.0.0.1");
        assert_ne!(host, "localhost");
    }

    #[test]
    fn test_ini_backup_path() {
        let p = paths();
        assert!(p
            .config
            .to_string_lossy()
            .ends_with("sunshine/sunshine.conf"));
    }
}
