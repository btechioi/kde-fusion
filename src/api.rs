//! Local HTTP/JSON API used by the Kirigami GUI and `kdefusionctl`.
//!
//! It binds to loopback only and has no authentication: any process running as
//! the desktop user can already control that user's session.  The LAN control
//! channel is the authenticated one.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

use crate::config::Config;
use crate::control::{ControlContext, PublicPendingPair};
use crate::diagnostics;
use crate::notify;
use crate::state::PeerRecord;
use crate::sunshine;
use crate::types::SessionRequest;
use crate::util::{command_exists, now_unix, random_pair_code};

const MAX_HEADER: usize = 64 * 1024;
const MAX_BODY: usize = 1024 * 1024;

pub async fn serve(listener: TcpListener, ctx: Arc<ControlContext>) -> Result<()> {
    loop {
        let (stream, addr) = match listener.accept().await {
            Ok(pair) => pair,
            Err(err) => {
                tracing::warn!("API accept failed: {err}");
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
        };
        let ctx = ctx.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_connection(stream, addr, ctx).await {
                tracing::debug!("API connection from {addr} ended: {err:#}");
            }
        });
    }
}

async fn handle_connection(
    stream: TcpStream,
    _addr: SocketAddr,
    ctx: Arc<ControlContext>,
) -> Result<()> {
    let (request, mut writer) = match read_request(stream).await {
        Ok(pair) => pair,
        Err(err) => {
            tracing::debug!("malformed API request: {err:#}");
            return Ok(());
        }
    };
    let response = match route(&request, &ctx).await {
        Ok(response) => response,
        Err(err) => ApiResponse::error(400, format!("{err:#}")),
    };
    writer
        .write_all(response.render(&request.method).as_bytes())
        .await?;
    writer.flush().await.ok();
    Ok(())
}

struct HttpRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

impl HttpRequest {
    fn json<T: for<'de> Deserialize<'de>>(&self) -> Result<T> {
        serde_json::from_slice(&self.body).context("invalid JSON body")
    }
}

async fn read_request(stream: TcpStream) -> Result<(HttpRequest, tokio::net::tcp::OwnedWriteHalf)> {
    let (read_half, write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await? == 0 {
        bail!("empty request");
    }
    let mut parts = request_line.trim().split_whitespace();
    let method = parts.next().unwrap_or_default().to_ascii_uppercase();
    let path = parts.next().unwrap_or("/").to_string();
    let mut headers = HashMap::new();
    let mut header_bytes = 0;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }
        header_bytes += n;
        if header_bytes > MAX_HEADER {
            bail!("request headers too large");
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    let content_length = headers
        .get("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    if content_length > MAX_BODY {
        bail!("request body too large");
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).await?;
    }
    Ok((HttpRequest { method, path, body }, write_half))
}

struct ApiResponse {
    status: u16,
    body: Value,
}

impl ApiResponse {
    fn ok(body: Value) -> Self {
        Self { status: 200, body }
    }

    fn accepted(body: Value) -> Self {
        Self { status: 202, body }
    }

    fn error(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            body: json!({ "ok": false, "error": message.into() }),
        }
    }

    fn render(&self, method: &str) -> String {
        if method == "OPTIONS" {
            return "HTTP/1.1 204 No Content\r\n\
                    Access-Control-Allow-Origin: *\r\n\
                    Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
                    Access-Control-Allow-Headers: Content-Type\r\n\
                    Content-Length: 0\r\n\
                    Connection: close\r\n\r\n"
                .to_string();
        }
        let reason = match self.status {
            200 => "OK",
            202 => "Accepted",
            204 => "No Content",
            400 => "Bad Request",
            401 => "Unauthorized",
            404 => "Not Found",
            409 => "Conflict",
            500 => "Internal Server Error",
            _ => "Error",
        };
        let body = serde_json::to_string(&self.body).unwrap_or_else(|_| "{}".to_string());
        format!(
            "HTTP/1.1 {} {}\r\n\
             Content-Type: application/json; charset=utf-8\r\n\
             Content-Length: {}\r\n\
             Cache-Control: no-store\r\n\
             Access-Control-Allow-Origin: *\r\n\
             Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
             Access-Control-Allow-Headers: Content-Type\r\n\
             Connection: close\r\n\r\n{}",
            self.status,
            reason,
            body.len(),
            body
        )
    }
}

