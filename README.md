# niri-tag-sidebar

Gesture-driven slide-out panels for the niri Wayland compositor. Binds
`gtk4-layer-shell` drawers to niri IPC gesture tags — swipe in from an
edge and the matching panel follows your finger; release past the snap
threshold and it latches open, release below it and it slides back.

## Why this exists (Wayland gesture visibility)

Under Wayland, input is owned by the compositor. Touch events are
routed to the focused surface, but *compositor-level* gestures —
edge swipes, multi-finger pinches, workspace swipes — are consumed
by the compositor itself and never reach external clients. There is
no standard protocol for an app to say "tell me when the user did a
3-finger swipe on the left edge", so touch-driven HUDs, drawers, and
shortcut overlays are effectively impossible to build on most
Wayland compositors without being the compositor.

niri's tag-based gesture IPC inverts that: the compositor still owns
recognition, but every tagged bind republishes its begin / progress /
end events over the IPC socket. Any external client that subscribes
can react in real time, which is enough to drive slide animations,
latching drawers, gesture-mapped shortcuts, etc. — all from an
ordinary layer-shell client.

`niri-tag-sidebar` is a **prototype** that exercises that contract.
It's deliberately minimal (one TOML config, layer-shell drawers, no
content beyond a colored panel + label) and exists to prove the
pattern works end-to-end: compositor-owned gesture recognition + IPC
tags = touch UI built outside the compositor.

## How it works

1. niri-tag-sidebar reads `~/.config/niri-tag-sidebar/niri-tag-sidebar.toml`
   and spawns one layer-shell drawer per `[[panel]]` entry.
2. It connects to niri's IPC socket (`$NIRI_SOCKET`) and subscribes to
   `GestureBegin` / `GestureProgress` / `GestureEnd` events.
3. Each panel is bound to a `tag` that must match a `tag="..."`
   property on a gesture bind in your niri config. Progress events
   for that tag drive the drawer's slide animation in real time.

The panels use only the `progress` field from `GestureProgress`; the
typed `delta` payload (Swipe/Pinch/Rotate) is ignored.

## niri config

Tag your edge-swipe binds with values that match panels in the TOML
config. Example:

```kdl
binds {
    TouchEdgeLeft:Center  tag="zone-left-center"  { noop; }
    TouchEdgeRight:Top    tag="zone-right-top"    { noop; }
    TouchEdgeBottom:Full  tag="drawer-bottom"     { noop; }
}
```

The bind action itself can be `noop` — the sidebar only needs the tag
to match. See `sample-config.toml` for a full 12-zone example covering
every edge × {start, center, end} zone.

## Build / install

```bash
./install_niri_tag_sidebar.sh
```

Builds in debug, installs the binary to `/usr/local/bin/niri-tag-sidebar`,
and drops `sample-config.toml` at
`~/.config/niri-tag-sidebar/niri-tag-sidebar.toml` (only if it doesn't
already exist — the script never overwrites your config).

Run `niri-tag-sidebar` to launch. It logs gesture events to stderr for
debugging.

## Config reference

Per-panel settings (`sample-config.toml` has the canonical list):

| key              | default                          | meaning                                                              |
|------------------|----------------------------------|----------------------------------------------------------------------|
| `tag`            | —                                | gesture tag to listen for (required)                                 |
| `edge`           | —                                | `left` / `right` / `top` / `bottom` (required)                       |
| `zone`           | `full`                           | `full` / `start` / `center` / `end` — which third of the edge        |
| `size`           | `300`                            | panel size in px perpendicular to the edge                           |
| `snap_threshold` | `0.5`                            | progress value (0–1) above which release latches the drawer open     |
| `bg_color`       | `rgba(30, 30, 46, 0.85)`         | CSS color                                                            |
| `label`          | —                                | optional text shown in the panel                                     |
| `start_open`     | `false`                          | whether the drawer starts latched open                               |
| `exclusive_zone` | `0`                              | px reserved from the output (0 = overlay, doesn't reflow windows)    |
| `layer`          | `overlay`                        | `overlay` / `top` / `bottom` / `background`                          |

## Status

Proof-of-concept. Requires a niri build with the configurable touch
gesture work (continuous gesture IPC, `TouchEdge:<Zone>` triggers,
typed `delta` payload) — i.e. the `feat/configurable-touch-gestures`
branch of [julianjc84/niri](https://github.com/julianjc84/niri).
