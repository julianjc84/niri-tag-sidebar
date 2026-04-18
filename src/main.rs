mod config;
mod ipc;
mod panel;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{self as gtk, glib};

use config::{Config, PanelStyle};
use ipc::{GestureMsg, spawn_ipc_listener};
use panel::Panel;

fn main() {
    let config = load_config();

    let app = gtk::Application::builder()
        .application_id("com.niri.tag-sidebar")
        .build();

    app.connect_activate(move |app| {
        build_ui(app, &config);
    });

    app.run_with_args::<String>(&[]);
}

fn load_config() -> Config {
    // Check CLI args for --config <path>
    let args: Vec<String> = std::env::args().collect();
    for i in 0..args.len() {
        if (args[i] == "--config" || args[i] == "-c")
            && let Some(path) = args.get(i + 1)
        {
            match Config::load(&PathBuf::from(path)) {
                Ok(c) => {
                    eprintln!("[niri-tag-sidebar] Loaded config from {}", path);
                    return c;
                }
                Err(e) => {
                    eprintln!("[niri-tag-sidebar] {}", e);
                    std::process::exit(1);
                }
            }
        }
    }

    // Try default location
    if let Some(config_dir) = dirs_path() {
        let default_path = config_dir.join("niri-tag-sidebar/niri-tag-sidebar.toml");
        if default_path.exists() {
            match Config::load(&default_path) {
                Ok(c) => {
                    eprintln!(
                        "[niri-tag-sidebar] Loaded config from {}",
                        default_path.display()
                    );
                    return c;
                }
                Err(e) => {
                    eprintln!("[niri-tag-sidebar] Warning: {}", e);
                }
            }
        }
    }

    eprintln!("[niri-tag-sidebar] No config file found, using built-in sample config");
    Config::sample()
}

fn dirs_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
}

fn build_ui(app: &gtk::Application, config: &Config) {
    // Collect all tags we need to subscribe to
    let tags: Vec<String> = config.panel.iter().map(|p| p.tag.clone()).collect();

    eprintln!(
        "[niri-tag-sidebar] Creating {} panel(s) for tags: {:?}",
        config.panel.len(),
        tags
    );

    // Create panels, indexed by tag
    let panels: Rc<RefCell<HashMap<String, Panel>>> = Rc::new(RefCell::new(HashMap::new()));

    for panel_config in &config.panel {
        let panel = Panel::new(panel_config, app);
        panels.borrow_mut().insert(panel_config.tag.clone(), panel);
    }

    // Start IPC listener
    let rx = spawn_ipc_listener(tags);

    // Poll the IPC channel and drag-to-dismiss signals from the GTK main loop
    let panels_clone = panels.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(8), move || {
        while let Ok(msg) = rx.try_recv() {
            handle_gesture_msg(&panels_clone, msg);
        }
        // Check for pending drag-to-dismiss snaps.
        for panel in panels_clone.borrow_mut().values_mut() {
            if let Some(should_open) = panel.drag_snap_pending.take() {
                panel.is_open = should_open;
                panel.animate_to_target();
            }
        }
        glib::ControlFlow::Continue
    });
}

fn handle_gesture_msg(panels: &Rc<RefCell<HashMap<String, Panel>>>, msg: GestureMsg) {
    let mut panels = panels.borrow_mut();

    match msg {
        GestureMsg::Begin {
            tag,
            trigger,
            finger_count,
            is_continuous,
        } => {
            eprintln!(
                "[gesture] BEGIN tag={} trigger={} fingers={} continuous={}",
                tag, trigger, finger_count, is_continuous
            );

            if let Some(panel) = panels.get_mut(&tag) {
                // Cancel any running animation
                if let Some(id) = panel.tick_id.take() {
                    id.remove();
                }
                panel.gesture_active = true;

                if panel.config.style == PanelStyle::Bar {
                    // Bar: show immediately for both continuous and discrete
                    panel.show_bar();
                    if !is_continuous {
                        // Discrete bar — show briefly then hide
                        panel.set_reveal(1.0);
                        panel.gesture_active = false;
                    }
                } else if !is_continuous {
                    // Drawer: discrete gesture — just toggle
                    panel.toggle();
                    panel.gesture_active = false;
                    panel.animate_to_target();
                } else {
                    // Drawer: continuous gesture — present the window so
                    // Progress events can drive the reveal via set_reveal().
                    if !panel.window.is_visible() {
                        panel.window.present();
                    }
                }
            }
        }
        GestureMsg::Progress { tag, progress } => {
            eprintln!("[gesture] PROGRESS tag={} progress={:.3}", tag, progress);
            if let Some(panel) = panels.get_mut(&tag)
                && panel.gesture_active
                && !panel.drag_active.get()
            {
                // Map progress to reveal.
                // If panel is currently open, invert so the gesture closes it.
                let reveal = if panel.is_open {
                    (1.0 - progress.abs()).clamp(0.0, 1.0)
                } else {
                    progress.abs().clamp(0.0, 1.0)
                };
                panel.set_reveal(reveal);
            }
        }
        GestureMsg::End { tag, completed } => {
            eprintln!("[gesture] END tag={} completed={}", tag, completed);

            if let Some(panel) = panels.get_mut(&tag) {
                if panel.config.style == PanelStyle::Bar {
                    // Bar: hide when gesture ends
                    panel.gesture_active = false;
                    panel.hide_bar();
                } else if panel.gesture_active {
                    // Drawer: continuous gesture ended — snap to final state
                    // without animation since the gesture already drove the reveal.
                    panel.gesture_active = false;
                    let reveal_before = panel.reveal.get();
                    panel.snap();
                    eprintln!(
                        "[gesture] SNAP tag={} reveal={:.3} threshold={:.2} → {}",
                        tag,
                        reveal_before,
                        panel.config.snap_threshold,
                        if panel.is_open { "OPEN" } else { "CLOSED" },
                    );
                    let target = if panel.is_open { 1.0 } else { 0.0 };
                    panel.set_reveal(target);
                    if !panel.is_open {
                        panel.window.set_visible(false);
                    }
                }
            }
        }
        GestureMsg::Disconnected(reason) => {
            eprintln!("[niri-tag-sidebar] IPC disconnected: {}", reason);
        }
    }
}
