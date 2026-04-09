use serde::Deserialize;
use std::path::PathBuf;

/// Top-level config loaded from TOML.
#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    /// Panels to create. Each panel is an independent slide-out drawer.
    pub panel: Vec<PanelConfig>,
}

/// Which screen edge a panel slides from.
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

/// Which layer-shell layer the panel renders on.
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum LayerConfig {
    /// Above everything, including waybar and other bars.
    #[default]
    Overlay,
    /// Same layer as waybar — respects its exclusive zone.
    Top,
    /// Below normal windows.
    Bottom,
    /// Desktop background level.
    Background,
}

/// Display style for a panel.
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum PanelStyle {
    /// Full slide-out drawer panel.
    #[default]
    Drawer,
    /// Thin progress bar with percentage readout — shows during active gestures only.
    Bar,
}

/// Configuration for a single slide-out panel.
#[derive(Debug, Deserialize, Clone)]
pub struct PanelConfig {
    /// The niri gesture tag this panel responds to (must match `tag="..."` in niri config).
    pub tag: String,

    /// Which edge of the screen this panel slides from.
    pub edge: Edge,

    /// Size of the panel in pixels (width for left/right, height for top/bottom).
    #[serde(default = "default_size")]
    pub size: i32,

    /// Progress threshold (0.0..1.0) at which the panel snaps open on gesture end.
    /// Below this, it snaps closed.
    #[serde(default = "default_snap_threshold")]
    pub snap_threshold: f64,

    /// Background color as a CSS color string (e.g. "rgba(30, 30, 46, 0.9)").
    #[serde(default = "default_bg_color")]
    pub bg_color: String,

    /// Whether the panel starts in the open state.
    #[serde(default)]
    pub start_open: bool,

    /// Optional label text to display in the panel.
    pub label: Option<String>,

    /// Exclusive zone: how many pixels to reserve (push other windows aside).
    /// 0 = overlay mode (no exclusive zone). -1 = auto (match panel size).
    #[serde(default)]
    pub exclusive_zone: i32,

    /// Layer-shell layer: "overlay" draws above everything (including waybar),
    /// "top" shares the layer with waybar. Default: "overlay".
    #[serde(default = "default_layer")]
    pub layer: LayerConfig,

    /// Panel display style: "drawer" (full slide-out) or "bar" (thin progress indicator).
    /// Default: "drawer".
    #[serde(default)]
    pub style: PanelStyle,

    /// For bar style: height of the bar in pixels. Default: 40.
    #[serde(default = "default_bar_height")]
    pub bar_height: i32,
}

fn default_size() -> i32 {
    300
}

fn default_snap_threshold() -> f64 {
    0.5
}

fn default_bg_color() -> String {
    "rgba(30, 30, 46, 0.85)".to_string()
}

fn default_layer() -> LayerConfig {
    LayerConfig::Overlay
}

fn default_bar_height() -> i32 {
    40
}

impl Config {
    /// Load config from a TOML file path.
    pub fn load(path: &PathBuf) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config {}: {}", path.display(), e))?;
        toml::from_str(&content)
            .map_err(|e| format!("Failed to parse config {}: {}", path.display(), e))
    }

    /// Load default sample config (used when no config file is provided).
    pub fn sample() -> Self {
        Config {
            panel: vec![
                PanelConfig {
                    tag: "sidebar-left".to_string(),
                    edge: Edge::Left,
                    size: 400,
                    snap_threshold: 0.4,
                    bg_color: "rgba(40, 100, 220, 0.95)".to_string(),
                    start_open: false,
                    label: Some("Navigation".to_string()),
                    exclusive_zone: 0,
                    layer: LayerConfig::Overlay,
                    style: PanelStyle::Drawer,
                    bar_height: 40,
                },
                PanelConfig {
                    tag: "sidebar-right".to_string(),
                    edge: Edge::Right,
                    size: 400,
                    snap_threshold: 0.5,
                    bg_color: "rgba(220, 50, 50, 0.95)".to_string(),
                    start_open: false,
                    label: Some("Quick Settings".to_string()),
                    exclusive_zone: 0,
                    layer: LayerConfig::Overlay,
                    style: PanelStyle::Drawer,
                    bar_height: 40,
                },
            ],
        }
    }
}
