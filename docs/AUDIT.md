# Audit

2026-10-10. Bugs, correctness, and UX only. High confidence only.

A deep read of the kernel, the document and recompute, the sketch solver, file persistence and STEP, the expression and geometry crates, the renderer and picking, and the modelling and file interface. Each candidate below was opened again in the current tree and kept only when the cited lines still do what the finding says, the behaviour is not the written design, and `docs/BUGS.md` does not already list it. Findings already on the roadmap were left there.

## What this does not cover

Security, performance, tests, tech debt, dependencies, developer experience, documentation quality, and new capability. Open product decisions in `docs/DECISIONS.md` (assemblies, surfaces, sheet metal, trim at the reference axes, a failed measurement keeping its last reading) are not bugs. `caditor-windows` and `caditor-wayland` had no pass of their own. The kernel, the expression and geometry crates, and the file format, journal, and STEP reader were read and produced no new high-confidence defect.

## Findings

Ordered by how much a user feels the failure against how small the fix is.

### [BUG-01] Leave a recovery offer in place when discard fails

- **Evidence**: `crates/caditor/src/files.rs:1589` — `Event::Discarded` removes the journal from `recoverable` before it looks at `error`. `crates/caditor/src/files.rs:967` — `has_recoverable` is that list being non-empty, and Recover unsaved work is offered only while it is (`crates/caditor/src/files.rs:3068`).
- **Impact**: Discard can fail, the journal stays on disk, and the notice says so, but the card is gone and Recover unsaved work stays unavailable until the next launch. An untitled journal has no other way back in this session.
- **Effort**: S
- **Risk**: LOW. Remove the row only when `error` is absent.
- **Confidence**: HIGH
- **Fix sketch**: On `Some(error)`, leave the candidate in `recoverable` and say the changes are still there to restore.

### [BUG-02] Scale every configuration's length expressions with the model

- **Evidence**: `crates/caditor-document/src/model_scale.rs:147` — `Document::scaled` rewrites parameters, feature lengths, and saved views, and never reads `configurations`. `crates/caditor-document/src/edit.rs:763` — applying a parameter edit copies the new live expression into the active row only. `crates/caditor-document/src/configurations.rs:332` — switching writes each other row's stored expression back.
- **Impact**: After Scale model, another configuration still holds the sizes from before the scale. Switching to it rebuilds the part at the old size.
- **Effort**: M
- **Risk**: MED. Inactive rows need the same length rewrite; the active row must keep the live value so it is not scaled twice.
- **Confidence**: HIGH
- **Fix sketch**: In the scale transaction, rewrite every inactive row's length cells with the same rescaler and store them in one `SetConfigurations`. Leave angles, counts, and suppression cells alone.

### [BUG-03] Treat configuration cells as uses of a parameter

- **Evidence**: `crates/caditor-document/src/document.rs:1515` — `used_parameters` collects parameter expressions and `feature.kind.parameters()`, and does not read configuration cells. `crates/caditor-document/src/edit.rs:1116` — `remove_parameter` treats a parameter as used only when another parameter or a feature refers to it. `crates/caditor-document/src/inlining.rs:28` — `inline_parameter` rewrites live parameters, sketch dimensions, feature expressions, and densities, then removes the parameter. `crates/caditor-document/src/configurations.rs:346` — switching writes an inactive cell back with `SetParameterExpression`, and `crates/caditor-document/src/edit.rs:1186` rejects a missing id.
- **Impact**: A parameter that appears only in an inactive configuration, or one that was inlined out of the live model while a cell still names it, can be deleted. Activating that configuration then fails with a missing parameter, and the whole switch is rolled back.
- **Effort**: M
- **Risk**: MED. Substitute only in stored cells. The active row already follows the live expression. A cell whose column is the parameter being removed has to be dropped, or the switch would still name it.
- **Confidence**: HIGH
- **Fix sketch**: Count `Configurations::parameters_named` in `used_parameters` and in `remove_parameter`. Run `Expression::inlining` over inactive cells in the inline transaction.

### [BUG-04] Discard a zero-alpha face before writing a section cap in the pick pass

- **Evidence**: `crates/caditor-render/src/viewport.wgsl:911` — `fs_mesh_sectioned` discards a face whose alpha is already zero, before it draws a cap. `crates/caditor-render/src/viewport.wgsl:1066` — `fs_mesh_pick_sectioned` caps every back face and writes pick id 0. `crates/caditor/src/scene.rs:1238` — a see-through face stays on the opaque instance at alpha 0, with its pick id, and is also drawn in the translucent pass. `crates/caditor-render/src/viewport.wgsl:580` — the mesh vertex shader passes that alpha through to both fragment shaders.
- **Impact**: On a sectioned body that has a see-through face, the colour pass leaves a hole in the cap where that back face is, and the face shows through it. The pick pass writes an invisible cap of id 0 in front of it, so a click there selects nothing.
- **Effort**: S
- **Risk**: LOW. Opaque faces still cap, because their alpha stays above zero.
- **Confidence**: HIGH
- **Fix sketch**: Discard `in.color.a <= 0.0` in `fs_mesh_pick_sectioned` before the cap, as the colour shader does. The translucent pick pass can then record that face's id.

