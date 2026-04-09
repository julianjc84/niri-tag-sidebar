use std::cell::Cell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{self as gtk, glib};
use gtk4_layer_shell::{Edge as LayerEdge, KeyboardMode, Layer, LayerShell};

use crate::config::{Edge, LayerConfig, PanelConfig, PanelStyle};

/// Runtime state for a single panel.
pub struct Panel {
    pub config: PanelConfig,
    pub window: gtk::Window,
    /// Current reveal fraction: 0.0 = fully hidden, 1.0 = fully visible.
    /// Shared with animation tick callback so both stay in sync.
    pub reveal: Rc<Cell<f64>>,
    /// Whether the panel is logically "open" (will snap to 1.0).
    pub is_open: bool,
    /// Whether a gesture is actively driving this panel.
    pub gesture_active: bool,
    /// Animation tick callback ID (if animating).
    pub tick_id: Option<gtk::TickCallbackId>,
}

impl Panel {
    /// Create a new panel window using layer-shell.
    pub fn new(config: &PanelConfig, app: &gtk::Application) -> Self {
        match config.style {
            PanelStyle::Drawer => Self::new_drawer(config, app),
            PanelStyle::Bar => Self::new_bar(config, app),
        }
    }

    fn init_layer_shell_common(window: &gtk::Window, config: &PanelConfig) {
        window.init_layer_shell();
        window.set_layer(match config.layer {
            LayerConfig::Overlay => Layer::Overlay,
            LayerConfig::Top => Layer::Top,
            LayerConfig::Bottom => Layer::Bottom,
            LayerConfig::Background => Layer::Background,
        });
        window.set_keyboard_mode(KeyboardMode::None);
        window.set_exclusive_zone(config.exclusive_zone);
    }

    fn new_drawer(config: &PanelConfig, app: &gtk::Application) -> Self {
        let window = gtk::Window::builder()
            .application(app)
            .default_width(match config.edge {
                Edge::Left | Edge::Right => config.size,
                Edge::Top | Edge::Bottom => 1,
            })
            .default_height(match config.edge {
                Edge::Left | Edge::Right => 1,
                Edge::Top | Edge::Bottom => config.size,
            })
            .build();

        Self::init_layer_shell_common(&window, config);

        // Anchor to the panel's edge + stretch along that edge
        match config.edge {
            Edge::Left => {
                window.set_anchor(LayerEdge::Left, true);
                window.set_anchor(LayerEdge::Top, true);
                window.set_anchor(LayerEdge::Bottom, true);
                window.set_anchor(LayerEdge::Right, false);
            }
            Edge::Right => {
                window.set_anchor(LayerEdge::Right, true);
                window.set_anchor(LayerEdge::Top, true);
                window.set_anchor(LayerEdge::Bottom, true);
                window.set_anchor(LayerEdge::Left, false);
            }
            Edge::Top => {
                window.set_anchor(LayerEdge::Top, true);
                window.set_anchor(LayerEdge::Left, true);
                window.set_anchor(LayerEdge::Right, true);
                window.set_anchor(LayerEdge::Bottom, false);
            }
            Edge::Bottom => {
                window.set_anchor(LayerEdge::Bottom, true);
                window.set_anchor(LayerEdge::Left, true);
                window.set_anchor(LayerEdge::Right, true);
                window.set_anchor(LayerEdge::Top, false);
            }
        }

        // Set margin to hide the panel initially
        let initial_reveal = if config.start_open { 1.0 } else { 0.0 };
        let hidden_margin = if config.start_open { 0 } else { -config.size };

        match config.edge {
            Edge::Left => window.set_margin(LayerEdge::Left, hidden_margin),
            Edge::Right => window.set_margin(LayerEdge::Right, hidden_margin),
            Edge::Top => window.set_margin(LayerEdge::Top, hidden_margin),
            Edge::Bottom => window.set_margin(LayerEdge::Bottom, hidden_margin),
        }

        // Content
        let label_text = config
            .label
            .clone()
            .unwrap_or_else(|| format!("{:?} panel [{}]", config.edge, config.tag));
        let label = gtk::Label::new(Some(&label_text));
        label.add_css_class("panel-label");

        let info_label = gtk::Label::new(Some("Waiting for gesture..."));
        info_label.add_css_class("panel-info");
        info_label.set_widget_name("info-label");

        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 12);
        vbox.set_margin_top(20);
        vbox.set_margin_bottom(20);
        vbox.set_margin_start(20);
        vbox.set_margin_end(20);
        vbox.append(&label);
        vbox.append(&info_label);

        window.set_child(Some(&vbox));

        let css = format!(
            "window {{ background-color: {}; }} \
             .panel-label {{ font-size: 18px; font-weight: bold; color: white; }} \
             .panel-info {{ font-size: 14px; color: rgba(255,255,255,0.7); }}",
            config.bg_color
        );
        let provider = gtk::CssProvider::new();
        provider.load_from_data(&css);
        gtk::style_context_add_provider_for_display(
            &gtk::prelude::WidgetExt::display(&window),
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        if config.start_open {
            window.present();
        }

        Panel {
            config: config.clone(),
            window,
            reveal: Rc::new(Cell::new(initial_reveal)),
            is_open: config.start_open,
            gesture_active: false,
            tick_id: None,
        }
    }

