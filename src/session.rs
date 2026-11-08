//! Session orchestration.
//!
//! Host: snapshot KScreen -> virtual output -> mirror/extend/headless layout ->
//! Sunshine -> pair viewer if needed -> tell the viewer to launch Moonlight ->
//! monitor/stop and restore.
//!
//! Viewer: launch Moonlight fullscreen and watch its process.

use anyhow::{bail, Context, Result};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};

use crate::config::ConfigStore;
use crate::control;
use crate::discovery::DiscoveryRegistry;
use crate::kscreen::{edge_positions, Kscreen, KscreenSnapshot, OutputInfo};
use crate::moonlight::{MoonlightProcess, MoonlightSpec};
use crate::notify;
use crate::secrets::SecretStore;
use crate::state::{PeerRecord, PersistedSession, StateStore};
use crate::sunshine::{self, SunshineRuntime};
use crate::types::{Phase, SessionMode, SessionRequest, SessionStatus, StartStream};
use crate::util::{now_unix, random_pin4, state_dir};
use crate::virtual_display::{kill_stale_virtual, VirtualDisplay, VirtualDisplaySpec};

struct HostSession {
    session_id: String,
    peer: PeerRecord,
    peer_addr: SocketAddr,
    token: String,
    request: SessionRequest,
    audio: bool,
    /// Physical KScreen pixel size (mirror source).
    physical_pixel: (u32, u32),
    /// Actual virtual output pixel size used for the Moonlight stream.
    stream_pixel: (u32, u32),
    snapshot: Option<KscreenSnapshot>,
    snapshot_path: PathBuf,
    virtual_display: Option<VirtualDisplay>,
    sunshine: Option<SunshineRuntime>,
    started_at: u64,
}

struct ViewerSession {
    session_id: String,
    host_name: String,
    start: StartStream,
    moonlight: Option<MoonlightProcess>,
    started_at: u64,
}

pub struct SessionManager {
    config: Arc<ConfigStore>,
    state: Arc<StateStore>,
    secrets: Arc<SecretStore>,
    discovery: DiscoveryRegistry,
    status: RwLock<SessionStatus>,
    active_host: Mutex<Option<HostSession>>,
    active_viewer: Mutex<Option<ViewerSession>>,
    operation: Mutex<()>,
    start_gate: Mutex<()>,
    stop_requested: AtomicBool,
}

impl SessionManager {
    pub fn new(
        config: Arc<ConfigStore>,
        state: Arc<StateStore>,
        secrets: Arc<SecretStore>,
        discovery: DiscoveryRegistry,
    ) -> Self {
        Self {
            config,
            state,
            secrets,
            discovery,
            status: RwLock::new(SessionStatus::default()),
            active_host: Mutex::new(None),
            active_viewer: Mutex::new(None),
            operation: Mutex::new(()),
            start_gate: Mutex::new(()),
            stop_requested: AtomicBool::new(false),
        }
    }

    pub async fn status(&self) -> SessionStatus {
        self.status.read().await.clone()
    }

    async fn set_phase(&self, phase: Phase, detail: impl Into<String>) {
        let mut status = self.status.write().await;
        status.phase = phase;
        status.detail = detail.into();
        if !matches!(phase, Phase::Failed) {
            status.error.clear();
        }
    }

    async fn fail_status(&self, error: impl Into<String>) {
        let mut status = self.status.write().await;
        status.phase = Phase::Failed;
        status.error = error.into();
        status.detail = "Session failed".to_string();
    }

    async fn set_idle(&self, detail: impl Into<String>) {
        let mut status = self.status.write().await;
        let old = status.clone();
        *status = SessionStatus {
            phase: Phase::Idle,
            detail: detail.into(),
            session_id: old.session_id,
            peer_id: old.peer_id,
            peer_name: old.peer_name,
            mode: old.mode,
            layout: old.layout,
            audio: old.audio,
            width: old.width,
            height: old.height,
            role: old.role,
            ..Default::default()
        };
    }

