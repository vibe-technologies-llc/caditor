# Comprehensive Codebase Audit: Bugs & UX Issues

This audit evaluates the entire Caditor parametric CAD codebase (~630,000 lines of Rust across 15 crates) for functional bugs, numerical stability hazards, transaction and persistence integrity flaws, and user experience (UX) violations against the design principles in `.claude/rules/` (`ux.md`, `reliability.md`, `document.md`, `kernel.md`, `file-format.md`, `sketch-solver.md`).

The audit was conducted via five specialized subagent investigations covering distinct subsystems:
1. **User Interface, Navigation & Accessibility** (`crates/caditor`)
2. **Parametric Document, Transactions & Worker Concurrency** (`crates/caditor-document`, `crates/caditor/src`)
3. **File Persistence, Journaling & Import/Export** (`crates/caditor-file`, `caditor-step`, `caditor-zstd`)
4. **2D Sketch Constraints, Solver & Unit Expressions** (`crates/caditor-sketch`, `caditor-expression`)
5. **Computational B-Rep Kernel, Tessellation & Rendering** (`crates/caditor-kernel`, `caditor-geometry`, `caditor-render`)

---

## Executive Summary & Findings Matrix

A total of **39 concrete issues** were identified across the codebase:
- **Critical (5)**: Unblocked shortcut execution & background mutations during Home overlay; 6 modal dialogs failing to block shortcuts/file drops; interactive drag-solver tangency branch inversion; Windows mandatory file locks preventing journal discard/replacement; silent dropped intersection curves on shallow plane-cylinder cuts.
- **High (10)**: Escape and Enter key bubbling into drawings/dialogs; broken `footer_split` button layout; empty sketch removal rollback failure; background mesh starvation in recompute worker; missing coordinate `.is_finite()` checks; catastrophic cancellation in cone ellipse minor radius; coordinate precision loss in near-parallel plane intersections; omission of Y variables in zero-length line collapse detection; non-atomic backup placeholder race on Windows; orphaned constraint dimension parameters on sketch reshape.
- **Medium (14)**: Terminal recompute outcome clobbering; silent deletion of disconnected files from recent list; implicit base-unit coercion for plain numbers in expressions; spatial hash collapse on extreme coordinates; 3MF uncompressed size underestimation; singular vertex normal cache corruption; ray-plane boundary chatter; missing circle collapse diagnostics; division by zero in `line_offset`; shortcut editor trapped confirmation state; un-cancellable cloud loading; configuration stale cell tracking; orphaned version deltas after thinning; auxiliary variable DOF distortion.
- **Low (10)**: Readback buffer alignment assertion; malformed ZIP unchecked integer addition; STEP product ID collision on duplicate body names; ArcLength residual discontinuity; sparse Givens null-space sensitivity; cycle dependent parameter error reporting; recovery card button alignment; status pill missing icons; feature tree drag cancel escape bubbling; datum placing name collisions.

---

## 1. User Interface, Experience & Accessibility (`crates/caditor`)

