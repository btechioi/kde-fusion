//! LAN control protocol.
//!
//! Transport: newline-delimited JSON over TCP.  Version 1 is LAN-only and uses
//! an explicit pairing approval + a 256-bit bearer token.  Tokens are stored in
//! the Secret Service.  The protocol deliberately exposes a fixed command set;
//! there is no shell execution.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

use crate::config::ConfigStore;
use crate::discovery::DiscoveryRegistry;
use crate::kscreen::Kscreen;
use crate::moonlight;
use crate::notify;
use crate::secrets::SecretStore;
use crate::session::SessionManager;
use crate::state::{PeerRecord, StateStore};
use crate::types::{PairState, SessionStatus, StartStream};
use crate::util::{now_unix, random_token, run};

pub const PROTO_VERSION: u32 = 1;
pub const MAX_LINE: usize = 1024 * 1024;
pub const HELLO_VERSION: &str = crate::VERSION;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    pub message: Message,
}

fn default_version() -> u32 {
    PROTO_VERSION
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    Hello {
        device_id: String,
        name: String,
        version: String,
        platform: String,
    },
    HelloAck {
        device_id: String,
        name: String,
        version: String,
    },
    PairRequest {
        device_id: String,
        name: String,
        platform: String,
        /// Six-digit code shown on both screens.
        code: String,
    },
    PairPending {
        request_id: String,
    },
    PairPoll {
        request_id: String,
    },
    PairStatus {
        request_id: String,
        status: PairState,
        token: Option<String>,
        message: Option<String>,
    },
    PairSunshine {
        pin: String,
        host_name: String,
    },
    PairSunshineResult {
        ok: bool,
        message: String,
    },
    Info,
    InfoResult {
        name: String,
        width: u32,
        height: u32,
        scale: f64,
        platform: String,
    },
    StartSession {
        session: StartStream,
    },
    StopSession {
        session_id: String,
        reason: String,
    },
    Status,
    StatusResult {
        status: SessionStatus,
    },
    Ping,
    Pong,
    Error {
        code: String,
        message: String,
    },
}

