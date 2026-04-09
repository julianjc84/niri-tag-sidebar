#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "=== Building niri-tag-sidebar (debug) ==="
cd "$SCRIPT_DIR"
cargo build

echo ""
echo "=== Stopping running instance ==="
killall niri-tag-sidebar 2>/dev/null && echo "Stopped." || echo "Not running."

echo ""
echo "=== Installing niri-tag-sidebar ==="
sudo cp "$SCRIPT_DIR/target/debug/niri-tag-sidebar" /usr/local/bin/niri-tag-sidebar

echo ""
echo "=== Installing sample config ==="
mkdir -p "$HOME/.config/niri-tag-sidebar"
if [ ! -f "$HOME/.config/niri-tag-sidebar/niri-tag-sidebar.toml" ]; then
    cp "$SCRIPT_DIR/sample-config.toml" "$HOME/.config/niri-tag-sidebar/niri-tag-sidebar.toml"
    echo "Installed sample config to ~/.config/niri-tag-sidebar/niri-tag-sidebar.toml"
else
    echo "Config already exists at ~/.config/niri-tag-sidebar/niri-tag-sidebar.toml (not overwritten)"
fi

echo ""
echo "Done! Run 'niri-tag-sidebar' to open the sidebar panels."
echo "Edit ~/.config/niri-tag-sidebar/niri-tag-sidebar.toml to configure panels."
