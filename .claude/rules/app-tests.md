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
- A constraint trial (`app-sketching.md`) is waited for at the end of each harness frame
  (`Model::finish_checking_constraints`), so a constraint button's result is in the document by
  the next check as if applied at once.
- Slow file work is staged with test-only seams on `Files`: `read_models_with` swaps the reader an
  import thread calls (`ui_tests/import_jobs.rs` blocks it until released or cancelled) and
  `hold_worker` parks the files worker so an open stays pending.
