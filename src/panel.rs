use std::cell::Cell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{self as gtk, gdk, glib};
use gtk4_layer_shell::{Edge as LayerEdge, KeyboardMode, Layer, LayerShell};

use crate::config::{Edge, LayerConfig, PanelConfig, PanelStyle, Zone};

/// Build a unique CSS class name from a panel tag.
///
/// Each panel registers its own CSS provider at display scope. Without a
/// per-panel selector, every provider's `window { background-color: ... }`
/// rule would match every panel's window, and the last one loaded would
/// win — every drawer ending up the same color. Scoping the selector to
/// `window.<class>` makes each rule apply only to its own panel.
///
/// Sanitizes the tag to CSS-identifier-safe characters (`[a-zA-Z0-9-]`)
/// and prefixes with `panel-` so it's guaranteed to start with a letter.
fn css_class_for_tag(tag: &str) -> String {
    let sanitized: String = tag
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
        .collect();
    format!("panel-{sanitized}")
}

/// Query the primary output's geometry so we can compute zone sizes.
///
/// Layer-shell panels don't always know which output they'll land on, and
/// GTK's monitor list can be empty during early window construction on some
/// compositors. Fall back to a 1920x1080 default in that case — the panel
/// will still render, just sized for a "typical" display.
fn primary_monitor_size() -> (i32, i32) {
    gdk::Display::default()
        .and_then(|d| d.monitors().item(0))
        .and_downcast::<gdk::Monitor>()
        .map(|m| {
            let g = m.geometry();
            (g.width(), g.height())
        })
        .unwrap_or((1920, 1080))
}

/// How big the panel is along the edge it's anchored to (parallel axis).
///
/// For Top/Bottom edges that's width; for Left/Right edges that's height.
/// `Full` returns the whole monitor extent; thirds return `extent / 3`.
fn parallel_extent(edge: Edge, zone: Zone, mon_w: i32, mon_h: i32) -> i32 {
    let extent = match edge {
        Edge::Top | Edge::Bottom => mon_w,
        Edge::Left | Edge::Right => mon_h,
    };
    match zone {
        Zone::Full => extent,
        Zone::Start | Zone::Center | Zone::End => extent / 3,
    }
}

/// Which corner to anchor a zoned panel to.
///
/// For Start zones, anchor to the "low" end of the parallel axis
/// (Top for horizontal edges, Left for vertical edges). For End zones,
/// anchor to the "high" end. Center anchors to the same corner as Start
/// but is then pushed inward by `extent/3` via a parallel-axis margin.
/// `Full` doesn't use this — it anchors both parallel sides.
///
/// Returns a tuple of (primary_edge, secondary_edge) where primary is the
/// panel's own edge and secondary is the parallel-axis anchor.
fn zoned_anchor_corner(edge: Edge, zone: Zone) -> (LayerEdge, LayerEdge) {
    let primary = match edge {
        Edge::Left => LayerEdge::Left,
        Edge::Right => LayerEdge::Right,
        Edge::Top => LayerEdge::Top,
        Edge::Bottom => LayerEdge::Bottom,
    };
    let secondary_low = match edge {
        Edge::Top | Edge::Bottom => LayerEdge::Left,
        Edge::Left | Edge::Right => LayerEdge::Top,
    };
    let secondary_high = match edge {
        Edge::Top | Edge::Bottom => LayerEdge::Right,
        Edge::Left | Edge::Right => LayerEdge::Bottom,
    };
    let secondary = match zone {
        Zone::Start | Zone::Center | Zone::Full => secondary_low,
        Zone::End => secondary_high,
    };
    (primary, secondary)
}

