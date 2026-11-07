//! Shared session types used by the control protocol, HTTP API and QML UI.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SessionMode {
    #[default]
    Mirror,
    Extend,
    Headless,
}

impl fmt::Display for SessionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SessionMode::Mirror => "mirror",
            SessionMode::Extend => "extend",
            SessionMode::Headless => "headless",
        })
    }
}

impl FromStr for SessionMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "mirror" | "clone" => Ok(SessionMode::Mirror),
            "extend" | "extended" => Ok(SessionMode::Extend),
            "headless" | "virtual" => Ok(SessionMode::Headless),
            other => Err(format!(
                "unknown mode '{other}' (expected mirror, extend or headless)"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    Left,
    #[default]
    Right,
    Top,
    Bottom,
}

impl fmt::Display for Layout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Layout::Left => "left",
            Layout::Right => "right",
            Layout::Top => "top",
            Layout::Bottom => "bottom",
        })
    }
}

impl FromStr for Layout {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "left" => Ok(Layout::Left),
            "right" => Ok(Layout::Right),
            "top" | "above" => Ok(Layout::Top),
            "bottom" | "below" => Ok(Layout::Bottom),
            other => Err(format!(
                "unknown layout '{other}' (expected left, right, top or bottom)"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Idle,
    Preparing,
    CreatingDisplay,
    ConfiguringDisplay,
    StartingSunshine,
    Pairing,
    Launching,
    Streaming,
    Stopping,
    Failed,
}

impl Phase {
    pub fn is_busy(self) -> bool {
        matches!(
            self,
            Phase::Preparing
                | Phase::CreatingDisplay
                | Phase::ConfiguringDisplay
                | Phase::StartingSunshine
                | Phase::Pairing
                | Phase::Launching
                | Phase::Stopping
        )
    }

    pub fn is_active(self) -> bool {
        self == Phase::Streaming
    }
}

/// A request made by the GUI or CLI to the local daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionRequest {
    pub peer_id: String,
    pub mode: SessionMode,
    pub layout: Layout,
    /// `None` means fullscreen (default).
    pub display_mode: Option<String>,
    /// `None` means use the mode default from config.
    pub audio: Option<bool>,
    /// Optional override such as `1920x1080`.
    pub resolution: String,
    pub fps: Option<u32>,
    pub bitrate_kbps: Option<u32>,
    pub codec: Option<String>,
    pub app: Option<String>,
}

impl Default for SessionRequest {
    fn default() -> Self {
        Self {
            peer_id: String::new(),
            mode: SessionMode::Mirror,
            layout: Layout::Right,
            display_mode: None,
            audio: None,
            resolution: String::new(),
            fps: None,
            bitrate_kbps: None,
            codec: None,
            app: None,
        }
    }
}

/// Wire request sent from the host that wants to stream to the viewer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartStream {
    pub session_id: String,
    pub mode: SessionMode,
    pub layout: Layout,
    pub display_mode: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub audio: bool,
    pub codec: String,
    pub app: String,
    pub host_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionStatus {
    pub phase: Phase,
    pub session_id: String,
    pub peer_id: String,
    pub peer_name: String,
    pub mode: SessionMode,
    pub layout: Layout,
    pub audio: bool,
    pub width: u32,
    pub height: u32,
    pub detail: String,
    pub error: String,
    pub started_at: u64,
    pub role: String,
}

impl Default for SessionStatus {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            session_id: String::new(),
            peer_id: String::new(),
            peer_name: String::new(),
            mode: SessionMode::Mirror,
            layout: Layout::Right,
            audio: false,
            width: 0,
            height: 0,
            detail: "Idle".to_string(),
            error: String::new(),
            started_at: 0,
            role: String::new(),
        }
    }
}

impl SessionStatus {
    pub fn busy(&self) -> bool {
        self.phase.is_busy()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PairState {
    Pending,
    Approved,
    Rejected,
    Expired,
    NotFound,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_parse() {
        assert_eq!(
            "MIRROR".parse::<SessionMode>().unwrap(),
            SessionMode::Mirror
        );
        assert_eq!(
            "extended".parse::<SessionMode>().unwrap(),
            SessionMode::Extend
        );
        assert_eq!(
            "headless".parse::<SessionMode>().unwrap(),
            SessionMode::Headless
        );
        assert!("bogus".parse::<SessionMode>().is_err());
    }

    #[test]
    fn layouts_parse() {
        assert_eq!("above".parse::<Layout>().unwrap(), Layout::Top);
        assert_eq!("below".parse::<Layout>().unwrap(), Layout::Bottom);
        assert_eq!(Layout::default(), Layout::Right);
    }

    #[test]
    fn status_serializes_phase_as_snake_case() {
        let status = SessionStatus {
            phase: Phase::StartingSunshine,
            ..Default::default()
        };
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(value["phase"], "starting_sunshine");
    }
}