impl Message {
    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Message::Error {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicPendingPair {
    pub id: String,
    pub device_id: String,
    pub name: String,
    pub platform: String,
    pub remote_addr: String,
    pub code: String,
    pub created: u64,
    pub status: PairState,
}

#[derive(Debug, Clone)]
struct PendingPair {
    public: PublicPendingPair,
    token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PairLookup {
    pub status: PairState,
    pub token: Option<String>,
}

pub struct PairingStore {
    inner: Mutex<Vec<PendingPair>>,
    max_pending: usize,
    timeout_secs: u64,
}

impl PairingStore {
    pub fn new(max_pending: usize, timeout_secs: u64) -> Self {
        Self {
            inner: Mutex::new(Vec::new()),
            max_pending,
            timeout_secs,
        }
    }

    fn prune(&self, pairs: &mut Vec<PendingPair>) {
        let now = now_unix();
        for pair in pairs.iter_mut() {
            if pair.public.status == PairState::Pending
                && now.saturating_sub(pair.public.created) > self.timeout_secs
            {
                pair.public.status = PairState::Expired;
            }
        }
        pairs.retain(|p| {
            now.saturating_sub(p.public.created) < self.timeout_secs.saturating_mul(2).max(300)
        });
    }

    pub fn create(
        &self,
        device_id: &str,
        name: &str,
        platform: &str,
        remote_addr: &str,
        code: &str,
    ) -> Result<PublicPendingPair, String> {
        let mut pairs = self.inner.lock().map_err(|_| "pairing lock poisoned")?;
        self.prune(&mut pairs);
        if pairs
            .iter()
            .filter(|p| p.public.status == PairState::Pending)
            .count()
            >= self.max_pending
        {
            return Err(
                "too many pending pairing requests; approve or reject them first".to_string(),
            );
        }
        let id = crate::util::random_hex(16);
        let pair = PendingPair {
            public: PublicPendingPair {
                id: id.clone(),
                device_id: device_id.to_string(),
                name: name.to_string(),
                platform: platform.to_string(),
                remote_addr: remote_addr.to_string(),
                code: code.to_string(),
                created: now_unix(),
                status: PairState::Pending,
            },
            token: None,
        };
        pairs.push(pair.clone());
        Ok(pair.public)
    }

    pub fn list(&self) -> Vec<PublicPendingPair> {
        let mut pairs = match self.inner.lock() {
            Ok(p) => p,
            Err(_) => return Vec::new(),
        };
        self.prune(&mut pairs);
        pairs
            .iter()
            .filter(|p| p.public.status == PairState::Pending)
            .map(|p| p.public.clone())
            .collect()
    }

    pub fn lookup(&self, request_id: &str) -> PairLookup {
        let mut pairs = match self.inner.lock() {
            Ok(p) => p,
            Err(_) => {
                return PairLookup {
                    status: PairState::NotFound,
                    token: None,
                }
            }
        };
        self.prune(&mut pairs);
        match pairs.iter().find(|p| p.public.id == request_id) {
            Some(p) => PairLookup {
                status: p.public.status,
                token: p.token.clone(),
            },
            None => PairLookup {
                status: PairState::NotFound,
                token: None,
            },
        }
    }

    pub fn get(&self, request_id: &str) -> Option<PublicPendingPair> {
        let mut pairs = self.inner.lock().ok()?;
        self.prune(&mut pairs);
        pairs
            .iter()
            .find(|p| p.public.id == request_id)
            .map(|p| p.public.clone())
    }

    pub fn approve(&self, request_id: &str, token: &str) -> Result<PublicPendingPair> {
        let mut pairs = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("pairing lock poisoned"))?;
        self.prune(&mut pairs);
        let pair = pairs
            .iter_mut()
            .find(|p| p.public.id == request_id)
            .context("pairing request not found")?;
        if pair.public.status != PairState::Pending {
            bail!("pairing request is no longer pending");
        }
        pair.public.status = PairState::Approved;
        pair.token = Some(token.to_string());
        Ok(pair.public.clone())
    }

    pub fn reject(&self, request_id: &str) -> Result<PublicPendingPair> {
        let mut pairs = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("pairing lock poisoned"))?;
        self.prune(&mut pairs);
        let pair = pairs
            .iter_mut()
            .find(|p| p.public.id == request_id)
            .context("pairing request not found")?;
        if pair.public.status != PairState::Pending {
            bail!("pairing request is no longer pending");
        }
        pair.public.status = PairState::Rejected;
        Ok(pair.public.clone())
    }

    pub fn remove_device(&self, device_id: &str) {
        if let Ok(mut pairs) = self.inner.lock() {
            pairs.retain(|p| p.public.device_id != device_id);
        }
    }
}

/// Everything the control server needs to act on commands.
pub struct ControlContext {
    pub config: Arc<ConfigStore>,
    pub state: Arc<StateStore>,
    pub secrets: Arc<SecretStore>,
    pub pairing: Arc<PairingStore>,
    pub session: Arc<SessionManager>,
    pub discovery: DiscoveryRegistry,
    pub ui: Arc<crate::ui_bridge::UiBridge>,
    pub installer: Arc<crate::installer::Installer>,
}

impl ControlContext {
    pub fn validate_token(&self, token: &str) -> Option<PeerRecord> {
        if token.len() < 32 {
            return None;
        }
        for peer in self.state.peers() {
            if let Some(expected) = self.secrets.peer_token(&peer.device_id) {
                if constant_time_eq(expected.as_bytes(), token.as_bytes()) {
                    return Some(peer);
                }
            }
        }
        None
    }

    pub async fn approve_pairing(&self, request_id: &str) -> Result<PublicPendingPair> {
        let token = random_token();
        let approved = self.pairing.approve(request_id, &token)?;
        self.secrets.set_peer_token(&approved.device_id, &token)?;
        let now = now_unix();
        self.state.upsert_peer(PeerRecord {
            device_id: approved.device_id.clone(),
            name: approved.name.clone(),
            addresses: vec![approved.remote_addr.clone()],
            control_port: self.config.get().control.port,
            platform: approved.platform.clone(),
            version: String::new(),
            paired_at: now,
            last_seen: now,
            sunshine_paired: false,
            last_status: "Paired".to_string(),
        })?;
        notify::send(
            "Pairing approved",
            format!("{} can now start sessions on this PC", approved.name),
        )
        .await;
        Ok(approved)
    }

    pub fn reject_pairing(&self, request_id: &str) -> Result<PublicPendingPair> {
        self.pairing.reject(request_id)
    }