### [BUG-05] Pattern the body the selection names

- **Evidence**: `crates/caditor/src/sketch_pattern_tools.rs:143` — Curve pattern and Point pattern take the last standing body whenever no tree row is chosen and `bodies_in` is empty. `crates/caditor/src/selection.rs:249` — `Pickable::body` is a face, edge, or vertex only, so a datum, a centre of mass, or a sketch item counts as no body. `crates/caditor/src/pattern_tools.rs:181` — Linear pattern uses the last body only when the selection is empty.
- **Impact**: With a datum axis, a principal plane, or the centre of mass of an earlier body selected, Curve pattern or Point pattern patterns the last standing body, along a guessed sketch when the selection holds no sketch.
- **Effort**: S
- **Risk**: MED. A selection that holds only sketch curves, points, and regions must still fall through to the last body, which is the rule for those two patterns.
- **Confidence**: HIGH
- **Fix sketch**: Use the last standing body only when the selection is empty or holds only sketch geometry. Any other non-body selection returns the same refusal Linear pattern uses. Cover a datum axis and a centre of mass.

### [BUG-06] Refuse a datum when the selection names nothing it can use

- **Evidence**: `crates/caditor/src/datum_tools.rs:420` — `why_unusable` returns nothing for a sketch region, a sketch constraint, a centre of mass, or a body item picked while sketching. `crates/caditor/src/datum_tools.rs:533` — with no plane gathered, a new datum plane starts on the XY plane. `crates/caditor/src/datum_tools.rs:650` — with no point gathered, a new datum point starts at the origin. Datum axis already refuses that selection (`crates/caditor/src/datum_tools.rs:586`).
- **Impact**: Plane or Point, with a sketch region, a constraint, or a centre of mass selected, creates a datum on XY or at the origin and opens it. The selection is not what the datum is built on.
- **Effort**: S
- **Risk**: LOW. An empty selection still uses XY and the origin.
- **Confidence**: HIGH
- **Fix sketch**: When the selection is non-empty and nothing usable was gathered, return a refusal in the same words datum axis already uses. Test a centre of mass, a sketch region, and an empty selection.

### [BUG-07] Skip the back faces a section cap already covers

- **Evidence**: `crates/caditor-render/src/through.rs:161` — `hits_through` lists every triangle on the kept side of a plane, with no back-face or cap test. `crates/caditor/src/paint_selection.rs:71` — painting keeps the first of those faces. The sectioned pick shader writes id 0 on the cap (`crates/caditor-render/src/viewport.wgsl:1071`), so a click on that same pixel selects nothing.
- **Impact**: Painting or listing under the pointer on a section cap selects the hidden far wall. A click on that cap selects nothing.
- **Effort**: M
- **Risk**: MED. A closed mesh's back face behind the cap has to be dropped without dropping a front-facing interior face. An open sheet must still report its far side.
- **Confidence**: HIGH
- **Fix sketch**: For a closed mesh, treat a back-facing hit behind the ray's section entry as the cap: omit that hit and later sectioned hits along the ray. Leave open meshes and front-facing hits unchanged.

### [BUG-08] Keep a disabled constraint disabled when a fillet merges its point

- **Evidence**: `crates/caditor-sketch/src/fillet.rs:1073` — `merge_point` removes each constraint on the discarded corner point and inserts it on the kept point. `crates/caditor-sketch/src/sketch.rs:1240` — `remove_constraint` clears the inactive flag and the label offset, and the insert does not put either back. Fillet and chamfer call `merge_point` for a corner held by two coincident points (`crates/caditor-sketch/src/fillet.rs:1020`).
- **Impact**: Filleting or chamfering two curves that meet through a coincident pair, rather than one shared point, turns a disabled dimension on the discarded point back on. The solver then enforces a value the user had switched off, and the label jumps back to its automatic place.
- **Effort**: S
- **Risk**: LOW. A duplicate or rejected constraint stays removed.
- **Confidence**: HIGH
- **Fix sketch**: Record `is_active` and `label_offset` before `remove_constraint`, and restore both after a successful insert. Test a disabled distance on the second corner point.

### [BUG-09] Put a kept dimension's label offset back after a reshape

- **Evidence**: `crates/caditor-sketch/src/trim.rs:1138` — `restructure` remembers the inactive flag of each kept constraint and restores it after insert, and does not remember the label offset. `remove_constraint` drops that offset (`crates/caditor-sketch/src/sketch.rs:1246`). `crates/caditor-sketch/src/trim.rs:1228` — `move_constraints` copies the inactive flag onto the far piece and not the offset.
- **Impact**: Trim, extend, split, break, fillet, and chamfer drop the placed position of every dimension they keep. The label jumps back to the automatic place. The geometry is unchanged.
- **Effort**: S
- **Risk**: LOW. The offset is already valid for the kept dimension.
- **Confidence**: HIGH
- **Fix sketch**: Remember `label_offset` beside the inactive flag in `restructure` and `move_constraints`, and write it back after insert. Cover a placed distance that survives a line shorten and a split.

