//! KScreen integration.
//!
//! KScreen is the single source of truth for display layout.  We never use
//! xrandr.  Layout changes are applied atomically through `kscreen-doctor` and
//! snapshots are plain JSON so they remain useful even after a crash.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

use crate::util::{atomic_write, now_unix, run_checked};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModeInfo {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub refresh: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OutputInfo {
    pub id: i64,
    pub name: String,
    pub enabled: bool,
    pub connected: bool,
    pub priority: i32,
    pub current_mode_id: String,
    pub modes: Vec<ModeInfo>,
    pub pos_x: i32,
    pub pos_y: i32,
    /// Logical size (mode size divided by scale).
    pub logical_width: u32,
    pub logical_height: u32,
    /// Physical mode size.
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub rotation: i32,
    pub replication_source: i64,
    pub clones: Vec<i64>,
}

impl OutputInfo {
    pub fn is_virtual(&self) -> bool {
        self.name.starts_with("Virtual-")
    }

    pub fn current_mode(&self) -> Option<&ModeInfo> {
        self.modes
            .iter()
            .find(|m| m.id == self.current_mode_id)
            .or_else(|| self.modes.first())
    }

    pub fn mode_for_size(&self, width: u32, height: u32) -> Option<&ModeInfo> {
        self.modes
            .iter()
            .filter(|m| m.width == width && m.height == height)
            .min_by(|a, b| {
                // Prefer ~60 Hz for desktop streaming, but stay deterministic.
                (a.refresh - 60.0)
                    .abs()
                    .partial_cmp(&(b.refresh - 60.0).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    pub fn rotation_name(&self) -> &'static str {
        match self.rotation {
            2 => "left",
            4 => "inverted",
            8 => "right",
            _ => "none",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KscreenSnapshot {
    pub taken_at: u64,
    pub outputs: Vec<OutputInfo>,
    pub raw: Value,
}

impl KscreenSnapshot {
    pub fn from_json(raw: Value) -> Result<Self> {
        let outputs = raw
            .get("outputs")
            .and_then(Value::as_array)
            .context("kscreen JSON has no outputs array")?
            .iter()
            .map(parse_output)
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            taken_at: now_unix(),
            outputs,
            raw,
        })
    }

    pub fn write(&self, path: &Path) -> Result<()> {
        let text = serde_json::to_vec_pretty(self).context("serialize KScreen snapshot")?;
        atomic_write(path, &text, Some(0o600))
    }

    pub fn read(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("read KScreen snapshot {}", path.display()))?;
        serde_json::from_str(&text)
            .with_context(|| format!("parse KScreen snapshot {}", path.display()))
    }

    pub fn physical_outputs(&self) -> Vec<&OutputInfo> {
        let mut outputs: Vec<_> = self
            .outputs
            .iter()
            .filter(|o| o.connected && !o.is_virtual())
            .collect();
        outputs.sort_by_key(|o| (o.priority == 0, o.priority, o.name.clone()));
        outputs
    }

    pub fn enabled_physical(&self) -> Vec<&OutputInfo> {
        self.physical_outputs()
            .into_iter()
            .filter(|o| o.enabled)
            .collect()
    }

    pub fn find(&self, name: &str) -> Option<&OutputInfo> {
        self.outputs.iter().find(|o| o.name == name)
    }

    pub fn find_virtual(&self, configured_name: &str) -> Option<&OutputInfo> {
        let expected = format!("Virtual-{configured_name}");
        self.outputs
            .iter()
            .find(|o| o.name == expected)
            .or_else(|| self.outputs.iter().find(|o| o.name.starts_with(&expected)))
            .or_else(|| self.outputs.iter().find(|o| o.is_virtual()))
    }

    /// Pick the physical output to mirror/extend.  An explicit name wins; the
    /// KScreen priority then decides.
    pub fn choose_physical(&self, configured: &str) -> Result<&OutputInfo> {
        if !configured.is_empty() {
            return self
                .find(configured)
                .with_context(|| format!("configured output {configured} not found"));
        }
        self.enabled_physical()
            .into_iter()
            .next()
            .context("no enabled physical output found")
    }
}

fn parse_output(value: &Value) -> Result<OutputInfo> {
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .context("output without name")?
        .to_string();
    let scale = value
        .get("scale")
        .and_then(Value::as_f64)
        .filter(|s| *s > 0.0)
        .unwrap_or(1.0);
    let size = value.get("size");
    let width = size
        .and_then(|s| s.get("width"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let height = size
        .and_then(|s| s.get("height"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let logical_width = ((width as f64) / scale).ceil() as u32;
    let logical_height = ((height as f64) / scale).ceil() as u32;
    let pos = value.get("pos");
    let modes = value
        .get("modes")
        .and_then(Value::as_array)
        .map(|modes| {
            modes
                .iter()
                .filter_map(|m| {
                    Some(ModeInfo {
                        id: m.get("id")?.as_str()?.to_string(),
                        name: m
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        width: m.get("size")?.get("width")?.as_u64()? as u32,
                        height: m.get("size")?.get("height")?.as_u64()? as u32,
                        refresh: m.get("refreshRate").and_then(Value::as_f64).unwrap_or(60.0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(OutputInfo {
        id: value.get("id").and_then(Value::as_i64).unwrap_or(0),
        name,
        enabled: value
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or_else(|| {
                // KScreen omits the `enabled` field for virtual outputs.  Treat a
                // connected output with an active mode as enabled.
                value
                    .get("connected")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                    && !value
                        .get("currentModeId")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .is_empty()
            }),
        connected: value
            .get("connected")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        priority: value.get("priority").and_then(Value::as_i64).unwrap_or(0) as i32,
        current_mode_id: value
            .get("currentModeId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        modes,
        pos_x: pos
            .and_then(|p| p.get("x"))
            .and_then(Value::as_i64)
            .unwrap_or(0) as i32,
        pos_y: pos
            .and_then(|p| p.get("y"))
            .and_then(Value::as_i64)
            .unwrap_or(0) as i32,
        logical_width,
        logical_height,
        width,
        height,
        scale,
        rotation: value.get("rotation").and_then(Value::as_i64).unwrap_or(1) as i32,
        replication_source: value
            .get("replicationSource")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        clones: value
            .get("clones")
            .and_then(Value::as_array)
            .map(|v| v.iter().filter_map(Value::as_i64).collect())
            .unwrap_or_default(),
    })
}

#[derive(Debug, Clone)]
pub struct Kscreen {
    program: String,
}

impl Default for Kscreen {
    fn default() -> Self {
        Self::new("kscreen-doctor")
    }
}

impl Kscreen {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
        }
    }

    pub async fn list(&self) -> Result<KscreenSnapshot> {
        let out = run_checked(&self.program, &["-j".to_string()])
            .await
            .context("kscreen-doctor is required (package: kscreen)")?;
        let raw: Value = serde_json::from_str(&out.stdout).context("parse kscreen-doctor JSON")?;
        KscreenSnapshot::from_json(raw)
    }

    pub async fn outputs_text(&self) -> Result<String> {
        Ok(run_checked(&self.program, &["-o".to_string()])
            .await?
            .stdout)
    }

    pub async fn apply(&self, args: &[String]) -> Result<()> {
        if args.is_empty() {
            return Ok(());
        }
        run_checked(&self.program, args).await?;
        Ok(())
    }

    pub async fn configure_mirror(&self, virtual_output: &str, physical: &str) -> Result<()> {
        validate_output_name(virtual_output)?;
        validate_output_name(physical)?;
        let args = vec![
            format!("output.{virtual_output}.enable"),
            format!("output.{physical}.enable"),
            format!("output.{virtual_output}.mirror.{physical}"),
        ];
        self.apply(&args).await
    }

    pub async fn configure_extend(
        &self,
        virtual_output: &str,
        mode_id: &str,
        x: i32,
        y: i32,
        physical: &str,
        physical_x: i32,
        physical_y: i32,
    ) -> Result<()> {
        validate_output_name(virtual_output)?;
        validate_output_name(physical)?;
        let mut args = vec![
            format!("output.{virtual_output}.mirror.none"),
            format!("output.{virtual_output}.mode.{mode_id}"),
            format!("output.{virtual_output}.scale.1"),
            format!("output.{virtual_output}.position.{x},{y}"),
            format!("output.{virtual_output}.enable"),
            // Keep the physical output pinned in place in the same atomic
            // transaction; otherwise KScreen may reposition it and the two
            // outputs end up overlapping (which looks like mirroring).
            format!("output.{physical}.position.{physical_x},{physical_y}"),
            format!("output.{physical}.enable"),
        ];
        if args.iter().any(|a| a.contains("..")) {
            bail!("invalid KScreen argument generated");
        }
        // Keep clippy happy about the mutable binding (some KScreen versions
        // need extra arguments appended in the future).
        args.shrink_to_fit();
        self.apply(&args).await
    }

    pub async fn disable_outputs(&self, outputs: &[&str]) -> Result<()> {
        let args: Vec<String> = outputs
            .iter()
            .map(|name| format!("output.{name}.disable"))
            .collect();
        self.apply(&args).await
    }

    pub async fn add_custom_mode(
        &self,
        output: &str,
        width: u32,
        height: u32,
        refresh_hz: f64,
    ) -> Result<()> {
        validate_output_name(output)?;
        let mhz = (refresh_hz * 1000.0).round() as u64;
        self.apply(&[format!(
            "output.{output}.addCustomMode.{width}.{height}.{mhz}.full"
        )])
        .await
    }

    pub async fn restore(&self, snapshot: &KscreenSnapshot) -> Result<()> {
        let mut enable_args = Vec::new();
        let mut layout_args = Vec::new();
        let id_to_name: std::collections::HashMap<i64, String> = snapshot
            .outputs
            .iter()
            .map(|o| (o.id, o.name.clone()))
            .collect();

        for output in &snapshot.outputs {
            validate_output_name(&output.name)?;
            if output.enabled {
                enable_args.push(format!("output.{}.enable", output.name));
                if !output.current_mode_id.is_empty() {
                    enable_args.push(format!(
                        "output.{}.mode.{}",
                        output.name, output.current_mode_id
                    ));
                }
                enable_args.push(format!(
                    "output.{}.scale.{}",
                    output.name,
                    format_scale(output.scale)
                ));
                enable_args.push(format!(
                    "output.{}.rotation.{}",
                    output.name,
                    output.rotation_name()
                ));
                layout_args.push(format!(
                    "output.{}.position.{},{}",
                    output.name, output.pos_x, output.pos_y
                ));
                if output.replication_source != 0 {
                    if let Some(source) = id_to_name.get(&output.replication_source) {
                        if source != &output.name {
                            layout_args.push(format!("output.{}.mirror.{}", output.name, source));
                        }
                    }
                } else {
                    layout_args.push(format!("output.{}.mirror.none", output.name));
                }
            } else {
                layout_args.push(format!("output.{}.disable", output.name));
            }
        }

        // Enable outputs and set their modes first.  Then apply positions and
        // mirror relationships.  This ordering is more forgiving when an output
        // temporarily disappeared (for example after unplugging a dock).
        let mut first = enable_args.clone();
        first.extend(layout_args.iter().cloned());
        if let Err(err) = self.apply(&first).await {
            tracing::warn!("atomic KScreen restore failed ({err}); retrying in phases");
            self.apply(&enable_args).await?;
            self.apply(&layout_args).await?;
        }
        Ok(())
    }
}

pub fn validate_output_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        bail!("invalid KScreen output name: {name}");
    }
    Ok(())
}

pub fn format_scale(scale: f64) -> String {
    if (scale - scale.round()).abs() < 0.0001 {
        format!("{}", scale.round() as i64)
    } else {
        let mut s = format!("{scale:.4}");
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
        s
    }
}

pub fn edge_position(
    physical: &OutputInfo,
    virtual_width: u32,
    virtual_height: u32,
    layout: crate::types::Layout,
) -> (i32, i32) {
    use crate::types::Layout;
    let pw = physical.logical_width as i32;
    let ph = physical.logical_height as i32;
    let vw = virtual_width as i32;
    let vh = virtual_height as i32;
    match layout {
        Layout::Right => (physical.pos_x + pw, physical.pos_y),
        Layout::Left => (physical.pos_x - vw, physical.pos_y),
        Layout::Top => (physical.pos_x, physical.pos_y - vh),
        Layout::Bottom => (physical.pos_x, physical.pos_y + ph),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Layout;

    const SAMPLE: &str = r#"{
      "outputs": [
        {
          "id": 1, "name": "eDP-1", "enabled": true, "connected": true,
          "currentModeId": "1", "priority": 1, "scale": 0.75,
          "pos": {"x": 0, "y": 0}, "size": {"width": 1366, "height": 768},
          "rotation": 1, "replicationSource": 0, "clones": [],
          "modes": [{"id":"1","name":"1366x768@60","refreshRate":60.0,
                     "size":{"width":1366,"height":768}}]
        },
        {
          "id": 2, "name": "Virtual-KDEFusion", "enabled": true, "connected": true,
          "currentModeId": "9", "priority": 2, "scale": 1,
          "pos": {"x": 1822, "y": 0}, "size": {"width": 1920, "height": 1080},
          "rotation": 1, "replicationSource": 0, "clones": [],
          "modes": [{"id":"9","name":"1920x1080@60","refreshRate":60.0,
                     "size":{"width":1920,"height":1080}}]
        }
      ]
    }"#;

    fn snapshot() -> KscreenSnapshot {
        KscreenSnapshot::from_json(serde_json::from_str(SAMPLE).unwrap()).unwrap()
    }

    #[test]
    fn parses_logical_geometry() {
        let snap = snapshot();
        let edp = snap.find("eDP-1").unwrap();
        assert_eq!(edp.width, 1366);
        assert_eq!(edp.logical_width, 1822);
        assert_eq!(edp.logical_height, 1024);
        assert_eq!(edp.rotation_name(), "none");
        assert_eq!(snap.enabled_physical().len(), 1);
        assert!(snap.find_virtual("KDEFusion").is_some());
    }

    #[test]
    fn chooses_physical_and_mode() {
        let snap = snapshot();
        assert_eq!(snap.choose_physical("").unwrap().name, "eDP-1");
        assert_eq!(snap.choose_physical("eDP-1").unwrap().name, "eDP-1");
        let v = snap.find_virtual("KDEFusion").unwrap();
        assert_eq!(v.mode_for_size(1920, 1080).unwrap().id, "9");
        assert!(v.mode_for_size(1280, 720).is_none());
    }

    #[test]
    fn computes_edge_positions() {
        let snap = snapshot();
        let edp = snap.find("eDP-1").unwrap();
        assert_eq!(edge_position(edp, 1920, 1080, Layout::Right), (1822, 0));
        assert_eq!(edge_position(edp, 1920, 1080, Layout::Left), (-1920, 0));
        assert_eq!(edge_position(edp, 1920, 1080, Layout::Top), (0, -1080));
        assert_eq!(edge_position(edp, 1920, 1080, Layout::Bottom), (0, 1024));
    }

    #[test]
    fn validates_names() {
        assert!(validate_output_name("Virtual-KDEFusion").is_ok());
        assert!(validate_output_name("eDP-1").is_ok());
        assert!(validate_output_name("bad name").is_err());
        assert!(validate_output_name("x;rm").is_err());
    }

    #[test]
    fn formats_scales() {
        assert_eq!(format_scale(1.0), "1");
        assert_eq!(format_scale(0.75), "0.75");
        assert_eq!(format_scale(1.25), "1.25");
        assert_eq!(format_scale(1.333333), "1.3333");
    }
}