    pub async fn handle_message(&self, envelope: Envelope, peer_addr: SocketAddr) -> Message {
        if envelope.version != PROTO_VERSION {
            return Message::error(
                "version",
                format!("unsupported protocol version {}", envelope.version),
            );
        }
        let config = self.config.get();
        match envelope.message {
            Message::Hello {
                device_id,
                name,
                version,
                platform,
            } => {
                tracing::info!(
                    "hello from {name} ({device_id}) at {peer_addr} (version {version}, platform {platform})"
                );
                Message::HelloAck {
                    device_id: config.device.id.clone(),
                    name: config.device.name.clone(),
                    version: HELLO_VERSION.to_string(),
                }
            }
            Message::PairRequest {
                device_id,
                name,
                platform,
                code,
            } => {
                if !valid_id(&device_id) {
                    return Message::error("bad_device", "invalid device id");
                }
                if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
                    return Message::error("bad_code", "pairing code must be six digits");
                }
                let pending = match self.pairing.create(
                    &device_id,
                    &name,
                    &platform,
                    &peer_addr.ip().to_string(),
                    &code,
                ) {
                    Ok(p) => p,
                    Err(err) => return Message::error("pairing_busy", err),
                };
                if config.pairing.auto_approve {
                    if let Err(err) = self.approve_pairing(&pending.id).await {
                        return Message::error("pairing_failed", format!("{err:#}"));
                    }
                    return Message::PairPending {
                        request_id: pending.id,
                    };
                }
                notify::send(
                    "KDE Fusion pairing request",
                    format!(
                        "{} wants to pair (code {}). Approve it in KDE Fusion.",
                        pending.name, pending.code
                    ),
                )
                .await;
                Message::PairPending {
                    request_id: pending.id,
                }
            }
            Message::PairPoll { request_id } => {
                let lookup = self.pairing.lookup(&request_id);
                Message::PairStatus {
                    request_id,
                    status: lookup.status,
                    token: lookup.token,
                    message: match lookup.status {
                        PairState::Pending => Some("waiting for approval".to_string()),
                        PairState::Rejected => Some("pairing rejected".to_string()),
                        PairState::Expired => Some("pairing request expired".to_string()),
                        PairState::NotFound => Some("pairing request not found".to_string()),
                        PairState::Approved => None,
                    },
                }
            }
            Message::PairSunshine { pin, host_name } => {
                if let Err(err) = self.require_auth(&envelope.token) {
                    return err;
                }
                let host = peer_addr.ip().to_string();
                tracing::info!("pairing Moonlight to {host} on behalf of {host_name}");
                match moonlight::pair(&host, &pin).await {
                    Ok(_) => Message::PairSunshineResult {
                        ok: true,
                        message: format!("paired with {host}"),
                    },
                    Err(err) => Message::PairSunshineResult {
                        ok: false,
                        message: format!("{err:#}"),
                    },
                }
            }
            Message::Info => {
                if let Err(err) = self.require_auth(&envelope.token) {
                    return err;
                }
                let (width, height, scale) = match Kscreen::default().list().await {
                    Ok(snapshot) => match snapshot.choose_physical(&config.display.physical_output)
                    {
                        Ok(output) => (output.width, output.height, output.scale),
                        Err(_) => (1920, 1080, 1.0),
                    },
                    Err(_) => (1920, 1080, 1.0),
                };
                Message::InfoResult {
                    name: config.device.name.clone(),
                    width,
                    height,
                    scale,
                    platform: std::env::consts::OS.to_string(),
                }
            }
            Message::StartSession { session } => {
                let peer = match self.require_auth(&envelope.token) {
                    Ok(peer) => peer,
                    Err(err) => return err,
                };
                match self
                    .session
                    .start_viewer(peer_addr.ip(), &peer, session)
                    .await
                {
                    Ok(status) => {
                        self.session.spawn_viewer_watcher(status.session_id.clone());
                        Message::StatusResult { status }
                    }
                    Err(err) => Message::error("start_failed", format!("{err:#}")),
                }
            }
            Message::StopSession { session_id, reason } => {
                if let Err(err) = self.require_auth(&envelope.token) {
                    return err;
                }
                match self.session.stop_viewer(&session_id, &reason).await {
                    Ok(status) => Message::StatusResult { status },
                    Err(err) => Message::error("stop_failed", format!("{err:#}")),
                }
            }
            Message::Status => {
                if let Err(err) = self.require_auth(&envelope.token) {
                    return err;
                }
                Message::StatusResult {
                    status: self.session.status().await,
                }
            }
            Message::Ping => Message::Pong,
            Message::HelloAck { .. }
            | Message::PairPending { .. }
            | Message::PairStatus { .. }
            | Message::PairSunshineResult { .. }
            | Message::InfoResult { .. }
            | Message::StatusResult { .. }
            | Message::Pong
            | Message::Error { .. } => {
                Message::error("unexpected", "unexpected message for a server")
            }
        }
    }