### Finding 1.1: Home Screen Overlay Does Not Block Hotkeys or Viewport Mutations
- **Type**: UX Issue / Functional Bug
- **Severity**: **Critical**
- **Location**: [`crates/caditor/src/app.rs:606-622, 693-697`](file:///home/tecnio/workspace/caditor/crates/caditor/src/app.rs#L606-L622), [`crates/caditor/src/viewport.rs:1418-1462, 5707-5710`](file:///home/tecnio/workspace/caditor/crates/caditor/src/viewport.rs#L1418-L1462)
- **Description**:
  When the Home screen is open (`home.is_open()`), it renders as an overlay `Area` over the viewport. However, `home.is_open()` is omitted from `modal_open` and `blocked` in `app.rs:606-622`. Consequently, `blocked` is `false` and `keys_free` is `true`. Shortcut keys (e.g. `L` for Line, `E` for Extrude, `Ctrl+Z` for Undo) continue to dispatch commands directly to the underlying document model.
  Furthermore, `viewport.cover(home.is_open())` only disables `show_start_card` in `viewport.rs:5708`. It does not inhibit mouse hover tracking or `viewport::handle_keys`. If a user presses `Delete` or `Backspace` while browsing recent files on the Home screen, the selected entities or features in the underlying model are deleted.
- **Impact**: Silent, accidental document modification or data loss while the user believes they are in a modal home/recent file view.
- **Recommendation**:
  1. Include `|| workspace.home.is_open()` in `modal_open` in `app.rs:619`.
  2. Early-return from `viewport::handle_keys` and viewport hover/click handling when `self.covered` is `true`.

---

### Finding 1.2: Six Modal Dialogs Omitted from `is_blocking()`, Leaking Shortcuts and File Drops
- **Type**: UX Issue / State Integrity
- **Severity**: **Critical**
- **Location**: [`crates/caditor/src/files.rs:1300-1310, 1631-1637`](file:///home/tecnio/workspace/caditor/crates/caditor/src/files.rs#L1300-L1310), [`crates/caditor/src/app.rs:606-622`](file:///home/tecnio/workspace/caditor/crates/caditor/src/app.rs#L606-L622)
- **Description**:
  In `files.rs:1300-1310`, `Files::is_blocking()` enumerates blocking workflows but omits:
  - `self.exporter.is_open()` (3D Export dialog)
  - `self.image.is_open()` (Image Export dialog)
  - `self.drawing.is_open()` (Drawing Export dialog)
  - `self.history.is_open()` (Version History dialog)
  - `self.parameter_import.is_some()` (Parameter Import dialog)
  - `self.configuration_import.is_some()` (Configuration Import dialog)
  Likewise, `modal_open` in `app.rs` omits all six.
  While any of these dialogs is displayed, model shortcut keys remain active. Furthermore, if a file is dragged and dropped onto the window while the Export or History dialog is open, `files.dropped` checks `self.is_blocking()`, evaluates `false`, and replaces or alters the open model beneath the modal dialog.
- **Impact**: Accidental mutation, feature deletion, or unexpected model replacement while configuring an export or browsing versions.
- **Recommendation**:
  Add all six dialog conditions to `Files::is_blocking()` in `files.rs:1300-1310`.

---

### Finding 1.3: `footer_split` Misplaces Destructive/Reset Actions Next to Primary Buttons
- **Type**: UX Issue / Layout Bug
- **Severity**: **High**
- **Location**: [`crates/caditor/src/widgets.rs:2528-2540`](file:///home/tecnio/workspace/caditor/crates/caditor/src/widgets.rs#L2528-L2540)
- **Description**:
  Under `.claude/rules/app-look.md:322`, `footer_split` must place destructive/reset actions at the far left edge of a dialog, isolating them from rightmost primary buttons.
  However, `widgets::footer` delegates to `Sides::new().show(ui, |_| {}, add).1`, putting all content into the right column of `Sides`. Inside `footer_split`, `start` is executed inside a `with_layout(Layout::left_to_right)` within the right-hand column, squeezing the destructive button directly adjacent to the confirm/save buttons.
- **Impact**: Users intending to click "Save" or "Done" risk misclicking adjacent destructive actions ("Close without saving", "Reset all").
- **Recommendation**:
  Implement `footer_split` using `Sides::new().show(ui, start, end)` directly:
  ```rust
  pub fn footer_split<T>(
      ui: &mut Ui,
      start: impl FnOnce(&mut Ui) -> Option<T>,
      end: impl FnOnce(&mut Ui) -> Option<T>,
  ) -> Option<T> {
      ui.add_space(DIALOG_FOOTER_GAP);
      let (started, ended) = Sides::new().show(ui, start, end);
      ended.or(started)
  }
  ```

---

### Finding 1.4: `typed_point.rs` Does Not Consume Enter and Escape Keys
- **Type**: UX Issue / Input Handling
- **Severity**: **High**
- **Location**: [`crates/caditor/src/typed_point.rs:405-422`](file:///home/tecnio/workspace/caditor/crates/caditor/src/typed_point.rs#L405-L422), [`crates/caditor/src/viewport.rs:4980-5010, 5295-5325`](file:///home/tecnio/workspace/caditor/crates/caditor/src/viewport.rs#L4980-L5010)
- **Description**:
  In `typed_point.rs:405`, `input.key_pressed(Key::Enter)` and `input.key_pressed(Key::Escape)` are inspected without calling `input.consume_key(...)`.
  1. If a user presses `Escape` while typing coordinates, `typed_point` closes. In the same frame, `viewport::handle_keys` receives the unconsumed `Escape`, sees `typed_point.is_waiting() == false`, and calls `self.drawing.cancel(...)`, discarding the user's active drawing tool and deleting drawn geometry.
  2. If a user presses `Enter` to submit the first point of a multi-point tool (spline, polyline), `typed_point` emits the point. In the same frame, `viewport::handle_keys` detects unconsumed `Enter` and invokes `self.drawing.finish(...)`, prematurely closing the polyline.
- **Impact**: Data loss and frustrating disruptions during precision keyboard drawing workflows.
- **Recommendation**:
  Consume both keys using `ctx.input_mut(|input| input.consume_key(Modifiers::NONE, key))`.

---

### Finding 1.5: Reverting Input Fields via Escape Closes Enclosing Modal Dialogs
- **Type**: UX Issue / Focus & Key Handling
- **Severity**: **High**
- **Location**: [`crates/caditor/src/field.rs:265-270`](file:///home/tecnio/workspace/caditor/crates/caditor/src/field.rs#L265-L270)
- **Description**:
  In `field.rs:266`, `ui.input(|input| input.key_pressed(Key::Escape))` tests for Escape without consuming it.
  When typing an expression or value inside any modal dialog, pressing Escape to revert edits triggers `response.lost_focus()`. Because Escape is unconsumed, the enclosing egui `Modal` also processes Escape in `response.should_close()`, immediately closing the entire dialog and discarding all other fields.
- **Impact**: Violates `.claude/rules/app-input.md:235-238` ("widgets::escape_cancels consumes the key so the modal does not close").
- **Recommendation**:
  Replace `ui.input` with `ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))`.

---

### Finding 1.6: Cancelling Key Recording Closes Entire Shortcut Editor Dialog
- **Type**: UX Issue
- **Severity**: **High**
- **Location**: [`crates/caditor/src/shortcut_editor.rs:104-116, 252-254`](file:///home/tecnio/workspace/caditor/crates/caditor/src/shortcut_editor.rs#L104-L116)
- **Description**:
  When capturing a new key combination in the Shortcut Editor (`editor.recording == Some(command)`), pressing Escape clears `self.recording = None` in `capture` without consuming Escape. Line 252 then checks `(!editor.is_recording() && response.should_close())`. Since `is_recording()` was just cleared, this evaluates to `true`, instantly dismissing the Shortcut Editor dialog.
- **Impact**: Frustrating UX when attempting to abort binding a key.
- **Recommendation**:
  Consume `Key::Escape` inside `editor.capture` when cancelling recording, and guard line 252 with `was_recording`.

---

### Finding 1.7: Shortcut Editor Reset Confirmation Traps State on Escape
- **Type**: UX Issue / Modal State
- **Severity**: **Medium**
- **Location**: [`crates/caditor/src/shortcut_editor.rs:211-225`](file:///home/tecnio/workspace/caditor/crates/caditor/src/shortcut_editor.rs#L211-L225)
- **Description**:
  Clicking "Reset all shortcuts" sets `editor.confirm_reset = true`. Pressing Escape triggers `response.should_close()` and exits the dialog without resetting `confirm_reset = false`. When the user later reopens the Shortcut Editor, the "Reset all shortcuts?" confirmation is stuck open.
- **Impact**: Stale modal confirmation state trapping user interactions across dialog sessions.
- **Recommendation**:
  Consume Escape to dismiss the confirmation prompt first, and ensure `confirm_reset = false` on dialog close.

---

### Finding 1.8: Opening Disconnected Files Silently Wipes Them from Recent Models
- **Type**: UX Issue / Data Retention
- **Severity**: **Medium**
- **Location**: [`crates/caditor/src/files.rs:2880-2893`](file:///home/tecnio/workspace/caditor/crates/caditor/src/files.rs#L2880-L2893)
- **Description**:
  When a user opens a file from an unmounted USB drive, disconnected network share, or offline VPN path, `OpenOutcome::Failed` calls `self.forget(&path)`. The path is immediately and permanently deleted from `recent.paths()`.
- **Impact**: Once the drive or network is reconnected, the file is missing from Recent Models with no way to recover the entry.
- **Recommendation**:
  Retain the path in `missing_recent`, present an informative toast/card explaining the path is currently unavailable, and provide an explicit "Remove from recents" button.

---

### Finding 1.9: Conflicting Primary Buttons & Un-Cancellable Loading in Cloud Dialog
- **Type**: UX Issue
- **Severity**: **Medium**
- **Location**: [`crates/caditor/src/cloud_dialog.rs:170-193, 524-527`](file:///home/tecnio/workspace/caditor/crates/caditor/src/cloud_dialog.rs#L170-L193)
- **Description**:
  1. When cloud is disabled, `footer_split` places `widgets::primary_button(ui, TURN_ON)` on the left and `widgets::primary_button(ui, "Close")` on the right, violating the requirement that a dialog has exactly one primary action, placed rightmost.
  2. Background folder retrieval shows `widgets::progress("Loading designs…").show(ui)` without a `.cancel(...)` button, locking the dialog if network requests stall.
- **Impact**: Inconsistent button visual hierarchy; modal lock on poor network connections.
- **Recommendation**:
  Convert `TURN_ON` to a standard button and attach a cancellation handle to the loading progress widget.

---

### Finding 1.10: Status Pills Omit Status Icon in `menu_bar.rs`
- **Type**: UX / Accessibility Violation
- **Severity**: **Low**
- **Location**: [`crates/caditor/src/menu_bar.rs:416-418`](file:///home/tecnio/workspace/caditor/crates/caditor/src/menu_bar.rs#L416-L418)
- **Description**:
  Save status in the menu bar uses `widgets::pill(ui, Tone::Warning, "Unsaved changes")` instead of `widgets::status_pill`. `widgets::pill` lacks an icon, violating `.claude/rules/ux.md` ("No state is told by colour alone; status reads the same everywhere: an icon and a tone").
- **Recommendation**:
  Switch to `widgets::status_pill`.

---

### Finding 1.11: Primary "Restore" Button Placed Leftmost on Recovery Card
- **Type**: UX / Design System
- **Severity**: **Low**
- **Location**: [`crates/caditor/src/files/dialogs.rs:410-424`](file:///home/tecnio/workspace/caditor/crates/caditor/src/files/dialogs.rs#L410-L424)
- **Description**:
  In `recovery_card`, `ui.horizontal_wrapped` places the primary "Restore" button leftmost, followed by "Restore suppressed" and "Discard…". Under Caditor design standards, primary confirmative actions belong rightmost.
- **Recommendation**:
  Reorder buttons to position "Restore" rightmost.

---

### Finding 1.12: Feature Tree Drag Cancel Leaves Unconsumed Escape
- **Type**: UX Issue / Input Bubbling
- **Severity**: **Low**
- **Location**: [`crates/caditor/src/feature_tree.rs:609-619`](file:///home/tecnio/workspace/caditor/crates/caditor/src/feature_tree.rs#L609-L619)
- **Description**:
  Cancelling feature tree reordering checks `input.key_pressed(Key::Escape)` without consuming it, allowing Escape to propagate to the viewport and clear selections.
- **Recommendation**:
  Consume Escape via `input.consume_key(Modifiers::NONE, Key::Escape)`.

---

## 2. Parametric Document, Transactions & Modeling (`caditor-document`)

### Finding 2.1: `remove_empty` Discards Empty Sketches with Bare `RemoveFeature`, Failing at Rollback Bar
- **Type**: Functional Bug / Transaction Failure
- **Severity**: **High**
- **Location**: [`crates/caditor/src/editing.rs:776-782`](file:///home/tecnio/workspace/caditor/crates/caditor/src/editing.rs#L776-L782)
- **Description**:
  When an empty sketch is exited, `remove_empty` generates:
  ```rust
  Discard::Apply(Transaction::single(label, Edit::RemoveFeature { id: entered.feature }))
  ```
  If the rollback bar is located at this feature (`RollbackBar::Before(entered.feature)`), `edit.rs:1568` rejects the transaction with `EditError::RollbackBarAbove`. The user cannot close or cancel the empty sketch. Furthermore, bare `RemoveFeature` bypasses rollback bar movements (`bar_moves`), owned parameter cleanup (`releasing`), and stand-in carrier removal.
- **Impact**: UI gets stuck inside an empty sketch with unresolvable errors if the user undid actions or adjusted the rollback bar.
- **Recommendation**:
  Use `model.document().deletion(&[entered.feature], label)` instead of direct `Edit::RemoveFeature`.

---

### Finding 2.2: Worker Background Mesh Queue Starves Indefinitely on Tessellation Failure
- **Type**: Functional Bug / Concurrency & Worker
- **Severity**: **High**
- **Location**: [`crates/caditor-document/src/worker.rs:694-706`](file:///home/tecnio/workspace/caditor/crates/caditor-document/src/worker.rs#L694-L706)
- **Description**:
  In `mesh_pending`, background mesh jobs are processed via `while let Some(pending) = meshes.last()`. If `solid.tessellate` fails (e.g. `shown.mesh_failed() == true`) or `sketch.find_regions()` returns `None`, `return;` is executed **without** `meshes.pop()`.
  The failing item remains at the top of `meshes`. In every subsequent worker iteration, `mesh_pending` encounters this same item, fails immediately, and returns. All subsequent meshes (cut previews, requested body meshes, sketch triangulations) are permanently starved.
- **Impact**: Background rendering and viewport mesh updates freeze across the entire session following a single degenerate feature.
- **Recommendation**:
  Only return without popping when `cancel.is_cancelled()`. If uncancelled, pop the failing item before returning.

---

### Finding 2.3: `Recomputer::poll` Overwrites Terminal `Outcome::Finished` with Intermediate `FeaturesDone`
- **Type**: Concurrency Bug / State Synchronization
- **Severity**: **Medium-High**
- **Location**: [`crates/caditor-document/src/worker.rs:401-420`](file:///home/tecnio/workspace/caditor/crates/caditor-document/src/worker.rs#L401-L420)
- **Description**:
  In `Recomputer::poll`, the channel drain loop retains only the latest non-draft update. If revision R emits `Outcome::Finished` and the worker immediately starts revision R+1 emitting `Outcome::FeaturesDone`, `latest` overwrites `Finished` with `FeaturesDone`.
  `Model::poll_recomputer` never receives `Finished`, keeping `model.status` in `RecomputeStatus::Recomputing` and preventing `self.newest` from clearing, which locks out subsequent `waiting_draft` submissions.
- **Impact**: UI status indicator stuck in "Recomputing…"; draft recompute requests rejected.
- **Recommendation**:
  Do not overwrite terminal outcomes (`Finished`, `Failed`, `Cancelled`) with non-terminal progress reports (`FeaturesDone`).

---

### Finding 2.4: `reshape_sketch` Leaves Orphaned Dimension Parameters on Deleted Constraints
- **Type**: Functional Bug / Model Invariant
- **Severity**: **Medium**
- **Location**: [`crates/caditor-document/src/edit/sketch.rs:191-215, 481-487`](file:///home/tecnio/workspace/caditor/crates/caditor-document/src/edit/sketch.rs#L191-L215)
- **Description**:
  When entities and constraints are trimmed or deleted in `reshape_sketch`, constraints absent from `after` are removed via `Edit::RemoveSketchConstraint`. Unlike `remove_sketch_items`, `reshape_sketch` never invokes `release_dimensions`.
- **Impact**: Unused parameters linger in `document.parameters` with dangling `ParameterOwner::Dimension` references pointing to non-existent constraints.
- **Recommendation**:
  Collect constraints dropped in `after` and invoke `self.release_dimensions(feature, &truly_removed)`.

---

### Finding 2.5: `followed_value` Omits `Edit::RemoveParameter` and `Edit::SetFeaturePieces`
- **Type**: Functional Bug / Configurations
- **Severity**: **Medium**
- **Location**: [`crates/caditor-document/src/edit.rs:717-728`](file:///home/tecnio/workspace/caditor/crates/caditor-document/src/edit.rs#L717-L728)
- **Description**:
  `followed_value` lists edits that require updating active configuration table values. It omits `Edit::RemoveParameter` and `Edit::SetFeaturePieces`. Multi-step transactions (such as parameter inlining or body piece separation changes) leave active configuration cells stale.
- **Recommendation**:
  Add `Edit::RemoveParameter` and `Edit::SetFeaturePieces` to `followed_value`.

---

### Finding 2.6: Name Collision on Multiple Datum Projections in `DatumPlacing::insert`
- **Type**: Functional Bug / Edge Case
- **Severity**: **Low-Medium**
- **Location**: [`crates/caditor-document/src/model_scale.rs:951-961`](file:///home/tecnio/workspace/caditor/crates/caditor-document/src/model_scale.rs#L951-L961)
- **Description**:
  In `DatumPlacing::insert`, `unused_name` checks only `document.features()`, ignoring `self.names` accumulated during multi-feature scaling. Two projected datums sharing the same base name produce duplicate feature names, failing the scale transaction.
- **Recommendation**:
  Check `self.names` in addition to `document.features()`.

---

### Finding 2.7: `check_pieces_unused` Fails to Check Configuration References
- **Type**: Functional Bug / Data Integrity
- **Severity**: **Low-Medium**
- **Location**: [`crates/caditor-document/src/edit.rs:1240-1250`](file:///home/tecnio/workspace/caditor/crates/caditor-document/src/edit.rs#L1240-L1250)
- **Description**:
  When disabling separate pieces (`Pieces::BodyEach` $\to$ `Pieces::OneBody`), `check_pieces_unused` verifies other features do not reference piece bodies, but fails to check whether `self.configurations` configures piece appearances or colours.
- **Recommendation**:
  Ensure configuration table columns are checked before collapsing body pieces.

---

## 3. File Persistence, Recovery Journal & Import/Export (`caditor-file`, `caditor-step`)

### Finding 3.1: Windows Mandatory Locks Block Journal Discard, Replacement, and Deletion
- **Type**: Functional Bug / Platform Flaw (Windows)
- **Severity**: **Critical**
- **Location**: [`crates/caditor-file/src/recovery.rs:368-380, 394-410, 413-421`](file:///home/tecnio/workspace/caditor/crates/caditor-file/src/recovery.rs#L368-L380), [`crates/caditor-file/src/lock.rs:81-105`](file:///home/tecnio/workspace/caditor/crates/caditor-file/src/lock.rs#L81-L105)
- **Description**:
  On Windows, `lock_existing` opens a file and acquires an exclusive mandatory lock via `LockFileEx`.
  1. In `recovery.rs:discard()` and `remove_locked()`, `fs::remove_file(journal)` is executed while `file` is still held open with an active mandatory lock. In Windows NT, `DeleteFileW` fails with `ERROR_ACCESS_DENIED` (code 5) or `ERROR_SHARING_VIOLATION` (code 32).
  2. In `recovery.rs:set_aside_locked()`, `fs::rename(journal, &kept)` is invoked while `file` is held open, causing `MoveFileExW` to fail.
  3. In `lock.rs:install()`, `let existing = lock_existing(path)?;` holds an open handle. At line 104, `fs::rename(temporary, path)` attempts to overwrite `path` while `existing` is open, failing 100% of the time on Windows.
- **Impact**: On Windows, corrupted or unreadable journals cannot be discarded or replaced, causing perpetual startup recovery prompts and failed saves.
- **Recommendation**:
  Drop open `File` handles before invoking `fs::remove_file` or `fs::rename`, and use durable replace routines (`caditor_windows::move_file_durably`).

---

### Finding 3.2: Non-Atomic Placeholder Creation in `copy_then_place` Fails on Windows
- **Type**: Functional Bug / File System Race
- **Severity**: **High**
- **Location**: [`crates/caditor-file/src/save.rs:804-817`](file:///home/tecnio/workspace/caditor/crates/caditor-file/src/save.rs#L804-L817)
- **Description**:
  When hard linking is unavailable, `copy_then_place` creates a candidate backup file via `File::create_new(candidate)` and immediately drops it, followed by `fs::rename(temporary, candidate)`.
  On Windows NT, creating a new file notifies filter drivers, antivirus (Windows Defender), and search indexers, which immediately open read handles on the empty file. The subsequent `fs::rename` fails with sharing violations, leaving 0-byte corrupt backup files in the backup directory.
- **Impact**: Backup save failures and disk pollution with empty files.
- **Recommendation**:
  Rename temporary files directly into target candidate paths without pre-creating 0-byte placeholders.

---

### Finding 3.3: Missing `.is_finite()` Checks on Mesh Coordinates Causes Degenerate Welds
- **Type**: Functional Bug / Robustness
- **Severity**: **High**
- **Location**: [`crates/caditor-file/src/import/mesh.rs:260-269, 335-343, 389-408`](file:///home/tecnio/workspace/caditor/crates/caditor-file/src/import/mesh.rs#L260-L269)
- **Description**:
  Binary STL, OBJ, and 3MF parsers convert coordinates without checking `f.is_finite()`. Raw bit patterns for `NaN` and `Inf` in binary STL or string representations (`"NaN"`, `"inf"`) in OBJ/3MF are ingested into `mesh.positions`.
  When passed to `caditor-kernel/src/faceted/mod.rs:130-165` (`weld`), non-finite vertices are filtered out, tearing holes in meshes or causing `FacetedError::NoTriangles`.
- **Impact**: Silent geometry corruption or kernel panics/errors on importing malformed files.
- **Recommendation**:
  Validate `f.is_finite()` at all coordinate parsing boundaries, rejecting non-finite coordinates with descriptive errors.

---

### Finding 3.4: History Thinning Retains Orphaned Deltas When Baseline Frame Is Pruned
- **Type**: Data Integrity Bug
- **Severity**: **Medium**
- **Location**: [`crates/caditor-file/src/binary/model.rs:1356-1380`](file:///home/tecnio/workspace/caditor/crates/caditor-file/src/binary/model.rs#L1356-L1380)
- **Description**:
  In `write_versions()`, non-keyframe delta versions dependent on a pruned base must be recompressed. If decoding fails for a candidate, line 1379 executes `section.keep(version.data.whole)`. This copies the raw delta bytes into the file even though the reference frame was deleted, leaving an unresolvable delta chunk that fails on future decompression (`ZstdError::WrongPrefix`).
- **Impact**: Saved files contain corrupted historical versions that cannot be restored.
- **Recommendation**:
  Do not keep unresolvable deltas; prune them or mark them explicitly as broken.

---

### Finding 3.5: Extreme Coordinates Cause Spatial Hash Collapse to Vertex 0 in Kernel Weld
- **Type**: Numerical Bug
- **Severity**: **Medium**
- **Location**: [`crates/caditor-kernel/src/faceted/mod.rs:137-150`](file:///home/tecnio/workspace/caditor/crates/caditor-kernel/src/faceted/mod.rs#L137-L150)
- **Description**:
  If an imported mesh contains extreme coordinate values (e.g. $\pm 10^{200}$), `bounds.diagonal()` overflows `f64` to `INFINITY`. `reach` becomes `INFINITY`, all points map to cell `[0, 0, 0]`, and the entire mesh collapses into vertex 0, discarding all triangles.
- **Recommendation**:
  Validate `bounds.diagonal().is_finite()` and clamp bounds to valid CAD ranges.

---

### Finding 3.6: 3MF Model Size Bound Underestimates Saturated Decimal Strings
- **Type**: Export Failure Bug
- **Severity**: **Medium**
- **Location**: [`crates/caditor-file/src/export/three_mf.rs:130-159`](file:///home/tecnio/workspace/caditor/crates/caditor-file/src/export/three_mf.rs#L130-L159)
- **Description**:
  `model_size_bound` formats only `largest` to estimate coordinate string length. If `largest` is an integer (e.g. `100.0` $\to$ length 4), but internal vertices have fractional decimals (e.g. `99.123456` $\to$ length 9), the estimate undercounts actual XML size. In models near 4 GB, Zip64 extra headers are omitted, failing the export.
- **Recommendation**:
  Use maximum decimal representation length for string bound estimation.

---

### Finding 3.7: STEP Assembly Export Produces Colliding Product IDs on Duplicate Names
- **Type**: Export Compatibility Bug
- **Severity**: **Low**
- **Location**: [`crates/caditor-step/src/write/mod.rs:360-368, 469-477`](file:///home/tecnio/workspace/caditor/crates/caditor-step/src/write/mod.rs#L360-L368)
- **Description**:
  When exporting multiple bodies sharing identical names (e.g. default `"Body 1"`), duplicate `#... = PRODUCT('Body 1', 'Body 1', ...)` entities are emitted. Under ISO 10303-41, `PRODUCT.id` must be unique. Strict CAD importers (Siemens NX, PTC Creo) reject the file or collapse separate parts into one.
- **Recommendation**:
  Append unique occurrence counters to STEP product IDs.

---

### Finding 3.8: Unchecked Integer Additions in ZIP Central Directory Parsing
- **Type**: Robustness / Integer Overflow
- **Severity**: **Low**
- **Location**: [`crates/caditor-file/src/import/zip_read.rs:55-71, 85-92`](file:///home/tecnio/workspace/caditor/crates/caditor-file/src/import/zip_read.rs#L55-L71)
- **Description**:
  Offsets from untrusted ZIP headers are added via `at += CENTRAL_SIZE + name_length + extra + comment` without `checked_add`, risking integer overflow panics in debug mode.
- **Recommendation**:
  Use `checked_add` and bounds validation.

---

## 4. 2D Sketch Constraints, Solver & Expressions (`caditor-sketch`, `caditor-expression`)

### Finding 4.1: Moving Initial Guess in Interactive Drag Solver Flips Tangency Branches Permanently
- **Type**: UX Issue / Critical Numerical Stability
- **Severity**: **Critical**
- **Location**: [`crates/caditor/src/drag_solver.rs#L186-L208`](file:///home/tecnio/workspace/caditor/crates/caditor/src/drag_solver.rs#L186-L208)
- **Description**:
  During interactive entity dragging, `drag_solver.rs` solves intermediate frames using:
  ```rust
  let (from, memo) = match previous {
      Some(previous) => (&previous.sketch, Some(&previous.memo)),
      None => (&job.start, None),
  };
  from.solve_geometry_from(&job.parameters, &job.drags, memo)
  ```
  `from` is set to `&previous.sketch` (the intermediate frame). Inside `System::build`, branch decisions are re-evaluated against `from`:
  - `initial_side(from, ...)`: point left/right of line.
  - `initial_contact(from, ...)`: external vs internal circle-circle tangency.
  - `pair_parameter_start(from, ...)`: curve projection coordinates.
  If a user drags fast across a geometric boundary, an intermediate frame crosses the threshold, flipping the constraint branch. Once flipped, the solver locks onto the inverted branch—even if the user moves the cursor back to the starting point. When released, the inverted sketch commits to the document.
- **Impact**: Severe user frustration where dragging geometry suddenly inverts tangency or flips sides irreversibly.
- **Recommendation**:
  `System::build` must construct constraint equations and branch decisions using **`job.start`** (the baseline sketch before the drag). Intermediate frames (`previous.sketch`) must only be used as numeric initial values for Newton descent.

---

### Finding 4.2: `spans_at_variable` Omits Y Variables, Bypassing Zero-Length Collapse Detection
- **Type**: Functional Bug / Solver Invariant
- **Severity**: **High**
- **Location**: [`crates/caditor-sketch/src/solve/system.rs#L147-L154`](file:///home/tecnio/workspace/caditor/crates/caditor-sketch/src/solve/system.rs#L147-L154)
- **Description**:
  When registering spans in `System::build`:
  ```rust
  if let PointHandle::Variable(variable) = end {
      system.spans_at_variable.entry(variable).or_default().push(index);
  }
  ```
  `PointHandle::Variable(variable)` only indexes the $X$ coordinate variable. The $Y$ coordinate variable (`variable + 1`) is never registered.
  If a line is constrained purely along $Y$ (e.g. `Constraint::Horizontal`, constraining $y_1 = y_2$), the component contains only $Y$ variables. Looking them up in `spans_at_variable` yields nothing, so `component.spans` is empty. A zero-length line constrained by `Horizontal` solves without reporting `SketchError::NoLength`.
- **Impact**: Degenerate zero-length geometry bypasses validation and corrupts downstream profile generation.
- **Recommendation**:
  Register both `variable` ($X$) and `variable + 1` ($Y$) in `spans_at_variable`.

---

### Finding 4.3: Silent Base-Unit Coercion for Plain Numbers in Mixed Addition/Subtraction
- **Type**: Functional Bug / Expression Semantics
- **Severity**: **Medium**
- **Location**: [`crates/caditor-expression/src/expression.rs#L785-L797`](file:///home/tecnio/workspace/caditor/crates/caditor-expression/src/expression.rs#L785-L797), [`crates/caditor-expression/src/quantity.rs#L246-L260`](file:///home/tecnio/workspace/caditor/crates/caditor-expression/src/quantity.rs#L246-L260)
- **Description**:
  In `unify`, adding or subtracting a plain unitless number to a dimensioned quantity adopts the dimension. In Caditor, internal base units are `Millimetre` and `Degree`.
  - `10 mm + 5` evaluates to `15.0 mm`.
  - `1 m + 5` converts `1 m` to `1000.0 mm` and adds `5`, yielding `1005.0 mm = 1.005 m` instead of `6 m`.
- **Impact**: Silent calculation errors when users combine unitless numbers with non-millimetre SI units.
- **Recommendation**:
  Reject mixed addition/subtraction between plain numbers and dimensioned quantities with `EvalError::Mismatch`.

---

### Finding 4.4: Missing Circle Radius in Diagnosis Collapse Checks Wastes QuickXplain Budget
- **Type**: Performance Bug / Solver Diagnostics
- **Severity**: **Medium**
- **Location**: [`crates/caditor-sketch/src/solve/system.rs#L136-L143`](file:///home/tecnio/workspace/caditor/crates/caditor-sketch/src/solve/system.rs#L136-L143), [`crates/caditor-sketch/src/solve/diagnosis.rs#L46-L56`](file:///home/tecnio/workspace/caditor/crates/caditor-sketch/src/solve/diagnosis.rs#L46-L56)
- **Description**:
  `system.spans` only records spans for lines and arcs; circles are mapped to `None`. When a solve fails due to a collapsed zero-radius circle, `diagnose_failure` cannot find the collapsed entity and falls through to a full combinatorial conflict search, wasting solver budget and reporting confusing errors.
- **Recommendation**:
  Check `system.radius_variables` in `solver.collapsed`.

---

### Finding 4.5: Unchecked Division by Zero in `line_offset`
- **Type**: Numerical Bug
- **Severity**: **Medium**
- **Location**: [`crates/caditor-sketch/src/solve/system.rs#L988-L994`](file:///home/tecnio/workspace/caditor/crates/caditor-sketch/src/solve/system.rs#L988-L994)
- **Description**:
  In directional distance constraints, `direction.perp_dot(along)` can be zero if the line is parallel to the measurement axis, generating `NaN`/`Inf`.
- **Recommendation**:
  Guard with `denominator.abs() > 1e-12`.

---

### Finding 4.6: Auxiliary Parameter Variables Distort Reported System DOF
- **Type**: UX Issue / Calculation Inaccuracy
- **Severity**: **Medium**
- **Location**: [`crates/caditor-sketch/src/solve/mod.rs#L198-L203`](file:///home/tecnio/workspace/caditor/crates/caditor-sketch/src/solve/mod.rs#L198-L203)
- **Description**:
  DOF is computed as `system.values.len().saturating_sub(analysis.rank)`. Auxiliary constraint parameters (curve projection parameters $t, \theta$) are included in `system.values`. If an auxiliary constraint is redundant or rank-deficient, the UI displays spurious degrees of freedom.
- **Recommendation**:
  Filter auxiliary parameters out when computing user-facing DOF.

---

### Finding 4.7: Discontinuity in `ArcLength` Sweep at Zero Turn
- **Type**: Numerical Bug
- **Severity**: **Low**
- **Location**: [`crates/caditor-sketch/src/solve/equation.rs#L1588-L1590`](file:///home/tecnio/workspace/caditor/crates/caditor-sketch/src/solve/equation.rs#L1588-L1590)
- **Description**:
  `if turn > 0.0 { turn } else { TAU }` causes discontinuous jumps near zero, impeding Gauss-Newton convergence.
- **Recommendation**:
  Use epsilon comparison `turn > 1e-12`.

---

### Finding 4.8: Threshold Sensitivity in Sparse Givens Null-Space Elimination
- **Type**: Numerical Bug / UI Flickering
- **Severity**: **Low**
- **Location**: [`crates/caditor-sketch/src/solve/sparse.rs#L255-L315`](file:///home/tecnio/workspace/caditor/crates/caditor-sketch/src/solve/sparse.rs#L255-L315)
- **Description**:
  Fixed columns are detected using `NULL_SPACE_TOLERANCE.sqrt()` ($10^{-5}$). In near-singular linkages, columns oscillate across $10^{-5}$, causing entities to flicker between fully constrained and underconstrained.
- **Recommendation**:
  Introduce a hysteresis band for entity constraint state.

---

### Finding 4.9: Dependent Parameters in Cycles Report `UnknownParameter`
- **Type**: Error Diagnostics / UX
- **Severity**: **Low**
- **Location**: [`crates/caditor-document/src/values.rs#L180-L242`](file:///home/tecnio/workspace/caditor/crates/caditor-document/src/values.rs#L180-L242)
- **Description**:
  Parameters depending on a cyclic group are evaluated after cycle detection fails, reporting `UnknownParameter` instead of `Cycle`.
- **Recommendation**:
  Mark all transitively reachable parameters as `ParameterError::Cycle`.

---

## 5. Computational B-Rep Kernel, Tessellation & GPU Render (`caditor-kernel`, `caditor-render`)

### Finding 5.1: Missing Fallback to Marching on Shallow Angle Plane-Cylinder Intersections
- **Type**: Functional Bug / Critical Topology Integrity
- **Severity**: **Critical**
- **Location**: [`crates/caditor-kernel/src/intersect/surface_surface/analytic.rs:28-31, 141-167`](file:///home/tecnio/workspace/caditor/crates/caditor-kernel/src/intersect/surface_surface/analytic.rs#L28-L31), [`crates/caditor-kernel/src/intersect/surface_surface/mod.rs:120-125`](file:///home/tecnio/workspace/caditor/crates/caditor-kernel/src/intersect/surface_surface/mod.rs#L120-L125)
- **Description**:
  `analytic::intersect` unconditionally wraps `plane_cylinder` in `Some(...)`. When a plane cuts a cylinder at a shallow angle ($\theta \le 10^{-5}$ rad) or when $R / \cos\theta > \text{MAX\_SIZE}$, `Ellipse::new` fails and `plane_cylinder` returns an empty `Raw { curves: vec![], points: vec![] }`.
  Because `analytic::intersect` returns `Some(raw)` rather than `None`, `intersect_surfaces_through` treats the intersection as a successful empty intersection and **never falls back to `march::intersect`**.
- **Impact**: Intersection curves are silently discarded. Downstream boolean cut, union, or slice operations fail to partition faces, generating open shells or invalid non-manifold topology.
- **Recommendation**:
  Make `plane_cylinder` return `Option<Raw>`. Return `None` when `plane_cylinder_ellipse` fails so the marching solver takes over.

---

### Finding 5.2: Catastrophic Cancellation in Cone Ellipse Minor Radius Calculation
- **Type**: Numerical Bug
- **Severity**: **High**
- **Location**: [`crates/caditor-kernel/src/intersect/surface_surface/analytic.rs:243-251`](file:///home/tecnio/workspace/caditor/crates/caditor-kernel/src/intersect/surface_surface/analytic.rs#L243-L251)
- **Description**:
  When a plane cuts a cone, semi-minor axis calculation subtracts $(\text{offset} \cdot \text{axis})^2$ and $\cos^2\alpha \|\text{offset}\|^2$. When the cutting plane is distant from the apex ($L \approx 10^3\text{--}10^5$), subtracting two large $O(L^2)$ numbers eliminates 10 to 14 decimal digits. Roundoff errors frequently cause `minor_squared <= 0.0`, spuriously failing analytic intersection.
- **Impact**: Plane-cone cuts fail spuriously when the cutting plane is not near the apex.
- **Recommendation**:
  Use the cancellation-free Dandelin sphere formula:
  $$b = \sin\alpha \sqrt{\|near - apex\| \cdot \|far - apex\|}$$

---

### Finding 5.3: Coordinate Precision Collapse in Near-Parallel Plane-Plane Intersections
- **Type**: Numerical Bug
- **Severity**: **High**
- **Location**: [`crates/caditor-kernel/src/intersect/surface_surface/analytic.rs:97-109`](file:///home/tecnio/workspace/caditor/crates/caditor-kernel/src/intersect/surface_surface/analytic.rs#L97-L109)
- **Description**:
  In `plane_plane`, the line point closest to the world origin is computed. When planes meet at $\theta \approx 10^{-9}$ rad, the line origin is placed at $\|point\| \approx 10^{12}\text{ to } 10^{15}\text{ mm}$.
  Trimming this line to a local window $[-50, 50]$ evaluates $point + direction \cdot t$, subtracting numbers of magnitude $10^{15}$. At $10^{15}$, 1 ULP is $\approx 0.22\text{ mm}$, introducing millimeter-to-meter errors in the intersection curve.
- **Impact**: Extreme vertex/edge deviation in booleans with near-coplanar faces, breaking topological vertex snapping.
- **Recommendation**:
  Anchor the line point to the query window center `window.center()` rather than the world origin.

---

### Finding 5.4: Singular Vertex Normal Cache Corruption in Face Triangulation
- **Type**: Visual / Rendering Bug
- **Severity**: **Medium**
- **Location**: [`crates/caditor-kernel/src/tessellation/face.rs:718-742`](file:///home/tecnio/workspace/caditor/crates/caditor-kernel/src/tessellation/face.rs#L718-L742)
- **Description**:
  At singular points (cone apex, sphere poles), `surface.normal()` is undefined. `vertex_normal` nudges `uv` toward the centroid of the *first triangle* and caches the result in `corners[index]`. All subsequent triangles around the pole reuse this first triangle's normal.
- **Impact**: Cones and spheres exhibit asymmetric, dark, or inverted specular highlights around singularities.
- **Recommendation**:
  Do not cache corners in `corners[index]` if `surface.normal()` is singular.

---

### Finding 5.5: Ray-Plane Intersection Boundary Chatter
- **Type**: Numerical Bug
- **Severity**: **Medium**
- **Location**: [`crates/caditor-geometry/src/ray.rs:40-47`](file:///home/tecnio/workspace/caditor/crates/caditor-geometry/src/ray.rs#L40-L47)
- **Description**:
  `Ray::intersect_plane` checks `distance >= 0.0`. When a ray origin lies on the plane, rounding produces $\pm 10^{-16}$, causing intermittent hit/miss oscillation.
- **Recommendation**:
  Allow a tolerance: `(distance >= -1e-12).then_some(distance.max(0.0))`.

---

### Finding 5.6: GPU Picking Readback Buffer Offset Alignment Assumption
- **Type**: Robustness / GPU Pipeline
- **Severity**: **Low**
- **Location**: [`crates/caditor-render/src/picking.rs:57-64`](file:///home/tecnio/workspace/caditor/crates/caditor-render/src/picking.rs#L57-L64)
- **Description**:
  `window.plane_bytes() * 2` allocates the readback buffer assuming ID and depth textures can be placed consecutively. The offset is not explicitly aligned to `wgpu::COPY_BUFFER_ALIGNMENT` (4 bytes).
- **Recommendation**:
  Enforce `.next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT)`.

---

## 6. Remediation Roadmap & Action Plan

| Priority | Task | Affected Files | Effort |
|:---|:---|:---|:---:|
| **P0** | Include Home overlay and 6 modal dialogs in `Files::is_blocking()` & `modal_open` | `caditor/src/app.rs`, `caditor/src/files.rs` | Low |
| **P0** | Lock branch orientation to `job.start` in `drag_solver.rs` to fix tangency flips | `caditor/src/drag_solver.rs` | Medium |
| **P0** | Drop locked file handles before deleting/renaming in recovery & lock install (Windows) | `caditor-file/src/recovery.rs`, `lock.rs` | Medium |
| **P0** | Return `Option<Raw>` in `plane_cylinder` to enable marching fallback on shallow cuts | `caditor-kernel/src/intersect/.../analytic.rs` | Low |
| **P1** | Consume Enter and Escape in `typed_point.rs` and `field.rs` | `caditor/src/typed_point.rs`, `field.rs` | Low |
| **P1** | Fix `widgets::footer_split` layout using `Sides::new().show(ui, start, end)` | `caditor/src/widgets.rs` | Low |
| **P1** | Replace bare `RemoveFeature` with `Document::deletion` in `remove_empty` | `caditor/src/editing.rs` | Low |
| **P1** | Pop failed items from `meshes` in worker to prevent background mesh starvation | `caditor-document/src/worker.rs` | Low |
| **P1** | Add both X and Y coordinates to `spans_at_variable` in sketch system | `caditor-sketch/src/solve/system.rs` | Low |
| **P1** | Add `.is_finite()` validation to STL, OBJ, and 3MF coordinate importers | `caditor-file/src/import/mesh.rs` | Low |
| **P1** | Fix Dandelin sphere formula and window-centered plane-plane intersection | `caditor-kernel/src/intersect/.../analytic.rs` | Medium |
| **P1** | Eliminate 0-byte placeholder race in `copy_then_place` | `caditor-file/src/save.rs` | Low |
| **P2** | Release dimension parameters in `reshape_sketch` on removed constraints | `caditor-document/src/edit/sketch.rs` | Low |
| **P2** | Do not overwrite terminal `Outcome::Finished` in `Recomputer::poll` | `caditor-document/src/worker.rs` | Low |
| **P2** | Disallow implicit plain number base-unit coercion in `unify` | `caditor-expression/src/expression.rs` | Low |
| **P2** | Retain missing recent models instead of silently purging on failed open | `caditor/src/files.rs` | Low |
| **P2** | Fix singular vertex normal caching in face triangulation | `caditor-kernel/src/tessellation/face.rs` | Medium |