async fn route(request: &HttpRequest, ctx: &Arc<ControlContext>) -> Result<ApiResponse> {
    let path = request.path.split('?').next().unwrap_or("/");
    match (request.method.as_str(), path) {
        ("OPTIONS", _) => Ok(ApiResponse {
            status: 204,
            body: Value::Null,
        }),
        ("GET", "/api/health") => Ok(ApiResponse::ok(json!({
            "ok": true,
            "version": crate::VERSION,
            "pid": std::process::id(),
        }))),
        ("GET", "/api/status") => {
            ctx.ui.heartbeat();
            let (open_request, page) = ctx.ui.snapshot();
            let config = ctx.config.get();
            let session = ctx.session.status().await;
            Ok(ApiResponse::ok(json!({
                "ok": true,
                "version": crate::VERSION,
                "device": { "name": config.device.name, "id": config.device.id },
                "session": session,
                "ui": {
                    "open_request": open_request,
                    "page": page,
                    "close_to_tray": config.ui.close_to_tray,
                },
                "host_active": ctx.session.host_active().await,
                "viewer_active": ctx.session.viewer_active().await,
                "paired_peers": ctx.state.peers().len(),
                "discovered_peers": ctx.discovery.len(),
                "default_audio": {
                    "mirror": config.audio.default_on_mirror,
                    "extend": config.audio.default_on_extend,
                    "headless": config.audio.default_on_headless,
                },
                "sunshine": {
                    "host": config.sunshine.host,
                    "port": config.sunshine.port,
                    "manage": config.sunshine.manage,
                }
            })))
        }
        ("GET", "/api/peers") => Ok(ApiResponse::ok(json!({ "peers": peers(ctx) }))),
        ("POST", "/api/peers/pair") => pair_peer(request, ctx).await,
        ("POST", "/api/peers/remove") => {
            #[derive(Deserialize)]
            struct Body {
                peer_id: String,
            }
            let body: Body = request.json()?;
            ctx.state.remove_peer(&body.peer_id)?;
            ctx.secrets.delete_peer_token(&body.peer_id)?;
            ctx.pairing.remove_device(&body.peer_id);
            Ok(ApiResponse::ok(json!({ "ok": true })))
        }
        ("POST", "/api/peers/sunshine-reset") => {
            #[derive(Deserialize)]
            struct Body {
                peer_id: String,
            }
            let body: Body = request.json()?;
            ctx.state.set_sunshine_paired(&body.peer_id, false)?;
            Ok(ApiResponse::ok(json!({ "ok": true })))
        }
        ("GET", "/api/pairing/pending") => {
            let pending: Vec<PublicPendingPair> = ctx.pairing.list();
            Ok(ApiResponse::ok(json!({ "pending": pending })))
        }
        ("POST", "/api/pairing/approve") => {
            #[derive(Deserialize)]
            struct Body {
                request_id: String,
            }
            let body: Body = request.json()?;
            let approved = ctx.approve_pairing(&body.request_id).await?;
            Ok(ApiResponse::ok(json!({ "ok": true, "paired": approved })))
        }
        ("POST", "/api/pairing/reject") => {
            #[derive(Deserialize)]
            struct Body {
                request_id: String,
            }
            let body: Body = request.json()?;
            let rejected = ctx.reject_pairing(&body.request_id)?;
            Ok(ApiResponse::ok(json!({ "ok": true, "rejected": rejected })))
        }
        ("POST", "/api/session/start") => {
            let req: SessionRequest = request.json()?;
            ctx.session.request_start(req).await?;
            Ok(ApiResponse::accepted(json!({
                "ok": true,
                "accepted": true,
                "detail": "session start requested"
            })))
        }
        ("POST", "/api/session/stop") => {
            #[derive(Deserialize, Default)]
            #[serde(default)]
            struct Body {
                reason: Option<String>,
            }
            let body: Body = request.json().unwrap_or_default();
            ctx.session
                .request_stop(body.reason.as_deref().unwrap_or("user requested"))
                .await?;
            Ok(ApiResponse::accepted(json!({
                "ok": true,
                "accepted": true
            })))
        }
        ("GET", "/api/session/status") => Ok(ApiResponse::ok(
            json!({ "ok": true, "status": ctx.session.status().await }),
        )),
        ("GET", "/api/config") => {
            let config = ctx.config.get();
            Ok(ApiResponse::ok(json!({
                "ok": true,
                "config": config,
                "paths": {
                    "config": ctx.config.path().display().to_string(),
                    "state": ctx.state.path().display().to_string(),
                    "sunshine_config": sunshine::paths().config.display().to_string()
                }
            })))
        }
        ("POST", "/api/config") => update_config(request, ctx),
        ("POST", "/api/sunshine/credentials") => set_credentials(request, ctx).await,
        ("GET", "/api/diagnostics") => {
            let config = ctx.config.get();
            let report =
                diagnostics::run_report(&config, &ctx.state, &ctx.secrets, &ctx.discovery).await;
            Ok(ApiResponse::ok(json!({ "ok": true, "report": report })))
        }
        ("GET", "/api/install") => Ok(ApiResponse::ok(json!({
            "ok": true,
            "install": ctx.installer.overview().await,
        }))),
        ("POST", "/api/install/dependencies") => match ctx.installer.start().await {
            Ok(()) => Ok(ApiResponse::accepted(json!({
                "ok": true,
                "install": ctx.installer.overview().await,
            }))),
            Err(err) => Ok(ApiResponse::error(409, format!("{err:#}"))),
        },
        ("GET", "/api/logs") => Ok(ApiResponse::ok(json!({
            "ok": true,
            "hint": "journalctl --user -u kdefusiond -n 200 --no-pager",
            "available": command_exists("journalctl"),
        }))),
        ("POST", "/api/sunshine/prepare") => {
            let config = ctx.config.get();
            let audio = config.default_audio_for("mirror");
            sunshine::write_config(&config, audio)?;
            Ok(ApiResponse::ok(json!({ "ok": true })))
        }
        ("GET", "/api/os") => Ok(ApiResponse::ok(json!({
            "ok": true,
            "session_type": std::env::var("XDG_SESSION_TYPE").unwrap_or_default(),
            "desktop": std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
            "wayland": crate::util::is_wayland(),
            "hostname": crate::util::hostname(),
            "addresses": crate::util::lan_ipv4_addresses(),
        }))),
        ("POST", "/api/notify") => {
            #[derive(Deserialize)]
            struct Body {
                title: String,
                body: String,
            }
            let body: Body = request.json()?;
            notify::send(body.title, body.body).await;
            Ok(ApiResponse::ok(json!({ "ok": true })))
        }
        _ => Ok(ApiResponse::error(404, "unknown endpoint")),
    }
}