    fn require_auth(&self, token: &Option<String>) -> Result<PeerRecord, Message> {
        let token = token
            .as_deref()
            .ok_or_else(|| Message::error("auth", "pair this PC first"))?;
        self.validate_token(token)
            .ok_or_else(|| Message::error("auth", "unknown or revoked pairing token"))
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub async fn serve(listener: TcpListener, ctx: Arc<ControlContext>) -> Result<()> {
    loop {
        let (stream, addr) = match listener.accept().await {
            Ok(pair) => pair,
            Err(err) => {
                tracing::warn!("control accept failed: {err}");
                tokio::time::sleep(Duration::from_millis(250)).await;
                continue;
            }
        };
        let ctx = ctx.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_connection(stream, addr, ctx).await {
                tracing::debug!("control connection from {addr} ended: {err:#}");
            }
        });
    }
}

async fn handle_connection(
    stream: TcpStream,
    addr: SocketAddr,
    ctx: Arc<ControlContext>,
) -> Result<()> {
    stream.set_nodelay(true).ok();
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    loop {
        let mut line = String::new();
        let read =
            tokio::time::timeout(Duration::from_secs(180), reader.read_line(&mut line)).await;
        let read = match read {
            Ok(result) => result?,
            Err(_) => break,
        };
        if read == 0 {
            break;
        }
        if line.len() > MAX_LINE {
            let response = Envelope {
                version: PROTO_VERSION,
                request_id: None,
                token: None,
                message: Message::error("too_large", "request exceeds 1 MiB"),
            };
            write_half
                .write_all(format!("{}\n", serde_json::to_string(&response)?).as_bytes())
                .await?;
            break;
        }
        let envelope: Envelope = match serde_json::from_str(line.trim_end()) {
            Ok(envelope) => envelope,
            Err(err) => {
                let response = Envelope {
                    version: PROTO_VERSION,
                    request_id: None,
                    token: None,
                    message: Message::error("bad_json", format!("{err}")),
                };
                write_half
                    .write_all(format!("{}\n", serde_json::to_string(&response)?).as_bytes())
                    .await?;
                continue;
            }
        };
        let request_id = envelope.request_id.clone();
        let response = ctx.handle_message(envelope, addr).await;
        let envelope = Envelope {
            version: PROTO_VERSION,
            request_id,
            token: None,
            message: response,
        };
        write_half
            .write_all(format!("{}\n", serde_json::to_string(&envelope)?).as_bytes())
            .await?;
        write_half.flush().await.ok();
    }
    Ok(())
}

pub struct ControlClient {
    reader: BufReader<tokio::net::tcp::OwnedReadHalf>,
    writer: tokio::net::tcp::OwnedWriteHalf,
    pub local_addr: SocketAddr,
    pub peer_addr: SocketAddr,
}

impl ControlClient {
    pub async fn connect(addr: SocketAddr) -> Result<Self> {
        let stream = tokio::time::timeout(Duration::from_secs(6), TcpStream::connect(addr))
            .await
            .context("control connection timed out")?
            .with_context(|| format!("connect to {addr}"))?;
        stream.set_nodelay(true).ok();
        let local_addr = stream.local_addr().context("local address")?;
        let peer_addr = stream.peer_addr().context("peer address")?;
        let (read_half, write_half) = stream.into_split();
        Ok(Self {
            reader: BufReader::new(read_half),
            writer: write_half,
            local_addr,
            peer_addr,
        })
    }

