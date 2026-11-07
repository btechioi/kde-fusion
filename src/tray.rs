//! Native StatusNotifierItem tray icon for the daemon.
//!
//! The QML frontend must not use `Qt.labs.platform.SystemTrayIcon`: the plugin
//! crashes inside the Qt 6.12 / Plasma 6 Wayland platform integration.  The
//! daemon owns the tray item through the `ksni` crate instead, and the GUI is
//! raised through the loopback API (see [`crate::ui_bridge`]).

use std::path::PathBuf;
use std::process::{Command, Stdio};

use ksni::menu::StandardItem;
use ksni::{MenuItem, Tray, TrayMethods};
use tokio::sync::mpsc::UnboundedSender;

/// Pages the GUI can be asked to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Devices,
    Session,
    Settings,
    Diagnostics,
}

impl Page {
    pub fn as_arg(self) -> &'static str {
        match self {
            Page::Devices => "devices",
            Page::Session => "session",
            Page::Settings => "settings",
            Page::Diagnostics => "diagnostics",
        }
    }

    pub fn from_arg(value: &str) -> Option<Self> {
        match value {
            "devices" => Some(Page::Devices),
            "session" => Some(Page::Session),
            "settings" => Some(Page::Settings),
            "diagnostics" => Some(Page::Diagnostics),
            _ => None,
        }
    }
}

/// Actions requested from the tray menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    Open(Option<Page>),
    StopSession,
    Quit,
}

pub struct KdeFusionTray {
    tx: UnboundedSender<TrayCommand>,
    device_name: String,
    active: bool,
    detail: String,
}

impl KdeFusionTray {
    fn emit(&self, command: TrayCommand) {
        let _ = self.tx.send(command);
    }

    /// Called by the daemon's status poller.
    pub fn set_status(&mut self, active: bool, detail: String) {
        self.active = active;
        self.detail = detail;
    }
}

impl Tray for KdeFusionTray {
    fn id(&self) -> String {
        "kdefusion".into()
    }

    fn title(&self) -> String {
        if self.active && !self.detail.is_empty() {
            format!("KDE Fusion — {}", self.detail)
        } else {
            "KDE Fusion".into()
        }
    }

    fn icon_name(&self) -> String {
        "kdefusion".into()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }

    fn status(&self) -> ksni::Status {
        if self.active {
            ksni::Status::Active
        } else {
            ksni::Status::Passive
        }
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            icon_name: "kdefusion".into(),
            title: self.title(),
            description: if self.detail.is_empty() {
                format!("{} · desktop mirroring and extending", self.device_name)
            } else {
                format!("{}\n{}", self.device_name, self.detail)
            },
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.emit(TrayCommand::Open(None));
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Open KDE Fusion".into(),
                icon_name: "window-new".into(),
                activate: Box::new(|tray: &mut Self| tray.emit(TrayCommand::Open(None))),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Devices".into(),
                icon_name: "network-wired".into(),
                activate: Box::new(|tray: &mut Self| {
                    tray.emit(TrayCommand::Open(Some(Page::Devices)))
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Session".into(),
                icon_name: "media-playback-start".into(),
                activate: Box::new(|tray: &mut Self| {
                    tray.emit(TrayCommand::Open(Some(Page::Session)))
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Settings".into(),
                icon_name: "settings-configure".into(),
                activate: Box::new(|tray: &mut Self| {
                    tray.emit(TrayCommand::Open(Some(Page::Settings)))
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Diagnostics".into(),
                icon_name: "utilities-system-monitor".into(),
                activate: Box::new(|tray: &mut Self| {
                    tray.emit(TrayCommand::Open(Some(Page::Diagnostics)))
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Stop session".into(),
                icon_name: "media-playback-stop".into(),
                enabled: self.active,
                activate: Box::new(|tray: &mut Self| tray.emit(TrayCommand::StopSession)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit KDE Fusion".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|tray: &mut Self| tray.emit(TrayCommand::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Publish the tray item.  Returns `None` when no StatusNotifier host is
/// available, which is normal in a plain TTY session.
pub async fn spawn(
    tx: UnboundedSender<TrayCommand>,
    device_name: String,
) -> Option<ksni::Handle<KdeFusionTray>> {
    let tray = KdeFusionTray {
        tx,
        device_name,
        active: false,
        detail: String::new(),
    };
    match tray.spawn().await {
        Ok(handle) => {
            tracing::info!("tray icon registered (StatusNotifierItem)");
            Some(handle)
        }
        Err(err) => {
            tracing::warn!("tray icon is unavailable: {err}");
            None
        }
    }
}

/// Launch the QML frontend as a detached process.
pub fn launch_gui(page: Option<Page>) -> bool {
    let program = gui_program();
    let mut command = Command::new(&program);
    if let Some(page) = page {
        command.arg(format!("--page={}", page.as_arg()));
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    // A systemd user unit does not necessarily inherit the graphical
    // environment, so fill in the obvious gaps.
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        if let Some(display) = detect_wayland_display() {
            command.env("WAYLAND_DISPLAY", display);
        }
    }
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
            let bus = PathBuf::from(runtime).join("bus");
            if bus.exists() {
                command.env(
                    "DBUS_SESSION_BUS_ADDRESS",
                    format!("unix:path={}", bus.display()),
                );
            }
        }
    }
    if std::env::var_os("XDG_CURRENT_DESKTOP").is_none() {
        command.env("XDG_CURRENT_DESKTOP", "KDE");
    }
    if std::env::var_os("XDG_SESSION_TYPE").is_none() {
        command.env("XDG_SESSION_TYPE", "wayland");
    }

    match command.spawn() {
        Ok(child) => {
            tracing::info!("launched {} (pid {})", program.display(), child.id());
            true
        }
        Err(err) => {
            tracing::warn!("could not launch the GUI {}: {err}", program.display());
            false
        }
    }
}

fn gui_program() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("kdefusion");
            if sibling.is_file() {
                return sibling;
            }
        }
    }
    PathBuf::from("kdefusion")
}

fn detect_wayland_display() -> Option<String> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let mut names: Vec<String> = std::fs::read_dir(runtime)
        .ok()?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            name.starts_with("wayland-").then_some(name)
        })
        .collect();
    names.sort();
    names.pop()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_round_trips() {
        for page in [
            Page::Devices,
            Page::Session,
            Page::Settings,
            Page::Diagnostics,
        ] {
            assert_eq!(Page::from_arg(page.as_arg()), Some(page));
        }
        assert_eq!(Page::from_arg("nope"), None);
    }
}