    /// Called by the local HTTP API.  Starts a background task and returns
    /// immediately so the GUI can poll progress.
    pub async fn request_start(self: &Arc<Self>, request: SessionRequest) -> Result<()> {
        if request.peer_id.trim().is_empty() {
            bail!("no peer selected");
        }
        let _gate = self.start_gate.lock().await;
        {
            let status = self.status.read().await;
            if status.phase.is_busy() || status.phase.is_active() {
                bail!("a session is already active ({})", status.phase_name());
            }
        }
        if self.active_viewer.lock().await.is_some() {
            bail!("this PC is currently viewing another desktop");
        }
        let (peer, addr, token) = self.resolve_peer(&request.peer_id).await?;
        let config = self.config.get();
        let audio = request
            .audio
            .unwrap_or_else(|| config.default_audio_for(&request.mode.to_string()));
        self.stop_requested.store(false, Ordering::SeqCst);
        {
            let mut status = self.status.write().await;
            *status = SessionStatus {
                phase: Phase::Preparing,
                session_id: crate::util::random_hex(8),
                peer_id: peer.device_id.clone(),
                peer_name: peer.name.clone(),
                mode: request.mode,
                layout: request.layout,
                audio,
                detail: "Preparing session".to_string(),
                role: "host".to_string(),
                started_at: now_unix(),
                ..Default::default()
            };
        }
        let this = self.clone();
        tokio::spawn(async move {
            this.run_host(request, peer, addr, token, audio).await;
        });
        Ok(())
    }

    async fn resolve_peer(&self, peer_id: &str) -> Result<(PeerRecord, SocketAddr, String)> {
        let peer = self
            .state
            .peer(peer_id)
            .with_context(|| format!("peer {peer_id} is not paired. Pair it first."))?;
        let token = self
            .secrets
            .peer_token(peer_id)
            .context("pairing token is missing; remove and pair the peer again")?;
        let discovered = self.discovery.get(peer_id);
        let mut addresses = peer.addresses.clone();
        let mut port = if peer.control_port == 0 {
            self.config.get().control.port
        } else {
            peer.control_port
        };
        if let Some(discovered) = discovered {
            for address in discovered.addresses {
                if !addresses.contains(&address) {
                    addresses.push(address);
                }
            }
            if discovered.port != 0 {
                port = discovered.port;
            }
        }
        let addr = control::pick_address(&addresses, port)
            .with_context(|| format!("no LAN address known for {}", peer.name))?;
        if !control::is_lan_ip(addr.ip()) {
            tracing::warn!("peer {} resolved to non-LAN address {}", peer.name, addr);
        }
        Ok((peer, addr, token))
    }

    async fn run_host(
        self: Arc<Self>,
        request: SessionRequest,
        peer: PeerRecord,
        peer_addr: SocketAddr,
        token: String,
        audio: bool,
    ) {
        let _guard = self.operation.lock().await;
        let config = self.config.get();
        let session_id = self.status.read().await.session_id.clone();
        let snapshot_path = state_dir().join(format!("display-{session_id}.json"));
        let mut session = HostSession {
            session_id: session_id.clone(),
            peer: peer.clone(),
            peer_addr,
            token: token.clone(),
            request: request.clone(),
            audio,
            physical_pixel: (0, 0),
            stream_pixel: (0, 0),
            snapshot: None,
            snapshot_path,
            virtual_display: None,
            sunshine: None,
            started_at: now_unix(),
        };

        let result = self.run_host_inner(&mut session).await;
        match result {
            Ok(()) => {
                let stream = session;
                let width = stream.stream_pixel.0 as u32;
                let height = stream.stream_pixel.1 as u32;
                {
                    let mut status = self.status.write().await;
                    status.phase = Phase::Streaming;
                    status.detail = format!("Streaming to {} at {}x{}", peer.name, width, height);
                    status.width = width;
                    status.height = height;
                    status.error.clear();
                }
                self.state
                    .set_active_session(persisted_from_host(&stream, &config))
                    .ok();
                self.active_host.lock().await.replace(stream);
                notify::send(
                    "KDE Fusion streaming",
                    format!("{} is now viewing this desktop", peer.name),
                )
                .await;
                let this = self.clone();
                let addr = peer_addr;
                let token = token.clone();
                let pid = session_id.clone();
                tokio::spawn(async move {
                    this.monitor_host(pid, addr, token).await;
                });
            }
            Err(err) => {
                tracing::error!("host session failed: {err:#}");
                self.cleanup_host(&mut session, "session failed").await;
                if self.stop_requested.load(Ordering::SeqCst) {
                    self.set_idle("Session stopped").await;
                } else {
                    self.fail_status(format!("{err:#}")).await;
                    notify::error("KDE Fusion session failed", format!("{err:#}")).await;
                }
            }
        }
    }