    pub async fn request(&mut self, token: Option<&str>, message: Message) -> Result<Message> {
        let request_id = crate::util::random_hex(8);
        let envelope = Envelope {
            version: PROTO_VERSION,
            request_id: Some(request_id.clone()),
            token: token.map(ToString::to_string),
            message,
        };
        let line = serde_json::to_string(&envelope)?;
        self.writer.write_all(line.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        let mut response_line = String::new();
        tokio::time::timeout(
            Duration::from_secs(60),
            self.reader.read_line(&mut response_line),
        )
        .await
        .context("control request timed out")??;
        if response_line.trim().is_empty() {
            bail!("control connection closed");
        }
        let response: Envelope =
            serde_json::from_str(response_line.trim_end()).context("parse control response")?;
        if let Some(id) = &response.request_id {
            if id != &request_id {
                tracing::debug!("control response id mismatch (got {id}, wanted {request_id})");
            }
        }
        match response.message {
            Message::Error { code, message } => bail!("{code}: {message}"),
            other => Ok(other),
        }
    }

    pub async fn hello(&mut self, device_id: &str, name: &str) -> Result<Message> {
        self.request(
            None,
            Message::Hello {
                device_id: device_id.to_string(),
                name: name.to_string(),
                version: HELLO_VERSION.to_string(),
                platform: std::env::consts::OS.to_string(),
            },
        )
        .await
    }
}

/// Pair with a remote daemon.  The returned token is only valid for that peer.
pub async fn pair(
    addr: SocketAddr,
    device_id: &str,
    name: &str,
    code: &str,
    timeout: Duration,
) -> Result<String> {
    let mut client = ControlClient::connect(addr).await?;
    client.hello(device_id, name).await?;
    let response = client
        .request(
            None,
            Message::PairRequest {
                device_id: device_id.to_string(),
                name: name.to_string(),
                platform: std::env::consts::OS.to_string(),
                code: code.to_string(),
            },
        )
        .await?;
    let request_id = match response {
        Message::PairPending { request_id } => request_id,
        other => bail!("unexpected pairing response: {other:?}"),
    };
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if tokio::time::Instant::now() >= deadline {
            bail!("pairing was not approved in time");
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        let response = client
            .request(
                None,
                Message::PairPoll {
                    request_id: request_id.clone(),
                },
            )
            .await?;
        if let Message::PairStatus {
            status,
            token,
            message,
            ..
        } = response
        {
            match status {
                PairState::Approved => {
                    return token.context("remote approved pairing without a token");
                }
                PairState::Rejected => {
                    bail!("{}", message.unwrap_or_else(|| "pairing rejected".into()))
                }
                PairState::Expired => bail!("pairing request expired"),
                PairState::NotFound => bail!("pairing request disappeared"),
                PairState::Pending => {}
            }
        }
    }
}

pub async fn start_session(
    addr: SocketAddr,
    token: &str,
    session: StartStream,
) -> Result<SessionStatus> {
    let mut client = ControlClient::connect(addr).await?;
    let response = client
        .request(Some(token), Message::StartSession { session })
        .await?;
    match response {
        Message::StatusResult { status } => Ok(status),
        other => bail!("unexpected start response: {other:?}"),
    }
}

pub async fn stop_session(
    addr: SocketAddr,
    token: &str,
    session_id: &str,
    reason: &str,
) -> Result<SessionStatus> {
    let mut client = ControlClient::connect(addr).await?;
    let response = client
        .request(
            Some(token),
            Message::StopSession {
                session_id: session_id.to_string(),
                reason: reason.to_string(),
            },
        )
        .await?;
    match response {
        Message::StatusResult { status } => Ok(status),
        other => bail!("unexpected stop response: {other:?}"),
    }
}

pub async fn pair_sunshine(
    addr: SocketAddr,
    token: &str,
    pin: &str,
    host_name: &str,
) -> Result<()> {
    let mut client = ControlClient::connect(addr).await?;
    let response = client
        .request(
            Some(token),
            Message::PairSunshine {
                pin: pin.to_string(),
                host_name: host_name.to_string(),
            },
        )
        .await?;
    match response {
        Message::PairSunshineResult { ok: true, .. } => Ok(()),
        Message::PairSunshineResult { ok: false, message } => bail!("{message}"),
        other => bail!("unexpected pairing response: {other:?}"),
    }
}

pub async fn viewer_info(addr: SocketAddr, token: &str) -> Result<Message> {
    let mut client = ControlClient::connect(addr).await?;
    client.request(Some(token), Message::Info).await
}

pub async fn status(addr: SocketAddr, token: &str) -> Result<SessionStatus> {
    let mut client = ControlClient::connect(addr).await?;
    match client.request(Some(token), Message::Status).await? {
        Message::StatusResult { status } => Ok(status),
        other => bail!("unexpected status response: {other:?}"),
    }
}

/// Resolve a peer's mDNS-discovered control endpoint.
pub fn pick_address(addresses: &[String], port: u16) -> Option<SocketAddr> {
    for addr in addresses {
        if let Ok(ip) = addr.parse::<IpAddr>() {
            if ip.is_ipv4() {
                return Some(SocketAddr::new(ip, port));
            }
        }
    }
    addresses.iter().find_map(|addr| {
        addr.parse::<IpAddr>()
            .ok()
            .map(|ip| SocketAddr::new(ip, port))
    })
}

/// Simple local network sanity check for pairing attempts.
pub fn is_lan_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_private() || ip.is_link_local(),
        IpAddr::V6(ip) => ip.is_unique_local() || ip.is_unicast_link_local(),
    }
}

