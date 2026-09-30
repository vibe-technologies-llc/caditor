---
paths:
  - "crates/caditor/src/ui_tests.rs"
---

# UI test harness

- `ui_tests.rs` drives the real toolbars, panels and viewport through a headless egui context with
  synthetic input.
- Picking needs the GPU, so the harness answers each pick request as the renderer would: nothing
  under the cursor unless a test hovers a chosen `Pickable` (`hover_pickable`, through
  `hover_through_pick`) until the pointer moves; `picks_held` holds answers back like a slow GPU.
- With `enable_accesskit` it keeps each frame's AccessKit nodes, so tests check names, captions and
  that no visible node shows a private-use glyph. Buttons showing only an icon, such as the sketch
  ribbon's constraints, are clicked and hovered by accessible name (`click_button`,
  `hover_button`, which turn AccessKit on when needed); labelled ones by their painted label.
- Tests may set the viewport selection directly; drawing tests click sketch positions mapped to the
  screen through the view, annotation tests the painted labels and glyphs.
