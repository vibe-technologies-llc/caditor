---
name: ui-screenshots
description: Render screenshots of caditor's interface headlessly, or capture the running app on the desktop, to see a UI change before and after. Use when changing anything visual in crates/caditor (theme, widgets, panels, dialogs, bars, canvas labels) or when asked to show or check how the UI looks.
---

# UI screenshots

## Headless, from the test harness

`crates/caditor/src/ui_tests/screenshots.rs` holds an ignored test that drives the real
`app::show` through the UI harness, paints each frame over the 3D view and writes PNGs. It needs a
graphics adapter (lavapipe included) but no display.

```sh
CADITOR_SCREENSHOTS=<dir> cargo test -p caditor screenshots -- --ignored
CADITOR_SCREENSHOT_LOOKS=dark,light CADITOR_SCREENSHOTS=<dir> cargo test -p caditor screenshots -- --ignored
```

- Files are `<scene>-<look>.png`. Scenes come from `screenshots()` and the scene functions beside
  it; looks are the `LOOKS` table (theme, canvas, contrast, scale), and `CADITOR_SCREENSHOT_LOOKS`
  takes a comma-separated subset of their names. Read both for the current names.
- Write to the session scratchpad, never the repository. Take a set before a change and one after,
  and compare them with the Read tool.
- A new scene is a few lines in `screenshots()` or its area's scene function: put the harness in
  the state (the helpers in `ui_tests.rs` all work) and call `shoot`.

## The real app on the desktop

Under a running Plasma Wayland session:

```sh
WAYLAND_DISPLAY=wayland-0 XDG_RUNTIME_DIR=/run/user/$(id -u) cargo run -p caditor
WAYLAND_DISPLAY=wayland-0 XDG_RUNTIME_DIR=/run/user/$(id -u) spectacle -b -n -a -o <file>.png
```

`-a` captures only the active window; never use `-f` or a full-screen capture, which would record
the rest of the user's desktop. Input cannot be scripted there, so states needing clicks come from
the harness.
