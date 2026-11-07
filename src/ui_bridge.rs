//! Coordination between the tray and the (possibly hidden) Kirigami window.
//!
//! The GUI polls `/api/status` every couple of seconds.  The daemon records
//! that heartbeat and can ask the running window to raise itself.  When no
//! window is polling, the tray launches a fresh frontend instead.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::tray::Page;

/// A GUI that has not polled for this long is considered closed.
const GUI_TIMEOUT_MS: u64 = 5000;
/// Avoid spawning two frontends when the user clicks twice during startup.
const LAUNCH_DEBOUNCE_MS: u64 = 3000;

#[derive(Debug, Default)]
pub struct UiBridge {
    last_seen_ms: AtomicU64,
    last_launch_ms: AtomicU64,
    open_request: AtomicU64,
    page: AtomicU64,
}

impl UiBridge {
    pub fn new() -> Self {
        Self::default()
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    /// Mark that a GUI is alive.  Called by `/api/status`.
    pub fn heartbeat(&self) {
        self.last_seen_ms.store(Self::now_ms(), Ordering::Relaxed);
    }

    pub fn gui_recently_seen(&self) -> bool {
        let last = self.last_seen_ms.load(Ordering::Relaxed);
        last > 0 && Self::now_ms().saturating_sub(last) < GUI_TIMEOUT_MS
    }

    /// Ask the running window to raise itself and navigate to `page`.
    pub fn request_open(&self, page: Option<Page>) {
        let code = match page {
            None => 0,
            Some(Page::Devices) => 1,
            Some(Page::Session) => 2,
            Some(Page::Settings) => 3,
            Some(Page::Diagnostics) => 4,
        };
        self.page.store(code, Ordering::Relaxed);
        self.open_request.fetch_add(1, Ordering::Relaxed);
    }

    /// True when a frontend was launched very recently.
    pub fn recently_launched(&self) -> bool {
        let last = self.last_launch_ms.load(Ordering::Relaxed);
        last > 0 && Self::now_ms().saturating_sub(last) < LAUNCH_DEBOUNCE_MS
    }

    pub fn mark_launched(&self) {
        self.last_launch_ms.store(Self::now_ms(), Ordering::Relaxed);
    }

    /// Snapshot for `/api/status`.
    pub fn snapshot(&self) -> (u64, &'static str) {
        let page = match self.page.load(Ordering::Relaxed) {
            1 => Page::Devices.as_arg(),
            2 => Page::Session.as_arg(),
            3 => Page::Settings.as_arg(),
            4 => Page::Diagnostics.as_arg(),
            _ => "",
        };
        (self.open_request.load(Ordering::Relaxed), page)
    }
}