    fn new_bar(config: &PanelConfig, app: &gtk::Application) -> Self {
        let bar_h = config.bar_height;

        let window = gtk::Window::builder()
            .application(app)
            .default_width(1)   // stretch via anchors
            .default_height(bar_h)
            .build();

        Self::init_layer_shell_common(&window, config);

        // Bar anchors to chosen edge + stretches horizontally
        match config.edge {
            Edge::Top => {
                window.set_anchor(LayerEdge::Top, true);
                window.set_anchor(LayerEdge::Left, true);
                window.set_anchor(LayerEdge::Right, true);
                window.set_anchor(LayerEdge::Bottom, false);
            }
            Edge::Bottom => {
                window.set_anchor(LayerEdge::Bottom, true);
                window.set_anchor(LayerEdge::Left, true);
                window.set_anchor(LayerEdge::Right, true);
                window.set_anchor(LayerEdge::Top, false);
            }
            Edge::Left => {
                window.set_anchor(LayerEdge::Left, true);
                window.set_anchor(LayerEdge::Top, true);
                window.set_anchor(LayerEdge::Bottom, true);
                window.set_anchor(LayerEdge::Right, false);
            }
            Edge::Right => {
                window.set_anchor(LayerEdge::Right, true);
                window.set_anchor(LayerEdge::Top, true);
                window.set_anchor(LayerEdge::Bottom, true);
                window.set_anchor(LayerEdge::Left, false);
            }
        }

        // Build bar content: [label] [===progress===] [percentage]
        let label_text = config
            .label
            .clone()
            .unwrap_or_else(|| config.tag.clone());

        let label = gtk::Label::new(Some(&label_text));
        label.add_css_class("bar-label");

        let progress_bar = gtk::ProgressBar::new();
        progress_bar.set_hexpand(true);
        progress_bar.set_fraction(0.0);
        progress_bar.set_widget_name("progress-bar");

        let pct_label = gtk::Label::new(Some("0%"));
        pct_label.add_css_class("bar-pct");
        pct_label.set_widget_name("info-label");
        pct_label.set_width_chars(5);

        let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        hbox.set_margin_top(4);
        hbox.set_margin_bottom(4);
        hbox.set_margin_start(12);
        hbox.set_margin_end(12);
        hbox.set_valign(gtk::Align::Center);
        hbox.append(&label);
        hbox.append(&progress_bar);
        hbox.append(&pct_label);

        window.set_child(Some(&hbox));

        // CSS
        let css = format!(
            "window {{ background-color: {}; }} \
             .bar-label {{ font-size: 14px; font-weight: bold; color: white; }} \
             .bar-pct {{ font-size: 14px; font-weight: bold; color: white; font-family: monospace; }} \
             progressbar trough {{ min-height: 14px; background-color: rgba(0,0,0,0.3); border-radius: 7px; }} \
             progressbar progress {{ min-height: 14px; background-color: rgba(255,255,255,0.9); border-radius: 7px; }}",
            config.bg_color
        );
        let provider = gtk::CssProvider::new();
        provider.load_from_data(&css);
        gtk::style_context_add_provider_for_display(
            &gtk::prelude::WidgetExt::display(&window),
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        // Bar starts hidden — only appears during active gestures
        // Don't present yet

        Panel {
            config: config.clone(),
            window,
            reveal: Rc::new(Cell::new(0.0)),
            is_open: false,
            gesture_active: false,
            tick_id: None,
        }
    }

    /// Set the panel's reveal fraction directly (during active gesture tracking).
    pub fn set_reveal(&mut self, fraction: f64) {
        self.reveal.set(fraction.clamp(0.0, 1.0));
        match self.config.style {
            PanelStyle::Drawer => {
                self.apply_margin();
                self.update_info_label();
            }
            PanelStyle::Bar => {
                // Show bar if not visible
                if !self.window.is_visible() {
                    self.window.present();
                }
                update_bar(&self.window, self.reveal.get());
            }
        }
    }

    /// Show the bar (for gesture begin on bar-style panels).
    pub fn show_bar(&mut self) {
        if self.config.style == PanelStyle::Bar && !self.window.is_visible() {
            self.window.present();
            update_bar(&self.window, 0.0);
        }
    }

    /// Hide the bar (for gesture end on bar-style panels).
    pub fn hide_bar(&mut self) {
        if self.config.style == PanelStyle::Bar {
            self.window.set_visible(false);
        }
    }

    /// Toggle open/closed state and animate to target.
    pub fn toggle(&mut self) {
        self.is_open = !self.is_open;
    }

    /// Snap the panel based on current reveal vs snap_threshold.
    pub fn snap(&mut self) {
        self.is_open = self.reveal.get() >= self.config.snap_threshold;
    }

    /// Start an animation towards the target state (open/closed).
    /// Uses GTK's frame clock tick callback for smooth 60fps animation.
    pub fn animate_to_target(&mut self) {
        // Remove any existing tick callback
        if let Some(id) = self.tick_id.take() {
            id.remove();
        }

        let target = if self.is_open { 1.0 } else { 0.0 };

        // If already at target, nothing to do
        if (self.reveal.get() - target).abs() < 0.001 {
            self.reveal.set(target);
            if target == 0.0 {
                self.window.set_visible(false);
            }
            return;
        }

        // Show the window before animating open
        if self.is_open && !self.window.is_visible() {
            // Reset margin to fully hidden before showing
            let hidden_margin = -self.config.size;
            match self.config.edge {
                Edge::Left => self.window.set_margin(LayerEdge::Left, hidden_margin),
                Edge::Right => self.window.set_margin(LayerEdge::Right, hidden_margin),
                Edge::Top => self.window.set_margin(LayerEdge::Top, hidden_margin),
                Edge::Bottom => self.window.set_margin(LayerEdge::Bottom, hidden_margin),
            }
            self.reveal.set(0.0);
            self.window.present();
        }

        let window = self.window.clone();
        let edge = self.config.edge;
        let size = self.config.size;

        // Shared with tick callback so panel.reveal stays in sync
        let reveal_shared = self.reveal.clone();
        let speed = 8.0; // Higher = faster snap animation

        let id = window.add_tick_callback(move |win, _clock| {
            let now = reveal_shared.get();
            let diff = target - now;

            if diff.abs() < 0.005 {
                // Close enough — snap to exact target
                reveal_shared.set(target);
                if target == 0.0 {
                    // Fully closed — hide the window entirely
                    win.set_visible(false);
                } else {
                    let margin = ((1.0 - target) * -(size as f64)) as i32;
                    match edge {
                        Edge::Left => win.set_margin(LayerEdge::Left, margin),
                        Edge::Right => win.set_margin(LayerEdge::Right, margin),
                        Edge::Top => win.set_margin(LayerEdge::Top, margin),
                        Edge::Bottom => win.set_margin(LayerEdge::Bottom, margin),
                    }
                }
                update_info(win, target);
                return glib::ControlFlow::Break;
            }

            // Ease towards target
            let step = diff * (speed / 60.0_f64).min(0.9);
            let new_val = (now + step).clamp(0.0, 1.0);
            reveal_shared.set(new_val);

            let margin = ((1.0 - new_val) * -(size as f64)) as i32;
            match edge {
                Edge::Left => win.set_margin(LayerEdge::Left, margin),
                Edge::Right => win.set_margin(LayerEdge::Right, margin),
                Edge::Top => win.set_margin(LayerEdge::Top, margin),
                Edge::Bottom => win.set_margin(LayerEdge::Bottom, margin),
            }
            update_info(win, new_val);

            glib::ControlFlow::Continue
        });

        self.tick_id = Some(id);
    }

    /// Apply the current reveal fraction as a layer-shell margin.
    fn apply_margin(&self) {
        let margin = ((1.0 - self.reveal.get()) * -(self.config.size as f64)) as i32;
        match self.config.edge {
            Edge::Left => self.window.set_margin(LayerEdge::Left, margin),
            Edge::Right => self.window.set_margin(LayerEdge::Right, margin),
            Edge::Top => self.window.set_margin(LayerEdge::Top, margin),
            Edge::Bottom => self.window.set_margin(LayerEdge::Bottom, margin),
        }
    }

    fn update_info_label(&self) {
        update_info(&self.window, self.reveal.get());
    }
}

fn update_info(window: &gtk::Window, reveal: f64) {
    if let Some(child) = window.child() {
        if let Some(vbox) = child.downcast_ref::<gtk::Box>() {
            if let Some(widget) = vbox.last_child() {
                if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                    label.set_text(&format!("Reveal: {:.0}%", reveal * 100.0));
                }
            }
        }
    }
}

fn update_bar(window: &gtk::Window, progress: f64) {
    if let Some(child) = window.child() {
        if let Some(hbox) = child.downcast_ref::<gtk::Box>() {
            // Walk children: label, progress bar, percentage label
            let mut child_widget = hbox.first_child();
            // Skip the label
            if let Some(ref w) = child_widget {
                child_widget = w.next_sibling();
            }
            // Progress bar
            if let Some(ref w) = child_widget {
                if let Some(bar) = w.downcast_ref::<gtk::ProgressBar>() {
                    bar.set_fraction(progress.abs().clamp(0.0, 1.0));
                }
                child_widget = w.next_sibling();
            }
            // Percentage label
            if let Some(ref w) = child_widget {
                if let Some(label) = w.downcast_ref::<gtk::Label>() {
                    label.set_text(&format!("{:.0}%", progress.abs() * 100.0));
                }
            }
        }
    }
}
