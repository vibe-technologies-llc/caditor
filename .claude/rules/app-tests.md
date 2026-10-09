---
paths:
  - "crates/caditor/src/ui_tests.rs"
  - "crates/caditor/src/ui_tests/**"
---

# UI test harness

- `ui_tests.rs` drives the real toolbars, panels and viewport through a headless egui context with
  synthetic input.
- Picking needs the GPU, so the harness answers pick requests as the renderer would: nothing
  under the cursor until a test hovers a chosen `Pickable` (`hover_pickable`); `picks_held` holds
  answers back like a slow GPU.
- With AccessKit enabled it keeps each frame's nodes, so tests check names, captions and that no
  visible node shows a private-use glyph. Icon-only buttons are clicked and hovered by accessible
  name (`click_button`, `hover_button`), labelled ones by their painted label.
- Modifier keys reach egui only through `Event::ModifiersChanged` (`click_with`); tree drags hold
  the button across frames (`hold_drag`, `release_drag`) so a test can read mid-drag state.
- Overlap checks compare only the visible part of each text (`Harness::text_clips`), since
  content scrolled under a bar is clipped there.
- `ui_tests/large_interface.rs` holds the 200% checks (panels beside the view, dialogs on screen,
  the compact ribbon, canvas readouts). A scene reaches what a user would scroll to with
  `wheel_until_shown` (`fully_shown`: the whole text inside its clip), clicks a ribbon tool with
  `click_tool` (its label, else its accessible name when the ribbon is compact) and reads a
  panel's rect with `panel_rect`. The focus scroll that brings a tree row into view is animated,
  so a test lets animations finish before clicking what it revealed.
- `settle` waits for the recompute and body meshing, runs two frames, and starts over while those
  frames started more (opening a feature meshes the body before it from a frame), so the scene
  after it is the finished one. `point_at`, and so `click_at`, settles first when a recompute or
  meshing is still running: its result changes the panels (a sketch's status pills), and a
  layout change landing between working out a screen position and the frame that reads the
  pointer would put the pointer somewhere else in the sketch.
- A constraint trial (`app-sketching.md`) is waited for at the end of each harness frame
  (`Model::finish_checking_constraints`), so a constraint button's result is in the document by
  the next check as if applied at once.
- Slow file work is staged with test-only seams on `Files`: `read_models_with` swaps the reader an
  import thread calls and `load_models_with` the loader an open thread calls
  (`ui_tests/import_jobs.rs` blocks either until released or cancelled).
- Opening a model resolves its path (`dunce::canonicalize`), and a Windows runner's temporary
  folder is an 8.3 short name (`C:\Users\RUNNER~1\...`) that resolving spells out in full, so a test
  comparing `Model::path` with a file it wrote builds that file under `canonical(&dir)`, never
  `dir.path()`; the comparison then holds on Linux and fails only on Windows CI.
- `Harness::pass` runs one frame without the text bookkeeping `frame` adds, so the ignored
  `app_frame_costs_while_editing_a_large_sketch_and_with_a_row_chosen` (`ui_tests/frame_costs.rs`)
  times the whole app pass (`app::show`, actions, scene build, pick answer) over `large_sketch`
  idle, with the pointer moving and with the line tool, and over the plate with a tree row
  chosen; `frame_costs_on_a_large_sketch_and_a_large_model` (`viewport.rs`) times the scene alone,
  a whole-body hover with the Bodies filter included.
- The renderer's upload state reaches the interface through `ViewportState::set_uploading`, which
  a test calls itself; `Harness::repaint_after` is the repaint delay egui reported for the last
  frame (`Duration::MAX` when nothing asked), so a test can hold that a widget's timer runs only
  while it is shown.
