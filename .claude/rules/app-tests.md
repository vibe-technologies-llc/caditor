---
paths:
  - "crates/caditor/src/ui_tests.rs"
  - "crates/caditor/src/ui_tests/**"
---

# UI test harness

- `ui_tests.rs` drives the real toolbars, panels and viewport through a headless egui context with
  synthetic input. It holds only the harness, the helpers more than one topic uses and the `mod`
  list; every test lives in a topic module under `ui_tests/`, with the helpers only that topic
  uses, so parallel changes meet in a small file rather than a 20,000-line one.
- Picking needs the GPU, so the harness answers pick requests as the renderer would: nothing under
  the cursor until a test hovers a chosen `Pickable` (`hover_pickable`); `picks_held` holds answers
  back like a slow GPU. It answers sharing the built scene's `PickTable`, never cloning the scene.
- AccessKit keeps each frame's nodes, so tests check names, captions and that no visible node shows
  a private-use glyph. Icon-only buttons are clicked and hovered by accessible name
  (`click_button`, `hover_button`), labelled ones by their painted label. A test reading
  accessibility at only a few screens leaves AccessKit off and calls `Harness::read_screen` before
  each check (one frame with it on).
- Modifier keys reach egui only through `Event::ModifiersChanged` (`click_with`); tree drags hold
  the button across frames (`hold_drag`, `release_drag`) to read mid-drag state.
- Overlap checks compare only the visible part of each text (`Harness::text_clips`).
- `ui_tests/large_interface.rs` holds the 200% checks. `wheel_until_shown` scrolls to what a user
  would (`fully_shown`: the whole text inside its clip), `click_tool` clicks a ribbon tool (its
  label, else its accessible name when the ribbon is compact), `panel_rect` reads a panel's rect.
  The focus scroll to a tree row is animated: let animations finish before clicking what it
  revealed.

## Sweeps and guards

- `Harness::pass` fails a frame in which a command was triggered after the handler offering it had
  run (`Workspace::dropped_commands`, `CommandFrame::triggered_after_their_handler`): the click was
  left in the frame's list and dropped, as the menu bar's gear once was. Commands with no offer in
  the frame are not counted, since the palette refuses those beforehand.
- `ui_tests/command_sweep.rs` queues every `Command` (`Workspace::queue_command`) in a fresh
  harness over the sample model, in four shares that run in parallel: none may panic, none may be
  unavailable with an empty reason, and each window one opens (found by the new visible layers)
  is laid out again at 75% and 200% and must stay on screen, with no text cut at the side by its
  clip and no two texts overlapping. Side panels are not windows; `large_interface.rs` covers them.
- `ui_tests/button_sweep.rs` clicks every enabled, fully visible button (by accessible name, once
  each) in the idle sample model and in the sketch editor, each in a fresh harness: none may
  panic, and the dropped-command guard above does the rest, so a button whose command nothing reads
  fails here without a test of its own. It would have caught the gear.