    async fn run_host_inner(&self, session: &mut HostSession) -> Result<()> {
        let config = self.config.get();
        self.check_stop()?;

        self.set_phase(Phase::Preparing, "Reading display layout")
            .await;
        let kscreen = Kscreen::default();
        let snapshot = kscreen.list().await?;
        let physical = snapshot
            .choose_physical(&config.display.physical_output)?
            .clone();
        session.physical_pixel = (physical.width, physical.height);
        session.snapshot_path = state_dir().join(format!("display-{}.json", session.session_id));
        snapshot.write(&session.snapshot_path)?;
        session.snapshot = Some(snapshot);
        self.check_stop()?;

        // Work out the virtual output geometry.
        let display_mode = session
            .request
            .display_mode
            .clone()
            .unwrap_or_else(|| "fullscreen".to_string());
        let forced_res = if display_mode == "windowed" {
            parse_resolution(&session.request.resolution)
                .or_else(|| parse_resolution(&config.streaming.resolution))
        } else {
            None
        };
        let (logical_w, logical_h, scale) = match session.request.mode {
            SessionMode::Mirror => {
                if let Some((w, h)) = forced_res {
                    (w, h, 1.0)
                } else {
                    (
                        physical.logical_width,
                        physical.logical_height,
                        physical.scale,
                    )
                }
            }
            SessionMode::Extend | SessionMode::Headless => {
                if let Some((w, h)) = forced_res {
                    (w, h, 1.0)
                } else {
                    let (w, h) = self
                        .client_resolution(
                            &session.peer_addr,
                            &session.token,
                            &session.request,
                            &physical,
                        )
                        .await;
                    (w, h, 1.0)
                }
            }
        };
        let virtual_name = config.display.virtual_name.clone();
        let spec = VirtualDisplaySpec {
            name: virtual_name,
            logical_width: logical_w,
            logical_height: logical_h,
            scale,
            vnc_port: config.display.vnc_port,
            timeout: Duration::from_secs(config.display.output_timeout_secs.max(5)),
        };

        self.set_phase(Phase::CreatingDisplay, "Creating virtual KDE output")
            .await;
        let virtual_display = VirtualDisplay::start(spec, &kscreen).await?;
        let output_name = virtual_display.output_name.clone();
        session.stream_pixel = virtual_display.spec.pixel_size();
        session.virtual_display = Some(virtual_display);
        self.state
            .set_active_session(persisted_partial(session))
            .ok();
        self.check_stop()?;

        self.set_phase(Phase::ConfiguringDisplay, "Applying KScreen layout")
            .await;
        let after_create = kscreen.list().await?;
        let virtual_output = after_create
            .find(&output_name)
            .with_context(|| format!("virtual output {output_name} vanished"))?
            .clone();
        session.stream_pixel = (virtual_output.width.max(1), virtual_output.height.max(1));

        match session.request.mode {
            SessionMode::Mirror => {
                kscreen
                    .configure_mirror(&output_name, &physical.name)
                    .await
                    .with_context(|| {
                        format!("could not mirror {} onto {}", output_name, physical.name)
                    })?;
            }
            SessionMode::Extend | SessionMode::Headless => {
                let mut virtual_output = virtual_output;
                let mut mode = virtual_output
                    .mode_for_size(virtual_output.width, virtual_output.height)
                    .cloned();
                if mode.is_none() {
                    tracing::warn!(
                        "no exact mode for {}x{} on {}; adding a custom mode",
                        virtual_output.width,
                        virtual_output.height,
                        output_name
                    );
                    let refresh = virtual_output
                        .current_mode()
                        .map(|m| m.refresh)
                        .unwrap_or(60.0);
                    kscreen
                        .add_custom_mode(
                            &output_name,
                            virtual_output.width,
                            virtual_output.height,
                            refresh,
                        )
                        .await
                        .ok();
                    let refreshed = kscreen.list().await?;
                    if let Some(v) = refreshed.find(&output_name) {
                        virtual_output = v.clone();
                        mode = v.mode_for_size(v.width, v.height).cloned();
                    }
                }
                let mode = mode.context("virtual output has no usable mode")?;
                let (virtual_pos, physical_pos) = edge_positions(
                    &physical,
                    virtual_output.logical_width,
                    virtual_output.logical_height,
                    session.request.layout,
                );
                let (x, y) = virtual_pos;
                let (px, py) = physical_pos;
                kscreen
                    .configure_extend(&output_name, &mode.id, x, y, &physical.name, px, py)
                    .await?;
                if session.request.mode == SessionMode::Headless {
                    let phys_names: Vec<&str> = session
                        .snapshot
                        .as_ref()
                        .map(|s| {
                            s.outputs
                                .iter()
                                .filter(|o| !o.is_virtual() && o.enabled)
                                .map(|o| o.name.as_str())
                                .collect()
                        })
                        .unwrap_or_default();
                    if !phys_names.is_empty() {
                        kscreen.disable_outputs(&phys_names).await?;
                    }
                }
            }
        }

        // Verify the layout actually stuck before starting Sunshine.
        let applied = kscreen.list().await?;
        let virtual_applied = applied
            .find(&output_name)
            .with_context(|| format!("{output_name} is missing after layout"))?;
        if !virtual_applied.enabled {
            bail!("{output_name} did not become enabled");
        }
        if session.request.mode == SessionMode::Mirror
            && virtual_applied.replication_source != physical.id
        {
            tracing::warn!(
                "KScreen reports replicationSource={} for {} (expected {})",
                virtual_applied.replication_source,
                output_name,
                physical.id
            );
        }
        self.check_stop()?;

        self.set_phase(Phase::StartingSunshine, "Starting Sunshine")
            .await;
        let runtime = SunshineRuntime::start(&config, session.audio).await?;
        let admin = sunshine::ensure_admin(&config, &self.secrets).await?;
        session.sunshine = Some(runtime);
        self.check_stop()?;

        let mut peer = session.peer.clone();
        if !peer.sunshine_paired && sunshine::is_client_paired(&peer.name) {
            // Previous pairing was approved by Sunshine but the session failed
            // afterwards, so the local flag never got persisted.
            tracing::info!(
                "Sunshine already trusts {}; skipping the Moonlight pairing step",
                peer.name
            );
            self.state.set_sunshine_paired(&peer.device_id, true).ok();
            peer.sunshine_paired = true;
            session.peer = peer.clone();
        }
        if !peer.sunshine_paired {
            self.set_phase(Phase::Pairing, "Pairing Moonlight with Sunshine")
                .await;
            let pin = random_pin4();
            tracing::info!("automating Sunshine pairing with {}", peer.name);
            let viewer_future = control::pair_sunshine(
                session.peer_addr,
                &session.token,
                &pin,
                &config.device.name,
            );
            let host_future = admin.poll_and_submit_pin(&pin, &peer.name, Duration::from_secs(50));
            let (viewer_result, host_result) = tokio::join!(viewer_future, host_future);
            let already_paired = |err: &anyhow::Error| {
                let text = format!("{err:#}").to_ascii_lowercase();
                text.contains("already") && text.contains("pair")
            };
            match viewer_result {
                Ok(()) => {}
                Err(err) if already_paired(&err) => {
                    tracing::info!("Moonlight reports the host is already paired");
                }
                Err(err) => return Err(err).context("Moonlight could not pair with Sunshine"),
            }
            host_result.context("Sunshine did not complete pairing")?;
            self.state.set_sunshine_paired(&peer.device_id, true).ok();
            peer.sunshine_paired = true;
            session.peer = peer.clone();
            notify::send(
                "Sunshine pairing complete",
                format!("{} can now connect without a PIN", peer.name),
            )
            .await;
        }
        self.check_stop()?;

        let support = sunshine::detect_encoder_support().await;
        let mut requested_codec = config.streaming.codec.clone();
        // Prefer H.264 when the viewer cannot hardware-decode HEVC, so both
        // sides stay hardware-accelerated.
        if let Ok(info) = control::viewer_info(session.peer_addr, &session.token).await {
            if let control::Message::InfoResult { hevc_decode, .. } = info {
                if !hevc_decode
                    && (requested_codec == "auto" || requested_codec == "HEVC")
                {
                    requested_codec = "H.264".to_string();
                }
            }
        }
        let codec = sunshine::select_codec(&requested_codec, &support);
        let fps = session.request.fps.unwrap_or(config.streaming.fps);
        let bitrate = session
            .request
            .bitrate_kbps
            .unwrap_or(config.streaming.bitrate_kbps);
        let start = StartStream {
            session_id: session.session_id.clone(),
            mode: session.request.mode,
            layout: session.request.layout,
            display_mode: session
                .request
                .display_mode
                .clone()
                .unwrap_or_else(|| "fullscreen".to_string()),
            host_device_id: config.device.id.clone(),
            width: session.stream_pixel.0.max(1),
            height: session.stream_pixel.1.max(1),
            fps,
            bitrate_kbps: bitrate,
            audio: session.audio,
            codec,
            app: sunshine::DESKTOP_APP.to_string(),
            host_name: config.device.name.clone(),
        };
        self.set_phase(Phase::Launching, "Starting Moonlight on the other PC")
            .await;
        let viewer_status =
            control::start_session(session.peer_addr, &session.token, start.clone())
                .await
                .context("the other PC could not start Moonlight")?;
        if viewer_status.phase == Phase::Failed {
            bail!(
                "the other PC failed to start Moonlight: {}",
                viewer_status.error
            );
        }
        if !matches!(viewer_status.phase, Phase::Streaming | Phase::Launching) {
            bail!(
                "the other PC returned unexpected state {}",
                viewer_status.phase_name()
            );
        }

        // Bidirectional clipboard sync.
        let peer_addr = session.peer_addr;
        let token = session.token.clone();
        tokio::spawn(async move {
            let mut last = crate::clipboard::take_last();
            loop {
                tokio::time::sleep(Duration::from_millis(750)).await;
                if let Some(text) = crate::clipboard::get().await {
                    if text != last && !text.is_empty() {
                        last = text.clone();
                        crate::clipboard::set_last(text.clone());
                        let _ = control::send_clipboard(peer_addr, &token, &text).await;
                    }
                }
            }
        });

        Ok(())
    }