fn peers(ctx: &ControlContext) -> Vec<Value> {
    let discovered = ctx.discovery.list();
    let mut by_id: HashMap<String, Value> = HashMap::new();
    for peer in ctx.state.peers() {
        let found = discovered.iter().find(|d| d.device_id == peer.device_id);
        let mut addresses = peer.addresses.clone();
        if let Some(found) = found {
            for address in &found.addresses {
                if !addresses.contains(address) {
                    addresses.push(address.clone());
                }
            }
        }
        let online = found.is_some();
        let port = found.map(|f| f.port).unwrap_or(peer.control_port);
        by_id.insert(
            peer.device_id.clone(),
            json!({
                "device_id": peer.device_id,
                "name": peer.name,
                "addresses": addresses,
                "port": port,
                "online": online,
                "paired": true,
                "sunshine_paired": peer.sunshine_paired,
                "platform": peer.platform,
                "version": peer.version,
                "last_seen": peer.last_seen,
                "last_status": peer.last_status,
            }),
        );
    }
    for peer in discovered {
        by_id.entry(peer.device_id.clone()).or_insert_with(|| {
            json!({
                "device_id": peer.device_id,
                "name": peer.name,
                "addresses": peer.addresses,
                "port": peer.port,
                "online": true,
                "paired": false,
                "sunshine_paired": false,
                "platform": "",
                "version": peer.version,
                "last_seen": peer.last_seen,
                "last_status": "Discovered",
            })
        });
    }
    let mut peers: Vec<Value> = by_id.into_values().collect();
    peers.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .cmp(&b["name"].as_str().unwrap_or("").to_lowercase())
    });
    peers
}

async fn pair_peer(request: &HttpRequest, ctx: &Arc<ControlContext>) -> Result<ApiResponse> {
    #[derive(Deserialize)]
    struct Body {
        peer_id: String,
        #[serde(default)]
        code: String,
    }
    let body: Body = request.json()?;
    let config = ctx.config.get();
    if ctx.state.peer(&body.peer_id).is_some() {
        bail!("peer is already paired; remove it first to pair again");
    }
    let discovered = ctx
        .discovery
        .get(&body.peer_id)
        .with_context(|| "peer is not currently visible over mDNS")?;
    let addresses = discovered.addresses.clone();
    let addr = crate::control::pick_address(&addresses, discovered.port)
        .with_context(|| "peer has no IPv4 address")?;
    let code = if body.code.is_empty() {
        random_pair_code()
    } else if body.code.len() == 6 && body.code.chars().all(|c| c.is_ascii_digit()) {
        body.code
    } else {
        bail!("pairing code must be six digits");
    };
    let own_id = config.device.id.clone();
    let own_name = config.device.name.clone();
    let timeout = Duration::from_secs(config.pairing.approval_timeout_secs.max(60));
    let peer_name = discovered.name.clone();
    let peer_platform = String::new();
    let peer_version = discovered.version.clone();
    let ctx = ctx.clone();
    let code_for_task = code.clone();
    let peer_id = body.peer_id.clone();
    tokio::spawn(async move {
        match crate::control::pair(addr, &own_id, &own_name, &code_for_task, timeout).await {
            Ok(token) => {
                let now = now_unix();
                let record = PeerRecord {
                    device_id: peer_id.clone(),
                    name: peer_name.clone(),
                    addresses: vec![addr.ip().to_string()],
                    control_port: addr.port(),
                    platform: peer_platform,
                    version: peer_version,
                    paired_at: now,
                    last_seen: now,
                    sunshine_paired: false,
                    last_status: "Paired".to_string(),
                };
                if let Err(err) = ctx.secrets.set_peer_token(&peer_id, &token) {
                    tracing::error!("could not store peer token: {err:#}");
                }
                if let Err(err) = ctx.state.upsert_peer(record) {
                    tracing::error!("could not store peer: {err:#}");
                }
                notify::send(
                    "Pairing complete",
                    format!("{peer_name} is now paired with this PC"),
                )
                .await;
            }
            Err(err) => {
                tracing::warn!("pairing with {peer_name} failed: {err:#}");
                notify::error("Pairing failed", format!("{err:#}")).await;
            }
        }
    });
    Ok(ApiResponse::accepted(json!({
        "ok": true,
        "peer_id": body.peer_id,
        "peer_name": discovered.name,
        "code": code,
        "status": "pending",
        "detail": "Approve the request on the other PC"
    })))
}

