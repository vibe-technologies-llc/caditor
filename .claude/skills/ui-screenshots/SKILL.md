---
name: ui-screenshots
description: Render screenshots of caditor's interface headlessly, or capture the running app on the desktop, to see a UI change before and after. Use when changing anything visual in crates/caditor (theme, widgets, panels, dialogs, bars, canvas labels) or when asked to show or check how the UI looks.
---

# UI screenshots

## Headless, from the test harness

`crates/caditor/src/ui_tests/screenshots.rs` holds an ignored test that drives the real
`app::show` through the UI harness and paints each frame with `egui_wgpu` over the 3D view drawn by
`caditor_render::ViewportRenderer`, then writes PNGs. It needs a graphics adapter (any Vulkan or GL
device, lavapipe included) but no display.

```sh
CADITOR_SCREENSHOTS=<dir> cargo test -p caditor screenshots -- --ignored
CADITOR_SCREENSHOT_LOOKS=dark,light CADITOR_SCREENSHOTS=<dir> cargo test -p caditor screenshots -- --ignored
```

- Files are `<scene>-<look>.png`, 1400×1000 physical pixels. Scenes: `welcome`, `empty`, `model`
  (the Angle bracket sample), `feature` (its extrusion open), `measure`, `sketch` (its profile
  edited), `palette`, `preferences`, `export`, and from `dialog_scenes`: `preferences-<tab>` for
  each tab, `shortcuts`, `about`, `image-export`, `history`, `unsaved` (the unsaved-changes
  prompt), `report` (a damaged file's report), `tip`, `welcome-recent` (with recent files) and
  `recovery`.
- Looks: `dark`, `light`, `dark-contrast`, `light-contrast`, `dark-200`, `light-150`;
  `CADITOR_SCREENSHOT_LOOKS` takes a comma-separated subset.
- Write them to the session scratchpad, never into the repository. Take a set before a change and
  one after, and compare the two with the Read tool.
- A new scene is a few lines in `screenshots()` (or `dialog_scenes()` for a dialog): put the harness in the state (the helpers in
  `ui_tests.rs` all work) and call `shoot`.

## The real app on the desktop

When a Plasma Wayland session is running, the app can be started in it and captured from the shell:

```sh
WAYLAND_DISPLAY=wayland-0 XDG_RUNTIME_DIR=/run/user/$(id -u) cargo run -p caditor
WAYLAND_DISPLAY=wayland-0 XDG_RUNTIME_DIR=/run/user/$(id -u) spectacle -b -n -a -o <file>.png
```

`-a` captures only the active window. Never use `-f` or a full-screen capture, which would record
whatever else is on the user's desktop. Input cannot be scripted there, so states that need clicks
are better taken from the harness.