- `ui_tests/interleavings.rs` plays seeded sequences of edits, undo, redo and held recomputes
  (`Model::hold_evaluations`; only undo, redo and frames while held, since `settle` never returns
  with a hold on) and checks that, once released, the model shows the evaluation a fresh
  `Recompute` of its document gives (feature states; a failure's words are not compared).

## Waiting

- `settle` waits for the recompute, body meshing and requested mass properties
  (`Model::masses_pending`), runs two frames, and repeats while those started more work, so the
  scene after it is the finished one. `point_at` (so `click_at`) and `open_combo` settle first when
  work is running, because a result landing between working out a position and the press shifts
  the panel; `open_combo` also holds the pointer still until the list stops moving.
- A harness's model wakes count into `Harness::wakes`; `frames_while_woken` runs a frame only after
  a wake or queued input, as the app does, until a second passes without one, so a test sees what
  a still pointer would (`a_distance_typed_after_extruding_by_key_is_previewed_without_moving_the_pointer`).
- `settle` does not wait for workers outside the model; a test waits for them itself: Measure's
  readout (`Measurements::is_measuring`), the journal (`Model::flush_journal` before reading the
  recovery folder), the file worker (`Files::wait_for_jobs`), and a harness's storage worker
  (`Model::close` and `Closing::wait` before another harness opens the same model, since Windows
  locks the journal until the worker lets go).
- The view can move between frames without input: a notice the storage worker reports (a harness
  has no recovery folder, so the first change reports one whenever the worker gets to it) or a
  progress line changes the status bar's height and so the view's. A drag aims afresh each frame
  (`drag_in_sketch` takes the sketch point, not a screen position fixed before the press).
- A value typed into a sketch field right after an edit waits for `settle` first:
  `Model::settled_sketch` sees none while it recomputes.
- A constraint trial is waited for at the end of each frame (`Model::finish_checking_constraints`),
  so its result is in the document by the next check. `Model::hold_sketch_results` holds back a
  trial's verdict and the drag worker's solutions (`ui_tests/stale_sketch_results.rs`);
  `Model::hold_evaluations` holds back recompute evaluations (release them before `settle`).
- A paste finding its picks runs on the recompute worker: `Model::finish_pasting` waits for it and
  applies it, `Model::hold_pastes` holds its result back so a test can see it running or cancel it
  (`ui_tests/paste_picks.rs`).
- A `bodies::Hold` keeps background work waiting, at its kernel interrupt, until released, so a
  test can see it running and cancel it: `Model::hold_body_work` holds the body pool's conversions
  or mass properties (`Holds`; `settle` never returns while one is held and pending), and
  `Analyses::hold_on_a_thread` (through `ViewportState::analyses_mut`) sends every face analysis
  to its thread and holds it there.
- `Model::stop_storage_worker` stops the model's storage worker on command (`Storage::stop`,
  `caditor-file`'s `test-support` feature), leaving its journal in place; `ui_tests/storage_worker.rs`
  covers a failed record, a poll finding the worker gone, a save that cannot be sent and quitting.
  The next frame replaces a stopped worker, so a test needing the dead one acts on the model
  before it. Waiting for the replacement means waiting for a single new journal file before
  inspecting it: `caditor_file::inspect` holds the lock while it decodes, and a replacement that
  finds the old journal locked leaves it behind.
- `Harness::render_preview` hands `Files::preview_rendered` solid rows of `PICTURE_SIZE`, so a test
  waits for `Preview::is_drawn` without a GPU.
- File work is staged with test-only seams on `Files`: `read_models_with` (import thread's reader),
  `load_models_with` (open thread's loader; `ui_tests/import_jobs.rs` blocks either until released
  or cancelled) and `scan_with` (startup scan, set before `Files::start` through
  `Harness::starting_with`; `HeldScan` holds its report or a journal's lock). Asserting that a
  cancelled or abandoned result was dropped waits for `Files::own_threads_delivered` and a frame,
  never a fixed sleep.
- Opening a model resolves its path (`dunce::canonicalize`) and a Windows runner's 8.3 temporary
  folder (`C:\Users\RUNNER~1\...`) is spelled out, so a test comparing `Model::path` with a file it
  wrote builds that file under `canonical(&dir)`, never `dir.path()`; otherwise it fails only on
  Windows CI.
- `Harness::repaint_after` is the repaint delay egui reported for the last frame (`Duration::MAX`
  when nothing asked); `ViewportState::set_uploading` and `set_upload_stopped` are called by the
  test itself, which reads the renderer's side with `take_upload_request`.
- `body_reach::REACH_BUDGET`, the UI thread's 150 ms wall-clock allowance for deciding what a new
  extrusion reaches, is a minute under test, so a loaded machine never runs it out in the middle of
  a detection test (`ui_tests/extrusion_detection.rs`); what a run-out says is covered by the unit
  test on `extrusion_detection::extrusion` with an unevaluated model.

## Timing tests

`Harness::pass` runs one frame without `frame`'s text bookkeeping. All of these are ignored or
bench-only:

- `app_frame_costs_while_editing_a_large_sketch_and_with_a_row_chosen` (`ui_tests/frame_costs.rs`)
  times the whole app pass over `large_sketch` idle, with the pointer moving and with the line tool,
  and over the plate with a tree row chosen.
- `frame_costs_on_a_large_sketch_and_a_large_model` (`viewport.rs`) times the scene alone.
  `frame_costs_on_a_large_assembly` (`viewport.rs`) settles 1,600 copies of the samples, or the STEP
  file `CADITOR_IMPORT_FILE` names, times the scene idle and with the hover changing, and with a
  graphics adapter draws 1920×1080 frames at 4x through its own `ViewportRenderer`, each split into
  scene, UI thread drawing and GPU wait. `CADITOR_BENCH_CASE` runs only cases whose name contains
  it.
- `a_large_step_import_is_timed_stage_by_stage` (`ui_tests/import_costs.rs`; the nightly stress job
  runs it) times a STEP import stage by stage (read, added, saved and reopened, recomputed,
  meshed, measured, drawn), then through the harness from the Import command to
  every body shown, listing each frame over 50 ms. It synthesises a file asserted to stay within
  `SYNTHESISED_WITHIN`, or times `CADITOR_IMPORT_FILE`; `CADITOR_IMPORT_IN_THE_APP_ONLY` skips the
  stages. After the camera's fit finishes (`CAMERA_SETTLE`; no pick is asked while the viewpoint
  moves) it times idle frames, pointer moves, hovering between two faces taken from the drawn
  `PickTable` (`forced_hover`), orbit, pan and zoom, each change until settled (`timed_change`:
  face click, clearing the selection, feature row click, hiding a body and undoing it through
  `app::perform`, opening the Bodies group), then scrolling the feature tree and the Bodies group.