fn update_config(request: &HttpRequest, ctx: &Arc<ControlContext>) -> Result<ApiResponse> {
    let patch: Value = request.json()?;
    let current = ctx.config.get();
    let original_id = current.device.id.clone();
    let mut value = serde_json::to_value(&current)?;
    merge_json(&mut value, patch);
    let mut updated: Config = serde_json::from_value(value).context("invalid configuration")?;
    updated.device.id = original_id;
    updated.validate()?;
    let restart_required = updated.api.port != current.api.port
        || updated.api.bind != current.api.bind
        || updated.control.port != current.control.port
        || updated.control.bind != current.control.bind
        || updated.device.name != current.device.name;
    ctx.config.set(updated.clone())?;
    Ok(ApiResponse::ok(json!({
        "ok": true,
        "config": updated,
        "restart_required": restart_required,
        "detail": if restart_required {
            "Some changes take effect after restarting kdefusiond."
        } else {
            "Configuration saved."
        }
    })))
}

fn merge_json(dst: &mut Value, src: Value) {
    match (dst, src) {
        (Value::Object(dst), Value::Object(src)) => {
            for (key, value) in src {
                merge_json(dst.entry(key).or_insert(Value::Null), value);
            }
        }
        (dst, src) => *dst = src,
    }
}

async fn set_credentials(request: &HttpRequest, ctx: &Arc<ControlContext>) -> Result<ApiResponse> {
    #[derive(Deserialize)]
    struct Body {
        username: String,
        password: String,
    }
    let body: Body = request.json()?;
    if body.username.trim().is_empty() || body.password.is_empty() {
        bail!("username and password are required");
    }
    let config = ctx.config.get();
    if !sunshine::check_credentials(&config, &body.username, &body.password).await? {
        return Ok(ApiResponse::error(
            401,
            "Sunshine rejected those credentials. Open the Sunshine web UI and verify them.",
        ));
    }
    ctx.secrets.set_sunshine_password(&body.password)?;
    ctx.config.update(|cfg| {
        cfg.sunshine.username = body.username.clone();
    })?;
    Ok(ApiResponse::ok(json!({
        "ok": true,
        "detail": "Sunshine credentials verified and saved in the Secret Service"
    })))
}

/// Used by the daemon startup path to log a compact one-line API banner.
pub fn banner(addr: SocketAddr) -> String {
    format!("local API: http://{addr}/api/status")
}

/// Test helper: not used by the daemon, but keeps the HTTP parser honest.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_nested_config_patches() {
        let mut base = json!({ "a": 1, "nested": { "x": 1, "y": 2 } });
        merge_json(&mut base, json!({ "nested": { "y": 3 }, "b": true }));
        assert_eq!(base["a"], 1);
        assert_eq!(base["nested"]["x"], 1);
        assert_eq!(base["nested"]["y"], 3);
        assert_eq!(base["b"], true);
    }

    #[tokio::test]
    async fn http_parser_reads_request() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = tokio::spawn(async move {
            let mut stream = TcpStream::connect(addr).await.unwrap();
            stream
                .write_all(
                    b"POST /api/session/start HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\n\r\n{}",
                )
                .await
                .unwrap();
        });
        let (stream, _) = listener.accept().await.unwrap();
        let (request, _writer) = read_request(stream).await.unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/api/session/start");
        assert_eq!(request.body, b"{}");
        client.await.unwrap();
    }
}
