use anyhow::{bail, Context, Result};
use clap::Parser;
use kdefusion::{
    api, config::ConfigStore, control, control::ControlContext, control::PairingStore, discovery,
    notify, secrets::SecretStore, session::SessionManager, state::StateStore, tray,
    tray::TrayCommand, ui_bridge::UiBridge, util, APP_NAME, VERSION,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tracing::Level;

#[derive(Parser, Debug)]
#[command(name = "kdefusiond", version = VERSION, about = "KDE Fusion user daemon")]
struct Args {
    /// Alternate configuration file.
    #[arg(long)]
    config: Option<PathBuf>,
    /// Print the effective configuration and exit.
    #[arg(long)]
    print_config: bool,
    /// Do not start mDNS discovery.
    #[arg(long)]
    no_discovery: bool,
    /// Log level: error, warn, info, debug, trace.
    #[arg(long, default_value = "info")]
    log_level: String,
}

struct PidGuard {
    path: PathBuf,
    pid: u32,
}

impl Drop for PidGuard {
    fn drop(&mut self) {
        if let Ok(text) = std::fs::read_to_string(&self.path) {
            if text.trim() == self.pid.to_string() {
                std::fs::remove_file(&self.path).ok();
            }
        }
    }
}

fn acquire_lock() -> Result<PidGuard> {
    let dir = util::state_dir();
    util::ensure_dir(&dir)?;
    let path = dir.join("daemon.pid");
    if path.exists() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(pid) = text.trim().parse::<u32>() {
                let cmdline = std::fs::read(format!("/proc/{pid}/cmdline"))
                    .ok()
                    .map(|b| String::from_utf8_lossy(&b).replace('\0', " "));
                if let Some(cmdline) = cmdline {
                    if cmdline.contains("kdefusiond") {
                        bail!(
                            "another KDE Fusion daemon is already running (pid {pid}). \
                             Stop it with: systemctl --user stop kdefusiond"
                        );
                    }
                }
            }
        }
        std::fs::remove_file(&path).ok();
    }
    let pid = std::process::id();
    util::atomic_write(&path, pid.to_string().as_bytes(), Some(0o600))?;
    Ok(PidGuard { path, pid })
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("kdefusiond: {err:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let args = Args::parse();
    let level = match args.log_level.to_ascii_lowercase().as_str() {
        "error" => Level::ERROR,
        "warn" => Level::WARN,
        "debug" => Level::DEBUG,
        "trace" => Level::TRACE,
        _ => Level::INFO,
    };
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(false)
        .compact()
        .init();

    let config_store = Arc::new(match args.config {
        Some(path) => ConfigStore::load_from(path)?,
        None => ConfigStore::load()?,
    });
    let config = config_store.get();
    if args.print_config {
        print!("{}", toml::to_string_pretty(&config)?);
        return Ok(());
    }

    tracing::info!(
        "{APP_NAME} {VERSION} starting on {} (session={}, desktop={})",
        config.device.name,
        std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into()),
        std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown".into())
    );
    if !util::is_wayland() {
        tracing::warn!("Wayland was not detected; virtual outputs will not work");
    }
    if !util::is_kde() {
        tracing::warn!("KDE was not detected; KScreen and KWin integration may not work");
    }

    let _lock = acquire_lock()?;
    let state = Arc::new(StateStore::load()?);
    let secrets = Arc::new(SecretStore::new());
    let discovery_registry = discovery::DiscoveryRegistry::new();
    let session = Arc::new(SessionManager::new(
        config_store.clone(),
        state.clone(),
        secrets.clone(),
        discovery_registry.clone(),
    ));

    if let Err(err) = session.recover_on_startup().await {
        tracing::warn!("startup recovery failed: {err:#}");
    }

    let pairing = Arc::new(PairingStore::new(
        config.pairing.max_pending.max(1),
        config.pairing.approval_timeout_secs.max(60),
    ));
    let ui = Arc::new(UiBridge::new());
    let installer = Arc::new(kdefusion::installer::Installer::new());
    let ctx = Arc::new(ControlContext {
        config: config_store.clone(),
        state: state.clone(),
        secrets: secrets.clone(),
        pairing,
        session: session.clone(),
        discovery: discovery_registry.clone(),
        ui: ui.clone(),
        installer: installer.clone(),
    });

    // StatusNotifierItem tray.  The QML side cannot own one on this Qt/Plasma
    // combination, so the daemon does.  Commands are handled in the wait loop
    // below.
    let (tray_tx, mut tray_rx) = tokio::sync::mpsc::unbounded_channel::<TrayCommand>();
    let tray_handle = tray::spawn(tray_tx.clone(), config.device.name.clone()).await;
    let _tray_tx = tray_tx;
    if let Some(handle) = tray_handle.clone() {
        let session = session.clone();
        tokio::spawn(async move {
            let mut last: Option<(bool, String)> = None;
            loop {
                let status = session.status().await;
                let active = status.phase.is_active() || status.phase.is_busy();
                let detail = if active {
                    let peer = if status.peer_name.is_empty() {
                        String::new()
                    } else {
                        format!(" with {}", status.peer_name)
                    };
                    format!("{}{}", status.detail, peer)
                } else {
                    String::new()
                };
                let next = (active, detail.clone());
                if last.as_ref() != Some(&next) {
                    last = Some(next);
                    if handle
                        .update(|tray| tray.set_status(active, detail))
                        .await
                        .is_none()
                    {
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
    }

    let mut discovery_handle = None;
    if !args.no_discovery && config.control.enabled {
        match discovery::start(
            &config.device.id,
            &config.device.name,
            config.control.port,
            discovery_registry.clone(),
        ) {
            Ok(handle) => {
                tracing::info!("mDNS discovery advertised as _kdefusion._tcp.local.");
                discovery_handle = Some(handle);
            }
            Err(err) => tracing::warn!("mDNS discovery is unavailable: {err:#}"),
        }
    }

    let api_listener = TcpListener::bind(format!("{}:{}", config.api.bind, config.api.port))
        .await
        .with_context(|| format!("bind local API to {}:{}", config.api.bind, config.api.port))?;
    let api_addr = api_listener.local_addr()?;
    tracing::info!("{}", api::banner(api_addr));
    {
        let ctx = ctx.clone();
        tokio::spawn(async move {
            if let Err(err) = api::serve(api_listener, ctx).await {
                tracing::error!("local API stopped: {err:#}");
            }
        });
    }

    if config.control.enabled {
        let control_listener =
            TcpListener::bind(format!("{}:{}", config.control.bind, config.control.port))
                .await
                .with_context(|| {
                    format!(
                        "bind control channel to {}:{}",
                        config.control.bind, config.control.port
                    )
                })?;
        let control_addr = control_listener.local_addr()?;
        tracing::info!("control channel listening on {control_addr} (LAN, token-authenticated)");
        let ctx = ctx.clone();
        tokio::spawn(async move {
            if let Err(err) = control::serve(control_listener, ctx).await {
                tracing::error!("control channel stopped: {err:#}");
            }
        });
    } else {
        tracing::warn!("control channel is disabled in the configuration");
    }

    // Wait for SIGINT/SIGTERM or a tray command.  systemd user units use SIGTERM.
    #[cfg(unix)]
    let exit_reason = 'wait: {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = signal(SignalKind::terminate())?;
        let mut interrupt = signal(SignalKind::interrupt())?;
        loop {
            tokio::select! {
                _ = interrupt.recv() => break 'wait "SIGINT",
                _ = term.recv() => break 'wait "SIGTERM",
                command = tray_rx.recv() => {
                    match command {
                        Some(TrayCommand::Open(page)) => {
                            if ui.gui_recently_seen() {
                                ui.request_open(page);
                            } else if !ui.recently_launched() {
                                ui.mark_launched();
                                if !tray::launch_gui(page) {
                                    notify::error(
                                        "KDE Fusion",
                                        "The graphical interface could not be started. \
                                         Check that kdefusion is installed.",
                                    )
                                    .await;
                                }
                            }
                        }
                        Some(TrayCommand::StopSession) => {
                            if let Err(err) =
                                session.request_stop("stopped from the tray").await
                            {
                                notify::send("KDE Fusion", format!("No session to stop: {err}"))
                                    .await;
                            }
                        }
                        Some(TrayCommand::Quit) => break 'wait "tray quit",
                        None => {}
                    }
                }
            }
        }
    };
    #[cfg(not(unix))]
    let exit_reason = 'wait: loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break 'wait "signal",
            command = tray_rx.recv() => {
                if matches!(command, Some(TrayCommand::Quit)) {
                    break 'wait "tray quit";
                }
            }
        }
    };

    tracing::info!("{exit_reason} received, shutting down");
    session.shutdown().await;
    if let Some(handle) = tray_handle.as_ref() {
        handle.shutdown();
    }
    if let Some(handle) = discovery_handle.as_ref() {
        handle.shutdown();
    }
    drop(discovery_handle);
    drop(_lock);
    Ok(())
}
