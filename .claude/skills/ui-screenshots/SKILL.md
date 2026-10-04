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

- Files are `<scene>-<look>.png`. Scenes are shot by `screenshots()` and the scene functions
  beside it (canvas, tree, feature panels, dialogs); read them for the current names. Looks are the
  `LOOKS` table (dark, light, high contrast, 150% and 200% scale); `CADITOR_SCREENSHOT_LOOKS` takes
  a comma-separated subset of their names.
- Write them to the session scratchpad, never into the repository. Take a set before a change and
  one after, and compare the two with the Read tool.
- A new scene is a few lines in `screenshots()` or the scene function for its area: put the
  harness in the state (the helpers in `ui_tests.rs` all work) and call `shoot`.

## The real app on the desktop

When a Plasma Wayland session is running, the app can be started in it and captured from the shell:

```sh
WAYLAND_DISPLAY=wayland-0 XDG_RUNTIME_DIR=/run/user/$(id -u) cargo run -p caditor
WAYLAND_DISPLAY=wayland-0 XDG_RUNTIME_DIR=/run/user/$(id -u) spectacle -b -n -a -o <file>.png
```

`-a` captures only the active window. Never use `-f` or a full-screen capture, which would record
whatever else is on the user's desktop. Input cannot be scripted there, so states that need clicks
are better taken from the harness.