pub async fn systemctl_active(unit: &str) -> bool {
    matches!(
        run(
            "systemctl",
            &["--user".to_string(), "is-active".to_string(), unit.to_string()]
        )
        .await,
        Ok(out) if out.stdout.trim() == "active"
    )
}

/// Helpful for diagnostics and tests; returns a compact view of a peer list.
pub fn peer_map(peers: &[PeerRecord]) -> HashMap<String, String> {
    peers
        .iter()
        .map(|p| (p.device_id.clone(), p.name.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_roundtrip() {
        let envelope = Envelope {
            version: 1,
            request_id: Some("r1".into()),
            token: Some("t".into()),
            message: Message::StartSession {
                session: StartStream {
                    session_id: "s".into(),
                    mode: crate::types::SessionMode::Mirror,
                    layout: crate::types::Layout::Right,
                    width: 1920,
                    height: 1080,
                    fps: 60,
                    bitrate_kbps: 40_000,
                    audio: true,
                    codec: "HEVC".into(),
                    app: "Desktop".into(),
                    host_name: "host".into(),
                },
            },
        };
        let text = serde_json::to_string(&envelope).unwrap();
        assert!(text.contains("\"type\":\"start_session\""));
        let decoded: Envelope = serde_json::from_str(&text).unwrap();
        assert_eq!(decoded.version, 1);
        assert!(matches!(decoded.message, Message::StartSession { .. }));
    }

    #[test]
    fn pairing_store_approval_flow() {
        let store = PairingStore::new(4, 120);
        let pending = store
            .create("dev", "Other", "linux", "192.168.1.2", "123456")
            .unwrap();
        assert_eq!(store.list().len(), 1);
        assert_eq!(store.lookup(&pending.id).status, PairState::Pending);
        store.approve(&pending.id, "token").unwrap();
        let lookup = store.lookup(&pending.id);
        assert_eq!(lookup.status, PairState::Approved);
        assert_eq!(lookup.token.as_deref(), Some("token"));
    }

    #[test]
    fn pairing_store_rejects_bad_state() {
        let store = PairingStore::new(4, 120);
        let pending = store
            .create("dev", "Other", "linux", "192.168.1.2", "123456")
            .unwrap();
        store.reject(&pending.id).unwrap();
        assert!(store.approve(&pending.id, "token").is_err());
        assert_eq!(store.lookup(&pending.id).status, PairState::Rejected);
    }

    #[test]
    fn validates_ids_and_lan() {
        assert!(valid_id("abc-123_XYZ"));
        assert!(!valid_id("bad id"));
        assert!(!valid_id(""));
        assert!(is_lan_ip("192.168.1.5".parse().unwrap()));
        assert!(is_lan_ip("10.0.0.5".parse().unwrap()));
        assert!(is_lan_ip("fd00::1".parse().unwrap()));
        assert!(!is_lan_ip("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn picks_ipv4_address() {
        let addr =
            pick_address(&["fe80::1".to_string(), "192.168.1.4".to_string()], 47811).unwrap();
        assert_eq!(addr.to_string(), "192.168.1.4:47811");
    }

    #[test]
    fn constant_time_eq_works() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