### [BUG-10] Count a body's density as a use of its parameter

- **Evidence**: `crates/caditor-document/src/document.rs:1515` — `used_parameters` skips `BodyAppearance::parameters`. `crates/caditor/src/parameter_table.rs:965` — the Parameters panel caches that set, draws the unused icon from it (`crates/caditor/src/parameter_table.rs:461`), and builds Delete unused parameters from it (`crates/caditor/src/parameter_table.rs:168`). `crates/caditor-document/src/edit.rs:1116` — removal still refuses, because `feature.uses_parameter` includes the density, and `body_appearance_tests.rs` pins that. `crates/caditor/src/parameter_table.rs:209` — the bulk delete posts a success notice after the apply, and `crates/caditor/src/model.rs:950` lets that notice replace the error from the refused edit.
- **Impact**: A parameter used only as a body's density is shown as unused, and Delete unused parameters says it was deleted. The parameter is still there, the mass is unchanged, and any genuinely unused parameters in the same transaction stay too, because the apply is atomic.
- **Effort**: S
- **Risk**: LOW. Include `feature.parameters()`, which already adds the density. Post the success notice only after the apply succeeds.
- **Confidence**: HIGH
- **Fix sketch**: Collect `Feature::parameters` in `used_parameters`. In `UnusedDeletion::perform`, inform only when the apply is accepted.

### [BUG-11] Report a file dialog that dies as a dialog error

- **Evidence**: `crates/caditor/src/portal/xdg.rs:143` — a portal response stream that ends is returned as a cancel. `crates/caditor/src/portal/xdg.rs:273` — a zenity process that exits with no status is returned as a cancel, beside the real cancel status 1. `crates/caditor/src/files.rs:2306` — a cancel clears `after_save`, which is how a save continues into quit, new, or open.
- **Impact**: If the portal drops the request, or zenity is killed, the waiting bar clears as though the user cancelled. A save that was meant to continue into quit or open is dropped, with no explanation.
- **Effort**: S
- **Risk**: MED. Status 1 must stay a user cancel. Only a missing response, or a zenity exit with no status, becomes `DialogError`.
- **Confidence**: HIGH
- **Fix sketch**: Return `DialogError` when `responses.next()` yields nothing and when zenity's status code is `None`. Keep status 1 as `Ok(None)`.

### [UX-01] Warn when a parameter import leaves rows out

- **Evidence**: `crates/caditor/src/files/parameters.rs:197` — Apply always posts `Notice::success`, including when `summary` reports refused rows (`crates/caditor/src/files/parameters.rs:239`).
- **Impact**: An import that adds some parameters and skips others shows the success icon. The skipped rows are ordinary text beside that icon, so a partial import reads as fully done.
- **Effort**: S
- **Risk**: LOW. Only the notice kind changes when a row was refused.
- **Confidence**: HIGH
- **Fix sketch**: Use `Notice::warning` when any row is refused. Keep `Notice::success` when every applied row was added, changed, or kept.

### [UX-02] Tell the user when the recent-file list cannot be saved

- **Evidence**: `crates/caditor/src/files.rs:2537` — Forget recent updates the in-memory list and posts success immediately. `crates/caditor/src/files.rs:2633` — the write runs on the files worker, and a failed `RecentFiles::save_changes` is only logged.
- **Impact**: Forget recent and the same path for other recent-list edits look finished. If the write fails, the next launch lists those models again. The model file is left untouched.
- **Effort**: S
- **Risk**: LOW. Report the error on the existing files-worker event path, as a failed preferences save already does.
- **Confidence**: HIGH
- **Fix sketch**: Send a failure event from the recent-file job and replace the success notice with one that names the write error and says the list will be written again on the next change.

## Considered and rejected

- An angle between a line and an arc is dropped whenever trim, extend, fillet, or split reshapes that arc (`keeps_sweep` in `crates/caditor-sketch/src/trim.rs:1544`). The solver could still hold an angle whose joint end did not move, but `sketch.md` states that those operations drop such an angle. That is the written design.
- Everything already listed in `docs/BUGS.md`, including the files and recovery items, the kernel boolean and shell items, the modelling reference and measurement items, the application session and drag items, the offset refusal, the solver diagnosis items, and the STEP healing items. Performance, missing checks, missing features, and platform work stay in their own roadmap files.
- The kernel pass (booleans, blends, shells, enclosure, patterns, naming, sweeps, intersection, classification, mapping, tessellation, mass properties, faceted solids) found no new high-confidence defect.
- The expression and geometry pass found no new high-confidence defect. The stored-versus-typed unit power, and imperial units remaining readable in stored text, are intentional.
- The file-format, journal, clipboard, DXF, SVG, and STEP pass found no new high-confidence defect beyond the roadmap.

## Method

Eight read-only passes, one each for the kernel, the document, the sketch solver, file persistence, the application shell, modelling and sketch editing, the renderer, and expressions. Line numbers from those passes were leads. The lines cited above were read again before they were written here. A finding that could not be confirmed, or that restated a rule or a roadmap entry, was dropped.
