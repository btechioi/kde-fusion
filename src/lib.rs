//! KDE Fusion: mirror or extend a KDE Plasma Wayland desktop over Sunshine and
//! Moonlight, with a native Kirigami frontend.
//!
//! The library contains all daemon logic.  The binaries are thin:
//! `kdefusiond` (daemon), `kdefusionctl` (CLI) and `kdefusion` (QML launcher).

pub mod api;
pub mod config;
pub mod control;
pub mod diagnostics;
pub mod discovery;
pub mod installer;
pub mod kscreen;
pub mod moonlight;
pub mod notify;
pub mod platform;
pub mod secrets;
pub mod session;
pub mod state;
pub mod sunshine;
pub mod tray;
pub mod types;
pub mod ui_bridge;
pub mod util;
pub mod virtual_display;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const APP_NAME: &str = "KDE Fusion";
pub const QML_DIR_ENV: &str = "KDEFUSION_QML_DIR";
pub const QML_RUNTIME_ENV: &str = "KDEFUSION_QML_RUNTIME";