/// Apply the anchor + margin layout for a zoned panel.
///
/// This covers everything *except* the perpendicular hide-margin on the
/// primary edge (that's the slide animation's job). It sets:
///  - the primary edge anchor (always on)
///  - one parallel-axis anchor for Start/Center/End (both for Full)
///  - a parallel-axis margin for Center zones to push them into the middle
///
/// Must be called before the hide-margin is applied so the two don't
/// overwrite each other.
fn apply_zoned_anchors(
    window: &gtk::Window,
    edge: Edge,
    zone: Zone,
    parallel_offset: i32,
) {
    // Clear all anchors first — we'll turn on exactly the ones we want.
    window.set_anchor(LayerEdge::Left, false);
    window.set_anchor(LayerEdge::Right, false);
    window.set_anchor(LayerEdge::Top, false);
    window.set_anchor(LayerEdge::Bottom, false);

    match zone {
        Zone::Full => {
            // Anchor to the primary edge and stretch between both parallel
            // edges — the original full-width behavior.
            match edge {
                Edge::Left => {
                    window.set_anchor(LayerEdge::Left, true);
                    window.set_anchor(LayerEdge::Top, true);
                    window.set_anchor(LayerEdge::Bottom, true);
                }
                Edge::Right => {
                    window.set_anchor(LayerEdge::Right, true);
                    window.set_anchor(LayerEdge::Top, true);
                    window.set_anchor(LayerEdge::Bottom, true);
                }
                Edge::Top => {
                    window.set_anchor(LayerEdge::Top, true);
                    window.set_anchor(LayerEdge::Left, true);
                    window.set_anchor(LayerEdge::Right, true);
                }
                Edge::Bottom => {
                    window.set_anchor(LayerEdge::Bottom, true);
                    window.set_anchor(LayerEdge::Left, true);
                    window.set_anchor(LayerEdge::Right, true);
                }
            }
        }
        Zone::Start | Zone::Center | Zone::End => {
            let (primary, secondary) = zoned_anchor_corner(edge, zone);
            window.set_anchor(primary, true);
            window.set_anchor(secondary, true);
            // Center pushes the panel off its corner by one-third of the
            // parallel axis, via a margin on the same anchored parallel side.
            if zone == Zone::Center {
                window.set_margin(secondary, parallel_offset);
            }
        }
    }
}

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
    /// Whether a local GTK drag is actively closing this panel.
    pub drag_active: Rc<Cell<bool>>,
    /// Pending snap from drag-to-dismiss: Some(true) = snap open, Some(false) = snap closed.
    /// Consumed by the main loop poll.
    pub drag_snap_pending: Rc<Cell<Option<bool>>>,
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
        // Compute the panel's along-edge extent from monitor geometry and
        // the zone. Full zones stretch the whole edge; thirds get 1/3 the
        // monitor's parallel dimension. Perpendicular extent is always
        // `config.size` — the zone only affects the parallel axis.
        let (mon_w, mon_h) = primary_monitor_size();
        let parallel = parallel_extent(config.edge, config.zone, mon_w, mon_h);
        let parallel_offset = match config.edge {
            Edge::Top | Edge::Bottom => mon_w / 3,
            Edge::Left | Edge::Right => mon_h / 3,
        };

        let window = gtk::Window::builder()
            .application(app)
            .default_width(match config.edge {
                Edge::Left | Edge::Right => config.size,
                Edge::Top | Edge::Bottom => parallel,
            })
            .default_height(match config.edge {
                Edge::Left | Edge::Right => parallel,
                Edge::Top | Edge::Bottom => config.size,
            })
            .build();

        Self::init_layer_shell_common(&window, config);

        apply_zoned_anchors(&window, config.edge, config.zone, parallel_offset);

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

        // Scope CSS to this panel's own class so its background color
        // doesn't bleed into the other 11 drawers. See css_class_for_tag.
        let class_name = css_class_for_tag(&config.tag);
        window.add_css_class(&class_name);
        let css = format!(
            "window.{cls} {{ background-color: {}; }} \
             window.{cls} .panel-label {{ font-size: 18px; font-weight: bold; color: white; }} \
             window.{cls} .panel-info {{ font-size: 14px; color: rgba(255,255,255,0.7); }}",
            config.bg_color,
            cls = class_name,
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

        let reveal = Rc::new(Cell::new(initial_reveal));
        let drag_active = Rc::new(Cell::new(false));

        // Attach drag-to-dismiss gesture for closing the drawer by swiping back.
        let drag = gtk::GestureDrag::new();
        drag.set_touch_only(true);
        {
            let drag_active = drag_active.clone();
            drag.connect_drag_begin(move |_, _, _| {
                drag_active.set(true);
                eprintln!("[drag] BEGIN — local drag-to-dismiss started");
            });
        }
        {
            let reveal = reveal.clone();
            let edge = config.edge;
            let size = config.size;
            let window_ref = window.clone();
            drag.connect_drag_update(move |_, offset_x, offset_y| {
                // Map drag offset to reveal reduction.
                // Dragging toward the panel's edge = closing.
                let drag_distance = match edge {
                    Edge::Left => -offset_x,   // drag left to close
                    Edge::Right => offset_x,    // drag right to close
                    Edge::Top => -offset_y,     // drag up to close
                    Edge::Bottom => offset_y,   // drag down to close
                };
                let delta = drag_distance / size as f64;
                let new_reveal = (1.0 - delta).clamp(0.0, 1.0);
                reveal.set(new_reveal);

                let margin = ((1.0 - new_reveal) * -(size as f64)) as i32;
                match edge {
                    Edge::Left => window_ref.set_margin(LayerEdge::Left, margin),
                    Edge::Right => window_ref.set_margin(LayerEdge::Right, margin),
                    Edge::Top => window_ref.set_margin(LayerEdge::Top, margin),
                    Edge::Bottom => window_ref.set_margin(LayerEdge::Bottom, margin),
                }
                update_info(&window_ref, new_reveal);
            });
        }
        let drag_snap_pending = Rc::new(Cell::new(None));
        {
            let drag_active = drag_active.clone();
            let reveal = reveal.clone();
            let snap_threshold = config.snap_threshold;
            let drag_snap_pending = drag_snap_pending.clone();
            drag.connect_drag_end(move |_, _, _| {
                drag_active.set(false);
                let current = reveal.get();
                let should_open = current >= snap_threshold;
                eprintln!(
                    "[drag] END — reveal={:.3} threshold={:.2} → {}",
                    current, snap_threshold,
                    if should_open { "OPEN" } else { "CLOSED" },
                );
                drag_snap_pending.set(Some(should_open));
            });
        }
        window.add_controller(drag);

        Panel {
            config: config.clone(),
            window,
            reveal,
            is_open: config.start_open,
            gesture_active: false,
            drag_active,
            drag_snap_pending,
            tick_id: None,
        }
    }

    fn new_bar(config: &PanelConfig, app: &gtk::Application) -> Self {
        let bar_h = config.bar_height;

        // Same zone geometry as drawer: thirds split the parallel axis,
        // Center gets an offset margin to push it inward. Bars only have
        // `bar_height` as their perpendicular extent (not `size`).
        let (mon_w, mon_h) = primary_monitor_size();
        let parallel = parallel_extent(config.edge, config.zone, mon_w, mon_h);
        let parallel_offset = match config.edge {
            Edge::Top | Edge::Bottom => mon_w / 3,
            Edge::Left | Edge::Right => mon_h / 3,
        };

        let window = gtk::Window::builder()
            .application(app)
            .default_width(match config.edge {
                Edge::Left | Edge::Right => bar_h,
                Edge::Top | Edge::Bottom => parallel,
            })
            .default_height(match config.edge {
                Edge::Left | Edge::Right => parallel,
                Edge::Top | Edge::Bottom => bar_h,
            })
            .build();

        Self::init_layer_shell_common(&window, config);

        apply_zoned_anchors(&window, config.edge, config.zone, parallel_offset);

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

        // Scope CSS to this bar's own class so its background color
        // doesn't bleed into other panels. See css_class_for_tag.
        let class_name = css_class_for_tag(&config.tag);
        window.add_css_class(&class_name);
        let css = format!(
            "window.{cls} {{ background-color: {}; }} \
             window.{cls} .bar-label {{ font-size: 14px; font-weight: bold; color: white; }} \
             window.{cls} .bar-pct {{ font-size: 14px; font-weight: bold; color: white; font-family: monospace; }} \
             window.{cls} progressbar trough {{ min-height: 14px; background-color: rgba(0,0,0,0.3); border-radius: 7px; }} \
             window.{cls} progressbar progress {{ min-height: 14px; background-color: rgba(255,255,255,0.9); border-radius: 7px; }}",
            config.bg_color,
            cls = class_name,
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
            drag_active: Rc::new(Cell::new(false)),
            drag_snap_pending: Rc::new(Cell::new(None)),
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