    async fn client_resolution(
        &self,
        addr: &SocketAddr,
        token: &str,
        request: &SessionRequest,
        physical: &OutputInfo,
    ) -> (u32, u32) {
        if let Some((w, h)) = parse_resolution(&request.resolution) {
            return (w, h);
        }
        let config = self.config.get();
        if let Some((w, h)) = parse_resolution(&config.streaming.resolution) {
            return (w, h);
        }
        if request.mode == SessionMode::Mirror {
            return (physical.width, physical.height);
        }
        match control::viewer_info(*addr, token).await {
            Ok(control::Message::InfoResult { width, height, .. }) if width > 0 && height > 0 => {
                (width, height)
            }
            Ok(other) => {
                tracing::warn!("unexpected viewer info response: {other:?}; using 1920x1080");
                (1920, 1080)
            }
            Err(err) => {
                tracing::warn!("could not query viewer resolution ({err:#}); using 1920x1080");
                (1920, 1080)
            }
        }
    }

    fn check_stop(&self) -> Result<()> {
        if self.stop_requested.load(Ordering::SeqCst) {
            bail!("session stopped by user");
        }
        Ok(())
    }

    async fn monitor_host(self: Arc<Self>, session_id: String, addr: SocketAddr, token: String) {
        let mut consecutive_failures = 0u32;
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            {
                let active = self.active_host.lock().await;
                if active
                    .as_ref()
                    .map(|s| s.session_id != session_id)
                    .unwrap_or(true)
                {
                    return;
                }
            }
            match control::status(addr, &token).await {
                Ok(status) if matches!(status.phase, Phase::Idle | Phase::Failed) => {
                    tracing::info!(
                        "viewer session ended ({}), stopping host",
                        status.phase_name()
                    );
                    let _ = self.stop_internal("viewer session ended").await;
                    return;
                }
                Ok(_) => consecutive_failures = 0,
                Err(err) => {
                    consecutive_failures += 1;
                    tracing::debug!("viewer status check failed ({consecutive_failures}): {err:#}");
                    if consecutive_failures >= 3 {
                        let _ = self.stop_internal("lost contact with the other PC").await;
                        return;
                    }
                }
            }
        }
    }

    pub async fn request_stop(self: &Arc<Self>, reason: &str) -> Result<()> {
        let status = self.status.read().await;
        let busy = status.phase.is_busy() || status.phase.is_active();
        drop(status);
        let has_host = self.active_host.lock().await.is_some();
        let has_viewer = self.active_viewer.lock().await.is_some();
        if !busy && !has_host && !has_viewer {
            bail!("no active session");
        }
        self.stop_requested.store(true, Ordering::SeqCst);
        self.set_phase(Phase::Stopping, "Stopping session").await;
        let this = self.clone();
        let reason = reason.to_string();
        tokio::spawn(async move {
            let _ = this.stop_internal(&reason).await;
        });
        Ok(())
    }

    async fn stop_internal(&self, reason: &str) -> Result<SessionStatus> {
        let _guard = self.operation.lock().await;
        let mut stopped_any = false;
        if let Some(mut host) = self.active_host.lock().await.take() {
            stopped_any = true;
            self.cleanup_host(&mut host, reason).await;
        }
        if let Some(viewer) = self.active_viewer.lock().await.take() {
            stopped_any = true;
            if let Some(moonlight) = viewer.moonlight {
                moonlight.stop().await;
            }
            self.state.clear_active_session().ok();
            notify::send(
                "KDE Fusion session ended",
                format!("Stopped viewing {}", viewer.host_name),
            )
            .await;
        }
        if stopped_any {
            self.set_idle(format!("Session stopped: {reason}")).await;
        } else {
            self.set_idle("Idle").await;
        }
        self.stop_requested.store(false, Ordering::SeqCst);
        Ok(self.status().await)
    }

    async fn cleanup_host(&self, session: &mut HostSession, reason: &str) {
        tracing::info!("cleaning up host session {}", session.session_id);
        if let Some(runtime) = session.sunshine.take() {
            runtime.stop().await;
        }
        // Tell the viewer first so its Moonlight window closes promptly.
        let _ = control::stop_session(
            session.peer_addr,
            &session.token,
            &session.session_id,
            reason,
        )
        .await;
        if let Some(virtual_display) = session.virtual_display.take() {
            virtual_display.stop().await;
        }
        let config = self.config.get();
        if config.display.restore_on_stop {
            self.restore_display(session).await;
        }
        self.state.clear_active_session().ok();
        notify::send(
            "KDE Fusion session ended",
            format!("Stopped streaming to {} ({reason})", session.peer.name),
        )
        .await;
    }

    async fn restore_display(&self, session: &HostSession) {
        let Some(snapshot) = session.snapshot.as_ref() else {
            return;
        };
        let kscreen = Kscreen::default();
        for attempt in 1..=2 {
            match kscreen.restore(snapshot).await {
                Ok(()) => {
                    tracing::info!("restored the previous KScreen layout");
                    std::fs::remove_file(&session.snapshot_path).ok();
                    return;
                }
                Err(err) => {
                    tracing::warn!("KScreen restore attempt {attempt} failed: {err:#}");
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
        notify::error(
            "Display layout was not restored",
            "Open System Settings > Display and enable your physical monitor.",
        )
        .await;
    }

    pub async fn start_viewer(
        &self,
        host_addr: IpAddr,
        peer: &PeerRecord,
        start: StartStream,
    ) -> Result<SessionStatus> {
        if self.active_host.lock().await.is_some() {
            bail!("this PC is currently hosting a session");
        }
        let mut viewer_guard = self.active_viewer.lock().await;
        if let Some(existing) = viewer_guard.as_ref() {
            bail!(
                "this PC is already viewing {} ({})",
                existing.host_name,
                existing.session_id
            );
        }
        self.stop_requested.store(false, Ordering::SeqCst);
        let config = self.config.get();
        self.set_phase(Phase::Launching, "Launching Moonlight")
            .await;
        {
            let mut status = self.status.write().await;
            *status = SessionStatus {
                phase: Phase::Launching,
                session_id: start.session_id.clone(),
                peer_id: peer.device_id.clone(),
                peer_name: start.host_name.clone(),
                mode: start.mode,
                layout: start.layout,
                audio: start.audio,
                width: start.width,
                height: start.height,
                detail: format!("Launching Moonlight for {}", start.host_name),
                role: "viewer".to_string(),
                started_at: now_unix(),
                ..Default::default()
            };
        }
        let spec = MoonlightSpec {
            host: host_addr.to_string(),
            app: start.app.clone(),
            width: start.width,
            height: start.height,
            fps: start.fps,
            bitrate_kbps: start.bitrate_kbps,
            codec: start.codec.clone(),
            audio: start.audio,
            display_mode: start.display_mode.clone(),
            absolute_mouse: config.streaming.absolute_mouse,
            capture_system_keys: config.streaming.capture_system_keys.clone(),
            keep_awake: config.streaming.keep_awake,
            performance_overlay: config.streaming.performance_overlay,
        };
        let moonlight = match MoonlightProcess::launch(&spec).await {
            Ok(process) => process,
            Err(err) => {
                self.fail_status(format!("{err:#}")).await;
                return Err(err);
            }
        };
        // Bidirectional clipboard sync (viewer -> host).
        let host_addr = host_addr;
        let host_device_id = start.host_device_id.clone();
        let secrets = self.secrets.clone();
        tokio::spawn(async move {
            let token = match secrets.peer_token(&host_device_id) {
                Some(t) => t,
                None => return,
            };
            let mut last = crate::clipboard::take_last();
            loop {
                tokio::time::sleep(Duration::from_millis(750)).await;
                if let Some(text) = crate::clipboard::get().await {
                    if text != last && !text.is_empty() {
                        last = text.clone();
                        crate::clipboard::set_last(text.clone());
                        let addr = std::net::SocketAddr::new(host_addr, 47811);
                        let _ = control::send_clipboard(addr, &token, &text).await;
                    }
                }
            }
        });
        let viewer = ViewerSession {
            session_id: start.session_id.clone(),
            host_name: start.host_name.clone(),
            start: start.clone(),
            moonlight: Some(moonlight),
            started_at: now_unix(),
        };
        self.state
            .set_active_session(PersistedSession {
                session_id: viewer.session_id.clone(),
                role: "viewer".to_string(),
                mode: viewer.start.mode,
                peer_id: peer.device_id.clone(),
                peer_name: viewer.host_name.clone(),
                started_at: viewer.started_at,
                audio: viewer.start.audio,
                ..Default::default()
            })
            .ok();
        *viewer_guard = Some(viewer);
        drop(viewer_guard);
        {
            let mut status = self.status.write().await;
            status.phase = Phase::Streaming;
            status.detail = format!(
                "Viewing {} at {}x{}",
                start.host_name, start.width, start.height
            );
        }
        notify::send(
            "KDE Fusion connected",
            format!("Viewing {}", start.host_name),
        )
        .await;
        Ok(self.status().await)
    }

    pub async fn stop_viewer(&self, session_id: &str, reason: &str) -> Result<SessionStatus> {
        let mut guard = self.active_viewer.lock().await;
        let viewer = match guard.take() {
            Some(viewer) => viewer,
            None => return Ok(self.status().await),
        };
        if viewer.session_id != session_id {
            *guard = Some(viewer);
            bail!("session id mismatch");
        }
        drop(guard);
        if let Some(moonlight) = viewer.moonlight {
            moonlight.stop().await;
        }
        self.state.clear_active_session().ok();
        self.set_idle(format!("Stopped viewing {} ({reason})", viewer.host_name))
            .await;
        notify::send(
            "KDE Fusion session ended",
            format!("Stopped viewing {}", viewer.host_name),
        )
        .await;
        Ok(self.status().await)
    }

    async fn watch_viewer(self: Arc<Self>, session_id: String) {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let mut guard = self.active_viewer.lock().await;
            let Some(viewer) = guard.as_mut() else {
                return;
            };
            if viewer.session_id != session_id {
                return;
            }
            let running = viewer
                .moonlight
                .as_mut()
                .map(|m| m.is_running())
                .unwrap_or(false);
            if running {
                continue;
            }
            let viewer = guard.take().expect("viewer exists");
            drop(guard);
            if let Some(moonlight) = viewer.moonlight {
                moonlight.stop().await;
            }
            self.state.clear_active_session().ok();
            self.set_idle(format!("Moonlight for {} exited", viewer.host_name))
                .await;
            notify::send(
                "KDE Fusion session ended",
                format!("Moonlight for {} exited", viewer.host_name),
            )
            .await;
            return;
        }
    }

    /// Called once by the daemon after a viewer session starts.
    pub fn spawn_viewer_watcher(self: &Arc<Self>, session_id: String) {
        let this = self.clone();
        tokio::spawn(async move {
            this.watch_viewer(session_id).await;
        });
    }

    /// Restore a display layout left behind by a crash, then clear the marker.
    pub async fn recover_on_startup(&self) -> Result<()> {
        let state = self.state.get();
        let Some(session) = state.active_session.clone() else {
            return Ok(());
        };
        tracing::warn!(
            "recovering from an unclean shutdown (session {}, role {})",
            session.session_id,
            session.role
        );
        let config = self.config.get();
        if let Some(pid) = session.virtual_pid {
            match kill_stale_virtual(pid, &config.display.virtual_name) {
                Ok(true) => tracing::info!("removed the stale virtual output"),
                Ok(false) => tracing::debug!("stale virtual monitor process was already gone"),
                Err(err) => tracing::warn!("could not remove stale virtual monitor: {err:#}"),
            }
        }
        if !session.snapshot_path.is_empty() {
            let path = PathBuf::from(&session.snapshot_path);
            if path.exists() {
                match KscreenSnapshot::read(&path) {
                    Ok(snapshot) => {
                        if let Err(err) = Kscreen::default().restore(&snapshot).await {
                            tracing::error!("could not restore display after crash: {err:#}");
                        } else {
                            std::fs::remove_file(&path).ok();
                        }
                    }
                    Err(err) => tracing::warn!("could not read crashed snapshot: {err:#}"),
                }
            }
        }
        self.state.clear_active_session().ok();
        self.set_idle("Recovered display layout after an unclean shutdown")
            .await;
        notify::send(
            "KDE Fusion recovered",
            "The previous display layout was restored after an unclean shutdown.",
        )
        .await;
        Ok(())
    }

    pub async fn shutdown(self: &Arc<Self>) {
        let _ = self.stop_internal("daemon shutting down").await;
    }

    pub async fn host_active(&self) -> bool {
        self.active_host.lock().await.is_some()
    }

    pub async fn viewer_active(&self) -> bool {
        self.active_viewer.lock().await.is_some()
    }
}

fn persisted_partial(session: &HostSession) -> PersistedSession {
    PersistedSession {
        session_id: session.session_id.clone(),
        role: "host".to_string(),
        mode: session.request.mode,
        peer_id: session.peer.device_id.clone(),
        peer_name: session.peer.name.clone(),
        started_at: session.started_at,
        virtual_pid: session.virtual_display.as_ref().map(|v| v.pid),
        virtual_output: session
            .virtual_display
            .as_ref()
            .map(|v| v.output_name.clone())
            .unwrap_or_default(),
        snapshot_path: session.snapshot_path.display().to_string(),
        audio: session.audio,
        started_sunshine: session
            .sunshine
            .as_ref()
            .map(|s| s.started_by_us)
            .unwrap_or(false),
        status: SessionStatus::default(),
    }
}

fn persisted_from_host(session: &HostSession, config: &crate::config::Config) -> PersistedSession {
    let mut persisted = persisted_partial(session);
    persisted.status = SessionStatus {
        phase: Phase::Streaming,
        session_id: session.session_id.clone(),
        peer_id: session.peer.device_id.clone(),
        peer_name: session.peer.name.clone(),
        mode: session.request.mode,
        layout: session.request.layout,
        audio: session.audio,
        width: session.stream_pixel.0,
        height: session.stream_pixel.1,
        detail: "Streaming".to_string(),
        role: "host".to_string(),
        started_at: session.started_at,
        error: String::new(),
    };
    persisted.virtual_output = session
        .virtual_display
        .as_ref()
        .map(|v| v.output_name.clone())
        .unwrap_or_else(|| config.output_name());
    persisted
}

pub fn parse_resolution(value: &str) -> Option<(u32, u32)> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() {
        return None;
    }
    let (w, h) = value.split_once('x')?;
    let width = w.parse::<u32>().ok()?;
    let height = h.parse::<u32>().ok()?;
    (width >= 320 && height >= 200).then_some((width, height))
}

impl SessionStatus {
    pub fn phase_name(&self) -> &'static str {
        match self.phase {
            Phase::Idle => "idle",
            Phase::Preparing => "preparing",
            Phase::CreatingDisplay => "creating_display",
            Phase::ConfiguringDisplay => "configuring_display",
            Phase::StartingSunshine => "starting_sunshine",
            Phase::Pairing => "pairing",
            Phase::Launching => "launching",
            Phase::Streaming => "streaming",
            Phase::Stopping => "stopping",
            Phase::Failed => "failed",
        }
    }
}

// Wall-clock times in this module are Unix seconds (u64).

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_resolution() {
        assert_eq!(parse_resolution("1920x1080"), Some((1920, 1080)));
        assert_eq!(parse_resolution(" 1280X720 "), Some((1280, 720)));
        assert_eq!(parse_resolution(""), None);
        assert_eq!(parse_resolution("100x100"), None);
        assert_eq!(parse_resolution("bad"), None);
    }

    #[test]
    fn phase_names_are_stable() {
        let status = SessionStatus {
            phase: Phase::StartingSunshine,
            ..Default::default()
        };
        assert_eq!(status.phase_name(), "starting_sunshine");
    }
}
