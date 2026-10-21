# Caditor Editor & CAD Experience Audit Report

**Date**: October 2026  
**Target Version**: Caditor 0.1.0-alpha  
**Scope**: Viewport, 2D Sketching, Constraint Solver, 3D Parametric Modeling, Direct Manipulation Gizmos, Feature Tree, Selection & Picking, UI Shell, Navigation, Accessibility & State Management.  
**Auditor**: Antigravity Pair-Programming Audit Engine

---

## Executive Summary

A comprehensive architectural and experiential audit was conducted across the Caditor CAD editor codebase (`crates/caditor`, `crates/caditor-sketch`, `crates/caditor-document`, `crates/caditor-render`, and `crates/caditor-geometry`).

Caditor exhibits an exceptionally thoughtful design philosophy centered on parametric stability, topological naming permanence, SI unit purity, SI-first expressions, and avoiding FreeCAD-style silent model breakage. However, this deep audit revealed **57 distinct issues** (9 Critical, 18 High, 22 Medium, 8 Low) that directly undermine user trust, cause mathematical divergence during viewport dragging, trap keyboard focus, silently corrupt user design intent, or render geometry invisible due to severe visual contrast failures.

### Primary Risk Themes
1. **Silent Design Intent Corruption**: Extrusion auto-detection silently converts finite-depth cuts into infinite `ThroughAll` piercing cuts without user notification, falls back to destructive subtraction when reach collision checks timeout or encounter symmetric extents, and auto-merges touching bodies without confirmation.
2. **Numerical Divergence in Direct Manipulation**: Skew-ray linear projection in reach handles exhibits hyperbolic jump divergence when viewed near-parallel to the drag axis, and revolution handles suffer dead-zone lockup when viewed edge-on.
3. **Universal Keyboard & Modal Traps**: Numeric text fields fail to surrender focus or revert on `Escape`, causing the global viewport command dispatcher to ignore tool cancellation and trapping users in active tools. Furthermore, solid tools become permanently uncancellable if the document history diverges while the panel is open.
4. **Solver & Dimension Trial Bypassing**: Smart dimensions, drag-release snap joins, and constraint activation completely bypass `ConstraintTrial` off-thread validation, committing conflicting constraints directly into the document and breaking the sketch state.
5. **Severe WCAG Accessibility Violations**: Several default 3D canvas presets (Studio, Grey, Blue-Grey) draw inactive bodies and sketch geometry with contrast ratios as low as 1.01:1 to 1.36:1 against the canvas, making geometry completely invisible to users.

---

## 1. 2D Sketching, Drawing Tools, & Chain Workflows

### Finding 1.1: Tool Switching Mid-Draw Silently Drops Chain Memory & Step-Back History
- **Location**: [`drawing.rs:1036-1068`](file:///home/tecnio/workspace/caditor/crates/caditor/src/drawing.rs#L1036-L1068), [`drawing.rs:1091-1126`](file:///home/tecnio/workspace/caditor/crates/caditor/src/drawing.rs#L1091-L1126), [`drawing.rs:494-506`](file:///home/tecnio/workspace/caditor/crates/caditor/src/drawing.rs#L494-L506)
- **Severity**: **High**
- **Description**: While drawing multi-segment continuous profiles (e.g. connected lines or tangent arcs), individual completed segments are applied to the document model as separate transactions. However, continuous chain context (`self.chain`, `self.chain_start`) is stored exclusively in transient memory inside `Drawing`. When the user switches tools (e.g., from Line to Circle, Rectangle, or Select), `continued_in` returns `None`. Consequently, `Drawing::sync` executes `*self = Self::default()`, permanently wiping `self.chain`.
- **UX Impact**: If a user temporarily switches to another tool or inspects geometry and returns to the Line tool, `self.chain` is gone. Pressing `Backspace` can no longer step back through previously drawn segments in that chain. In addition, switching between tools with incompatible start points (such as `Rectangle` to `Circle`) silently drops any uncommitted start point without feedback.
- **Recommendation**: Retain chain history across tool switches within the same sketch session, or prompt the user to finish the chain before switching tools.

---

### Finding 1.2: `remove_last` Step-Back Desynchronization Leaves Ghost Geometry
- **Location**: [`drawing.rs:1819-1842`](file:///home/tecnio/workspace/caditor/crates/caditor/src/drawing.rs#L1819-L1842), [`viewport.rs:5042-5046`](file:///home/tecnio/workspace/caditor/crates/caditor/src/viewport.rs#L5042-L5046)
- **Severity**: **High**
- **Description**: In `Drawing::remove_last`, step-back verifies:
  ```rust
  let undoes_segment = self.placed.len() == 1
      && self.chain.last().is_some_and(|step| undo_label == Some(step.label.as_str()));
  ```
  If `undo_label` in `Model` differs from `step.label` (which occurs when symmetric drawing appends `"symmetrically"` or when another model event modifies the top undo label), `undoes_segment` evaluates to `false`. When `false`, `remove_last` pops `self.placed`, clears `self.chain`, and returns `false` without dispatching `Action::Undo`.
- **UX Impact**: The last placed line remains in the document, while the active drawing tool resets to an unplaced state. The user can no longer step back through the chain with Backspace, leaving orphaned lines.
- **Recommendation**: Decouple step-back from rigid string equality against `model.undo_label()`, or ensure mirrored and modified transactions store matched label tokens.

---

### Finding 1.3: Key Collision Between Point Step-Back and Selection Deletion
- **Location**: [`viewport.rs:4978-4984`](file:///home/tecnio/workspace/caditor/crates/caditor/src/viewport.rs#L4978-L4984), [`viewport.rs:5035-5040`](file:///home/tecnio/workspace/caditor/crates/caditor/src/viewport.rs#L5035-L5040), [`commands.rs:2253`](file:///home/tecnio/workspace/caditor/crates/caditor/src/commands.rs#L2253)
- **Severity**: **Medium**
- **Description**: Both `Key::Backspace` and `Key::Delete` map to `back = true`. When `self.drawing.in_progress()` is true, `back` steps back the in-flight point. However, when a creation tool is active but no points have been clicked yet (`!self.drawing.in_progress()`), `handle_keys` does not consume `back`. The keystroke falls through to `Command::DeleteSelection`.
- **UX Impact**: If a user had background entities selected prior to choosing a tool and presses Backspace/Delete expecting to cancel the tool, the selected background geometry is deleted without confirmation.
- **Recommendation**: Restrict point step-back exclusively to `Key::Backspace`. Clear selection automatically upon entering any creation tool.

---

### Finding 1.4: Escape Ladder Risk of Accidental Sketch Termination
- **Location**: [`viewport.rs:5285-5347`](file:///home/tecnio/workspace/caditor/crates/caditor/src/viewport.rs#L5285-L5347)
- **Severity**: **Low**
- **Description**: `ViewportState::escape` implements a 16-tier ladder:
  `heading release -> cancel drawing -> deselect tool -> clear selection -> finish sketch`.
- **UX Impact**: Rapidly pressing `Escape` to clear in-flight previews or clear selection often overshoots and triggers `Action::Editing(EditingCommand::Finish)`, unintentionally exiting sketch mode.
- **Recommendation**: Require a distinct shortcut or threshold confirmation before `Escape` closes the sketch session.

---

### Finding 1.5: Incomplete Bounding Box Calculation in Annotations and Inferences
- **Location**: [`annotations.rs:907-920`](file:///home/tecnio/workspace/caditor/crates/caditor/src/annotations.rs#L907-L920), [`caditor-sketch/src/inference.rs:123-143`](file:///home/tecnio/workspace/caditor/crates/caditor-sketch/src/inference.rs#L123-L143)
- **Severity**: **Low**
- **Description**: `extent_of` in `annotations.rs` only loops over `Entity::Point`, ignoring Lines, Circles, Arcs, Splines, and Ellipses. In sketches composed strictly of curves without lone points, `extent_of` returns `0.0`.
- **UX Impact**: Annotation text scaling and dynamic tolerance calculations degrade or collapse to `SMALLEST_DISTANCE = 1e-6`.
- **Recommendation**: Calculate bounds over all curve and entity geometries in `extent_of`.

---

## 2. 2D Constraint Solver, Snapping & Dimensioning Experience

### Finding 2.1: Smart Dimension Placement Heuristic Collapses on Steep / Shallow Lines
- **Location**: [`dimensioning.rs:256-270`](file:///home/tecnio/workspace/caditor/crates/caditor/src/dimensioning.rs#L256-L270), [`dimensioning.rs:179-189`](file:///home/tecnio/workspace/caditor/crates/caditor/src/dimensioning.rs#L179-L189)
- **Severity**: **Medium**
- **Description**: Dimension orientation (Horizontal vs. Vertical vs. Aligned Distance) relies on bounding box interval containment:
  ```rust
  let within_x = (low.x..=high.x).contains(&pointer.x);
  let within_y = (low.y..=high.y).contains(&pointer.y);
  match (within_x, within_y) {
      (true, false) => ConstraintTool::HorizontalDistance,
      (false, true) => ConstraintTool::VerticalDistance,
      _ => ConstraintTool::Distance,
  }
  ```
- **UX Impact**: On a near-vertical line, the horizontal range `[low.x..=high.x]` is a sub-millimeter band. Moving the pointer even slightly outside this narrow corridor flips `within_x` to false, snapping the dimension to Aligned Distance. Placing a horizontal dimension on steep geometry requires pixel-level precision.
- **Recommendation**: Replace axis-aligned box containment with radial sector/Voronoi quadrant heuristics based on the line's midpoint and cursor ray.

---

### Finding 2.2: Arc Dimensioning Heuristic Forbids Interior Radius & Has No Diameter Support
- **Location**: [`dimensioning.rs:239-249`](file:///home/tecnio/workspace/caditor/crates/caditor/src/dimensioning.rs#L239-L249)
- **Severity**: **Medium**
- **Description**: Single-arc Smart Dimension logic enforces:
  - Cursor within angular sweep and `reach.length() < arc.radius`: returns `ConstraintTool::Angle` (Sweep).
  - Cursor within angular sweep and `reach.length() > arc.radius`: returns `ConstraintTool::Distance` (Arc Length).
  - Cursor outside angular sweep: returns `ConstraintTool::Radius`.
- **UX Impact**: Users cannot place a Radius dimension on the concave interior side of an arc; doing so forces an Angle dimension. On arcs with sweeps >300°, placing a Radius dimension is nearly impossible because almost all cursor positions fall within the sweep sector. Furthermore, arcs cannot be dimensioned by Diameter.
- **Recommendation**: Allow placing Radius dimensions inside the arc sector, and support toggling between Radius and Diameter on arcs.

---

### Finding 2.3: Dimension Constraints Bypass `ConstraintTrial` Off-Thread Validation
- **Location**: [`viewport.rs:3415-3436`](file:///home/tecnio/workspace/caditor/crates/caditor/src/viewport.rs#L3415-L3436), [`sketch_toolbar.rs:609-622`](file:///home/tecnio/workspace/caditor/crates/caditor/src/sketch_toolbar.rs#L609-L622)
- **Severity**: **High**
- **Description**: While geometric constraints are routed through off-thread `Action::Trial` validation, dimension constraints bypass trial solving:
  ```rust
  if tool.is_dimension() {
      actions.push(Action::Apply(added.transaction));
  } else {
      actions.push(Action::Trial(...));
  }
  ```
- **UX Impact**: When a user adds an overconstraining or conflicting dimension, Caditor commits the transaction directly. The document recompute fails immediately, plunging the sketch into `SketchStatus::Conflicting`.
- **Recommendation**: Route dimension placement through `ConstraintTrial` or perform pre-commit solvability validation.

---

### Finding 2.4: Conservative Reference Dimension Inference in `determines`
- **Location**: [`sketch_tools.rs:1131-1143`](file:///home/tecnio/workspace/caditor/crates/caditor/src/sketch_tools.rs#L1131-L1143)
- **Severity**: **High**
- **Description**: `determines` only marks a new dimension as reference (inactive) if all involved entities have state `EntityState::FullyConstrained`. If an entity has even one unconstrained degree of freedom (e.g., free along Y but fixed along X), `determines` evaluates to `false`.
- **UX Impact**: Placing a Horizontal Distance between two points that are already constrained horizontally creates an active driving dimension instead of a reference dimension, causing an immediate solver conflict.
- **Recommendation**: Inspect the constraint graph or solver Jacobian to check if the specific 1D freedom being dimensioned is already determined.

---

### Finding 2.5: No Interactive Prompt When Overconstraining Dimensions Are Placed
- **Location**: [`dimensioning.rs:455-480`](file:///home/tecnio/workspace/caditor/crates/caditor/src/dimensioning.rs#L455-L480), [`sketch_tools.rs:1088-1128`](file:///home/tecnio/workspace/caditor/crates/caditor/src/sketch_tools.rs#L1088-L1128)
- **Severity**: **Medium**
- **Description**: Caditor either silently marks a dimension as an inactive reference or applies it as an active dimension that causes a solver conflict.
- **UX Impact**: Standard CAD workflows prompt: *"This dimension will over-constrain the sketch. Make this dimension driven? [Yes / Cancel]"*. Caditor offers no prompt or conversion dialog.
- **Recommendation**: Provide a confirmation prompt when an overconstraining dimension is placed, offering to convert it to a driven reference annotation.

---

### Finding 2.6: Unchecked Join Constraints on Drag Release Break Sketch
- **Location**: [`model.rs:1347-1353`](file:///home/tecnio/workspace/caditor/crates/caditor/src/model.rs#L1347-L1353)
- **Severity**: **High**
- **Description**: When an entity drag finishes with a snap join, `commit_drag` commits the join constraints directly without `ConstraintTrial` validation:
  ```rust
  for constraint in constraints {
      transaction.add_sketch_constraint(feature, constraint);
  }
  self.perform(Action::Apply(transaction));
  ```
- **UX Impact**: Dragging a point onto another entity to snap-join them can commit an invalid constraint that contradicts existing dimensions, causing the sketch solve to fail on release.
- **Recommendation**: Validate snap join constraints through `ConstraintTrial` before applying on drag release; if conflicting, commit only the moved positions without the join constraint.

---

### Finding 2.7: Entity Dragging Completely Ignores User Inference Settings
- **Location**: [`tracking.rs:604-661`](file:///home/tecnio/workspace/caditor/crates/caditor/src/tracking.rs#L604-L661), [`sketch_drag.rs:300, 355`](file:///home/tecnio/workspace/caditor/crates/caditor/src/sketch_drag.rs#L300)
- **Severity**: **Medium**
- **Description**: In `tracking::land`, `onto_body`, and `land_freely`, lookup configurations hardcode `inferred: Inferences::default()`. `sketch_drag.rs` does not receive or respect the user's active `Inferences` preferences.
- **UX Impact**: When dragging sketch entities, any inference types the user disabled in settings (e.g., Tangent, Midpoint, or Tracks) continue to snap aggressively.
- **Recommendation**: Pass active user inferences into `sketch_drag.rs` and forward them into `tracking::land`.

---

### Finding 2.8: Background 3D Body Vertices Preempt Native 2D Sketch Curve Snaps
- **Location**: [`snap.rs:559-565`](file:///home/tecnio/workspace/caditor/crates/caditor/src/snap.rs#L559-L565)
- **Severity**: **Medium**
- **Description**: Snap priority in `held` resolves as:
  `nearest(point_like) -> body_point -> on_curve (sketch curves) -> body_edge -> grid`.
- **UX Impact**: Vertices on underlying 3D solid bodies take precedence over 2D curves drawn in the active sketch. A user attempting to snap along an existing sketch line near a background body vertex gets pulled off the sketch curve onto the 3D body corner.
- **Recommendation**: Prioritize active sketch curves over external background body geometry, or allow toggling background body snapping off.

---

### Finding 2.9: Two-Point Intersection Snaps Lack Branch Tracking and Flip During Drags
- **Location**: [`snap.rs:800-808`](file:///home/tecnio/workspace/caditor/crates/caditor/src/snap.rs#L800-L808)
- **Severity**: **Medium**
- **Description**: When two curves intersect at multiple points (e.g. circle-line or circle-circle), `Target::Intersection(first, second)` records only entity IDs without branch tracking.
- **UX Impact**: Because both roots satisfy the constraints, the solver can jump between intersection roots during drags or recomputations, flipping points across circles.
- **Recommendation**: Include branch disambiguation (e.g., sign of dot product or sector index) in intersection constraints.

---

### Finding 2.10: Probing Finite Difference Cancellation & Curvature Sign Loss in Drag Reach
- **Location**: [`caditor-sketch/src/solve/numeric/reach.rs:191-220`](file:///home/tecnio/workspace/caditor/crates/caditor-sketch/src/solve/numeric/reach.rs#L191-L220)
- **Severity**: **High**
- **Description**: In `reach.rs`, finite difference probing uses `probe = 1e-7 * scale`, causing floating-point cancellation. Furthermore, line 220 takes `curvature.abs()`, discarding negative curvature signs and corrupting trust-region steps in saddle/concave regions.
- **UX Impact**: Dragging free geometry across non-convex constraint spaces causes sudden freezes, handle jitter, or solver divergence.
- **Recommendation**: Utilize analytical second derivatives or Levenberg-Marquardt damping for drag reach calculations.

---

### Finding 2.11: Toggling Inactive Constraints to Active Bypasses Trial Solving
- **Location**: [`sketch_toolbar.rs:633-640`](file:///home/tecnio/workspace/caditor/crates/caditor/src/sketch_toolbar.rs#L633-L640)
- **Severity**: **Medium**
- **Description**: Toggling an inactive constraint back to active via `Command::ToggleConstraintActive` directly applies `Action::Apply` without trial evaluation.
- **UX Impact**: Activating an inactive constraint that conflicts with current geometry throws the sketch into an error state immediately.
- **Recommendation**: Route `ToggleConstraintActive` through `Action::Trial`.

---

### Finding 2.12: Automated Tidying Batch Apply Bypasses `ConstraintTrial`
- **Location**: [`tidy_panel.rs:204-227`](file:///home/tecnio/workspace/caditor/crates/caditor/src/tidy_panel.rs#L204-L227), [`tidying.rs:405-465`](file:///home/tecnio/workspace/caditor/crates/caditor/src/tidying.rs#L405-L465)
- **Severity**: **Medium**
- **Description**: Clicking "Apply" in the automatic constraint tidying panel applies all proposed relations as a batch transaction directly via `Action::Apply`.
- **UX Impact**: If any proposed relation conflicts with existing dimensions due to tolerance thresholds, the sketch fails into `SketchStatus::Conflicting`.
- **Recommendation**: Validate batch relations through `ConstraintTrial` before committing.

---

## 3. 3D Parametric Modeling Tools & Panels

### Finding 3.1: Extrusion Detection Falls Back to Destructive Cut on Symmetric/Two-Sides/Timeout
- **Location**: [`extrusion_detection.rs:48-71, 84-112`](file:///home/tecnio/workspace/caditor/crates/caditor/src/extrusion_detection.rs#L48-L71), [`body_reach.rs:225-268`](file:///home/tecnio/workspace/caditor/crates/caditor/src/body_reach.rs#L225-L268)
- **Severity**: **Critical**
- **Description**: In `extrusion_detection.rs`, `directed(template, reversed)` only supports `ExtrudeExtent::OneSide`. Furthermore, `body_reach::Swept::of` returns `None` for any extent other than `OneSide { end: Distance(_), .. }`. When `sweep(true)` returns `None` (which also occurs if the 150ms `REACH_BUDGET` expires during complex collision checks), line 54 unconditionally returns:
  ```rust
  match sweep(true) {
      None => return cut(false),
  ```
- **UX Impact**: Caditor **defaults to a boolean Remove (Cut)** on the body the sketch is attached to—even when the user intends to add a symmetric boss, a two-sided extrusion, or when collision detection times out.
- **Recommendation**: Support `Symmetric` and `TwoSides` in `body_reach::Swept::of`. Never assume a destructive cut upon detection timeout; default safely to `BodyOperation::NewBody`.

---

### Finding 3.2: Silent Mutation of Finite Extrusion Depth to Infinite `ThroughAll`
- **Location**: [`extrusion_detection.rs:50-62, 138-144`](file:///home/tecnio/workspace/caditor/crates/caditor/src/extrusion_detection.rs#L50-L62)
- **Severity**: **Critical**
- **Description**: If `reaches.clears(&reached)` evaluates to `true` (bounding depth is less than extrusion length), line 50/141 mutates `template.extent` so that `*end = ExtrudeEnd::ThroughAll`, with `told: None`.
- **UX Impact**: A user specifies a finite distance (e.g. 20mm), but Caditor silently converts it to an infinite `ThroughAll` cut without any notification. If other bodies exist further along the ray, they are unintentionally pierced.
- **Recommendation**: Never silently mutate `ExtrudeEnd::Distance` to `ExtrudeEnd::ThroughAll`. Display an explicit banner or confirmation button if `ThroughAll` is suggested.

---

### Finding 3.3: Edgewise Non-Manifold Contact Joins Default to Invalid Add Operation
- **Location**: [`extrusion_detection.rs:80-119`](file:///home/tecnio/workspace/caditor/crates/caditor/src/extrusion_detection.rs#L80-L119)
- **Severity**: **High**
- **Description**: If an extrusion contacts a body only along an edge (a non-manifold junction), line 80/93 flags it as `Err(body)`. Yet line 113 still assigns `BodyOperation::Add(chosen)`.
- **UX Impact**: Downstream boolean kernel recomputes fail with `BooleanError::NonManifold`, confusing the user.
- **Recommendation**: Default to `BodyOperation::NewBody` when edgewise non-manifold contact is detected.

---

### Finding 3.4: Sketch Region Selection Click Overridden by Unconditional `RegionChoice::All`
- **Location**: [`solid_tools.rs:174-192`](file:///home/tecnio/workspace/caditor/crates/caditor/src/solid_tools.rs#L174-L192)
- **Severity**: **High**
- **Description**: In `solid_tools::sweep_source`, line 177 extracts `Pickable::SketchRegion` from selection, but lines 184–186 unconditionally assign:
  ```rust
  let (sketch, regions) = match (editing.feature(), selected.as_slice()) {
      (Some(edited), _) => (edited, RegionChoice::All),
      (None, [sketch]) => (*sketch, RegionChoice::All),
  ```
- **UX Impact**: If a user clicks a single closed region in a multi-region sketch and launches Extrude, Caditor ignores their pick and extrudes the entire sketch (`RegionChoice::All`).
- **Recommendation**: Collect all selected `Pickable::SketchRegion` items into `RegionChoice::Chosen(...)` if present.

---

### Finding 3.5: Destructive Switching from "All closed" to "Chosen" Regions
- **Location**: [`solid_panel.rs:434-440`](file:///home/tecnio/workspace/caditor/crates/caditor/src/solid_panel.rs#L434-L440)
- **Severity**: **Medium**
- **Description**: Toggling the region mode from "All closed" to "Chosen" populates the chosen list with *all* regions rather than starting empty or retaining the user's clicked region.
- **UX Impact**: Users are forced to click "Clear" and re-select their intended regions from scratch.
- **Recommendation**: Start with an empty selection or preserve user-selected regions when switching modes.

---

### Finding 3.6: Absence of 3D Viewport Picking for Target Body and "Also Cuts" in Solid Panel
- **Location**: [`solid_panel.rs:1422-1450, 1500-1530`](file:///home/tecnio/workspace/caditor/crates/caditor/src/solid_panel.rs#L1422-L1450)
- **Severity**: **High**
- **Description**: Target body and secondary cut bodies ("Also cuts") can only be selected from egui dropdown combo boxes. There is no 3D viewport pick slot.
- **UX Impact**: In multi-body assemblies where parts have generic names ("Solid 1", "Solid 2"), users cannot select target bodies by clicking them in the 3D viewport.
- **Recommendation**: Add viewport picking slots (`Slot::SolidTargetBody`, `Slot::SolidAlsoCut`) to allow direct 3D selection.

---

### Finding 3.7: Topological Edge Reference Capture Failure Aborts Fillet/Chamfer Creation
- **Location**: [`blend_tools.rs:218-245`](file:///home/tecnio/workspace/caditor/crates/caditor/src/blend_tools.rs#L218-L245)
- **Severity**: **High**
- **Description**: In `blend_tools::create()`, `EdgeReference::capture_in(&naming, edge)` attempts topological naming. If naming fails for any selected edge (e.g. periodic seams or complex intersections), line 240 aborts feature creation with `BlendError::SomeEdgesGone`.
- **UX Impact**: Users experience `"Some of the selected edges are no longer part of the model"` errors when attempting to fillet visibly valid edges.
- **Recommendation**: Enhance fallback edge naming and provide descriptive warnings instead of failing the entire feature.

---

### Finding 3.8: Target Body Is Immutable in Blend Panels
- **Location**: [`blend_panel.rs:452`](file:///home/tecnio/workspace/caditor/crates/caditor/src/blend_panel.rs#L452)
- **Severity**: **Medium**
- **Description**: The target body is rendered as a static text label (`feature_fields::feature_row`).
- **UX Impact**: Users cannot reassign a fillet or chamfer to another body if selected incorrectly.
- **Recommendation**: Allow changing `blend.body` via a picker combo and viewport slot.

---

### Finding 3.9: Shell Rejects Non-Analytic Faces & Silently Purges Face Thickness Overrides
- **Location**: [`shell_tools.rs:89-94, 210-216`](file:///home/tecnio/workspace/caditor/crates/caditor/src/shell_tools.rs#L89-L94)
- **Severity**: **High**
- **Description**: `openable_face` rejects any non-analytic face (splines, lofts, sweeps). Furthermore, lines 210–212 call `without_own_thickness` when opening a face, permanently deleting any custom thickness overrides previously assigned to it.
- **UX Impact**: Non-analytic surfaces cannot be shelled, and toggling open faces wipes custom thickness parameters.
- **Recommendation**: Retain disabled `FaceThickness` entries when faces are temporarily opened, and provide approximate offset fallbacks.

---

### Finding 3.10: Shell "Own Thickness" Viewport Picking Desynchronization
- **Location**: [`shell_panel.rs:226-236`](file:///home/tecnio/workspace/caditor/crates/caditor/src/shell_panel.rs#L226-L236), [`shell_tools.rs:338-348`](file:///home/tecnio/workspace/caditor/crates/caditor/src/shell_tools.rs#L338-L348)
- **Severity**: **High**
- **Description**: Clicking "Open faces" enters `EditingCommand::OpenSolid(id)` (showing the pre-shelled solid mesh). However, clicking "Choose faces" for custom thickness does not enter `OpenSolid`.
- **UX Impact**: The user clicks on the already-shelled solid in the viewport. Clicking an inner wall face fails in `clicked_face` because the face key belongs to the shelled output, throwing `ShellError::NotAFace`.
- **Recommendation**: Automatically activate `EditingCommand::OpenSolid(id)` when picking custom thickness faces (`Slot::ShellOwnThickness`).

---

### Finding 3.11: Combine Selection Order Inversion When Choosing Bodies in Feature Tree
- **Location**: [`combine_tools.rs:108-124`](file:///home/tecnio/workspace/caditor/crates/caditor/src/combine_tools.rs#L108-L124)
- **Severity**: **High**
- **Description**: When two bodies are selected in the feature tree, `selected_bodies` sorts them according to their creation index in the document, completely ignoring which body the user clicked first.
- **UX Impact**: If a user selects Body 2 (target) and Body 1 (tool) in the tree, Caditor inverts the order, making Body 1 the target and Body 2 the tool. Booleans cut the wrong part.
- **Recommendation**: Preserve click order when selecting bodies in the tree (using `state.chosen()` in selection order).

---

### Finding 3.12: Hidden Bodies Trigger Misleading "Recompute" Pending Error in Combine
- **Location**: [`combine_tools.rs:78-87`](file:///home/tecnio/workspace/caditor/crates/caditor/src/combine_tools.rs#L78-L87)
- **Severity**: **High**
- **Description**: If a body is hidden (`!visibility::is_shown`), `combine_tools::pair` fails with `CombineError::NoShape` (mapped to `Pending::BodiesShape`).
- **UX Impact**: The UI displays `"waiting for the model to recompute"`, misleading the user into thinking Caditor is hung when the body is simply hidden.
- **Recommendation**: Add an explicit error: `CombineError::BodyHidden` -> `"Body is hidden; show it before combining"`.

---

### Finding 3.13: Arbitrary Last-Body Guessing for Unattached Hole Placement
- **Location**: [`hole_tools.rs:295-309`](file:///home/tecnio/workspace/caditor/crates/caditor/src/hole_tools.rs#L295-L309)
- **Severity**: **High**
- **Description**: If a hole sketch is not attached to a body face, lines 301–306 arbitrarily pick the *last* body in the document.
- **UX Impact**: In multi-body designs, holes drill into unrelated bodies located far from the sketch plane.
- **Recommendation**: Use ray collision along the hole normal to determine the intersected body, or prompt the user.

---

### Finding 3.14: Rigid Tree-Order Rejection When Referencing Datum Geometry
- **Location**: [`datum_tools.rs:98-139`](file:///home/tecnio/workspace/caditor/crates/caditor/src/datum_tools.rs#L98-L139)
- **Severity**: **Medium**
- **Description**: Selecting any geometric entity created later in the tree throws `UnusableSelection::AfterThisPoint` / `DatumMadeLater`.
- **UX Impact**: Reference geometry creation is blocked unless the user manually pre-positions the rollback bar.
- **Recommendation**: Offer an option to insert the datum below the referenced feature automatically.

---

### Finding 3.15: Fractional Pattern Step Spacing Generates Raw Un-Evaluated Binary Expressions
- **Location**: [`pattern_panel.rs:108-136`](file:///home/tecnio/workspace/caditor/crates/caditor/src/pattern_panel.rs#L108-L136)
- **Severity**: **Medium**
- **Description**: When switching from Total to BetweenCopies spacing, if `value / steps` has a fractional remainder, Caditor outputs `Expression::binary(Divide, spacing, steps)`.
- **UX Impact**: Setting 5mm total with 2 steps fills the field with `5 mm / 2` instead of `2.5 mm`.
- **Recommendation**: Evaluate and format fractional spacing as clean decimal measures.

---

## 4. Direct Manipulation & Viewport Gizmos

### Finding 4.1: Skew-Ray Hyperbolic Jump Divergence During Linear Handle Drag
- **Location**: [`reach_handles.rs:317-365`](file:///home/tecnio/workspace/caditor/crates/caditor/src/reach_handles.rs#L317-L365), [`caditor-geometry/src/ray.rs:29-38`](file:///home/tecnio/workspace/caditor/crates/caditor-geometry/src/ray.rs#L29-L38)
- **Severity**: **Critical**
- **Description**: In `ray.rs`, `closest_along_line` calculates line-to-ray distance using:
  ```rust
  let apart = 1.0 - across * across;
  if apart < PARALLEL_TOLERANCE { return None; }
  Some((across * offset.dot(self.direction) - offset.dot(along)) / apart)
  ```
  When the camera views the handle nearly parallel to its direction vector, `apart` approaches zero (e.g. `0.001`). Moving the mouse by one pixel shifts `offset.dot(self.direction)` slightly, which divides by `apart`, causing the handle position to explode by thousands of millimeters in a single frame.
- **UX Impact**: Dragging an extrusion arrow while viewing from an angle close to its axis causes the extrusion to jump violently out of the viewport.
- **Recommendation**: Implement screen-space plane projection for dragging: project the cursor delta onto a plane perpendicular to the camera look direction containing the handle, and take the dot product with the handle axis.

---

### Finding 4.2: Turn Handles Lock Up in Edge-On Dead Zones
- **Location**: [`turn_handles.rs:98-106`](file:///home/tecnio/workspace/caditor/crates/caditor/src/turn_handles.rs#L98-L106)
- **Severity**: **High**
- **Description**: In `turn_handles.rs`, angle dragging intersects the mouse ray with the revolution plane via `ray.intersect_plane(&plane)`. When the user views the revolution plane edge-on, `intersect_plane` returns `None`.
- **UX Impact**: Revolution handles freeze and become completely unresponsive when viewed edge-on.
- **Recommendation**: Switch to cylindrical tangent projection or screen-space angular delta when viewing edge-on.

---

### Finding 4.3: Orthographic Projection Hides ValueHandles When Camera Sits Forward of Target
- **Location**: [`value_handles.rs:83-88`](file:///home/tecnio/workspace/caditor/crates/caditor/src/value_handles.rs#L83-L88), [`caditor-render/src/camera.rs:359-365`](file:///home/tecnio/workspace/caditor/crates/caditor-render/src/camera.rs#L359-L365)
- **Severity**: **High**
- **Description**: Handle scaling is calculated as:
  ```rust
  let depth = view.view_depth(at);
  let per_point = pixels_per_point * view.units_per_pixel_at(depth);
  (depth.is_finite() && depth > 0.0 && per_point.is_finite() && per_point > 0.0).then_some(per_point)
  ```
  In orthographic projection, if a feature lies behind the virtual camera position along `forward` (standard in orthographic views with near planes behind the eye), `depth <= 0.0`. `per_point_at` returns `None`, causing all `ValueHandles` to vanish.
- **UX Impact**: Direct manipulation gizmos disappear when working in Orthographic view mode.
- **Recommendation**: Only check `depth > 0.0` in perspective mode; orthographic projection has uniform scaling across all depths.

---

### Finding 4.4: Dynamic Screen Step Clamping Artificially Blocks Small Dimensions
- **Location**: [`value_gauges.rs:819-845`](file:///home/tecnio/workspace/caditor/crates/caditor/src/value_gauges.rs#L819-L845)
- **Severity**: **Medium**
- **Description**: Almost every dimension gauge clamps values with `value.max(step)`, where `step` is computed from screen-space pixel density (`step_for(per_point)`).
- **UX Impact**: When zoomed out such that `step` is 5mm or 10mm, dragging a handle cannot set any value smaller than 5mm or 10mm.
- **Recommendation**: Decouple the minimum allowable dimension limit from the screen-space grid snap increment.

---

### Finding 4.5: Handle Snapping Fails on Faces Parallel to Drag Direction
- **Location**: [`handle_snap.rs:83-91, 201-227`](file:///home/tecnio/workspace/caditor/crates/caditor/src/handle_snap.rs#L83-L91)
- **Severity**: **High**
- **Description**: Face snapping calculates `let facing = direction.dot(normal)`. If dragging an extrusion and hovering over an adjacent side face parallel to the extrusion axis, `facing == 0.0`. `facing.abs() < SQUARE_ENOUGH` causes snapping to return `None`.
- **UX Impact**: Snapping to parallel bodies, side walls, or guide surfaces fails or shoots off to infinity.
- **Recommendation**: When a face is parallel to the drag line, snap directly to the projected point on the line.

---

### Finding 4.6: Asymmetric Flip Clamping in Two-Sided Extrusion Drag
- **Location**: [`reach_handles.rs:338-347, 401-405`](file:///home/tecnio/workspace/caditor/crates/caditor/src/reach_handles.rs#L338-L347)
- **Severity**: **Medium**
- **Description**: While `Reach::Only` allows dragging past zero to flip direction, `Reach::Symmetric`, `Reach::Forward`, and `Reach::Backward` clamp with `distance.max(self.step)`.
- **UX Impact**: Users dragging a two-sided extrusion handle hit an invisible wall at `step` and cannot collapse or flip the side.
- **Recommendation**: Allow forward and backward handles to collapse smoothly to zero.

---

## 5. Viewport Navigation, Camera & 3D Scene

### Finding 5.1: Destructive View History Without Forward / Redo Stack
- **Location**: [`view_history.rs:86-94`](file:///home/tecnio/workspace/caditor/crates/caditor/src/view_history.rs#L86-L94)
- **Severity**: **High**
- **Description**: `ViewHistory::back()` settles active gestures and pops from `self.views.pop_back()` without storing the current view on a forward stack.
- **UX Impact**: Pressing `Alt+Left` (Previous View) permanently destroys the forward camera trajectory. Users cannot navigate forward back to their perspective.
- **Recommendation**: Implement a forward view stack to support bidirectional camera navigation.

---

### Finding 5.2: Laptop Mode Hijacks Physical Mouse Wheel for Orbit/Pan
- **Location**: [`viewport.rs:2211-2226`](file:///home/tecnio/workspace/caditor/crates/caditor/src/viewport.rs#L2211-L2226)
- **Severity**: **High**
- **Description**: When `input_mode == InputMode::Laptop`, `scrolled_to_zoom` is forced to `0.0`, and all wheel scroll deltas are redirected to `pan_by` or `orbit_by`.
- **UX Impact**: Laptop users plugging in a physical USB/Bluetooth mouse find that spinning the wheel orbits or pans the viewport instead of zooming.
- **Recommendation**: Differentiate discrete mouse wheel ticks from smooth touchpad gestures.

---

### Finding 5.3: Asymmetric Zoom Inversion Between Wheel Scroll & Trackpad Pinch
- **Location**: [`viewport.rs:2227, 6444-6447`](file:///home/tecnio/workspace/caditor/crates/caditor/src/viewport.rs#L2227)
- **Severity**: **Medium**
- **Description**: `let factor = zoom_factor(&self.navigation, f64::from(scrolled_to_zoom)) / f64::from(pinch);`. `zoom_factor` respects `invert_zoom`, but `pinch` is divided directly outside `zoom_factor`.
- **UX Impact**: Enabling "Invert Zoom Direction" flips wheel zooming but leaves trackpad pinch zooming unchanged, causing contradictory behavior.
- **Recommendation**: Apply user inversion settings uniformly across wheel and gesture inputs.

---

### Finding 5.4: ViewCube Focus Lock Steals Global Arrow Shortcuts Without Checking Modifiers
- **Location**: [`view_cube.rs:334-355`](file:///home/tecnio/workspace/caditor/crates/caditor/src/view_cube.rs#L334-L355)
- **Severity**: **High**
- **Description**: When the ViewCube has focus, it traps arrow keys without checking `input.modifiers`.
- **UX Impact**: Global shortcuts combining arrow keys (`Alt+Left`, `Alt+Up`, `Shift+Arrow`) are trapped by the ViewCube and execute facet stepping instead of the user's shortcut.
- **Recommendation**: Require `input.modifiers.is_none()` before consuming arrow keys in the ViewCube.

---

### Finding 5.5: HUD Guidance Prompt Collides with ViewCube & Style Picker on Narrow Viewports
- **Location**: [`canvas_guidance.rs:260-270`](file:///home/tecnio/workspace/caditor/crates/caditor/src/canvas_guidance.rs#L260-L270), [`viewport.rs:6190-6206`](file:///home/tecnio/workspace/caditor/crates/caditor/src/viewport.rs#L6190-L6206)
- **Severity**: **Medium**
- **Description**: In `canvas_guidance.rs`, prompt centering uses `.min(band.right() - half).max(band.left() + half)`. If `band.width() < 2 * half`, `max` forces the prompt past `band.right()`.
- **UX Impact**: In split views or smaller windows, canvas prompt titles and hint chips collide with and overlap the ViewCube and Display Style picker.
- **Recommendation**: Wrap prompt titles and push chips below the top HUD line when width is constrained.

---

### Finding 5.6: Pointer Hover Labels Displace Static Corner HUD Readouts
- **Location**: [`canvas_guidance.rs:218-220`](file:///home/tecnio/workspace/caditor/crates/caditor/src/canvas_guidance.rs#L218-L220), [`viewport.rs:5632-5660`](file:///home/tecnio/workspace/caditor/crates/caditor/src/viewport.rs#L5632-L5660)
- **Severity**: **Medium**
- **Description**: Moving the pointer near the bottom-left corner causes transient pointer labels to intersect `taken`, forcing static coordinate and grid readouts to disappear or jump erratically.
- **UX Impact**: Coordinate and grid readouts flicker on and off as the cursor nears the bottom-left corner.
- **Recommendation**: Exclude pointer tooltips from the reservation list for corner HUD elements.

---

## 6. Selection, Picking & Occlusion Geometry

### Finding 6.1: Crossing Box Selection Misses Faces Completely Inside Selection Box
- **Location**: [`box_selection.rs:430-436`](file:///home/tecnio/workspace/caditor/crates/caditor/src/box_selection.rs#L430-L436)
- **Severity**: **Critical**
- **Description**: In `faces_within(...)`, `BoxMode::Crossing` tests:
  ```rust
  area.crosses(*a, *b) || area.crosses(*b, *c) || area.crosses(*c, *a) || inside_triangle(centre, [*a, *b, *c])
  ```
  It tests if any triangle edge crosses the box boundary OR if the box center is inside the triangle. It **omits** testing if any vertex of the face is inside the box (`area.contains(*a)`).
- **UX Impact**: If a face is completely contained within a crossing selection box and does not cross the box boundaries or enclose the center point of the box, it is completely ignored and unselected.
- **Recommendation**: Add vertex containment to the crossing test: `area.contains(*a) || area.contains(*b) || area.contains(*c)`.

---

### Finding 6.2: Ray Picking Best Hit Sorting Ignores Camera Z-Depth
- **Location**: [`scene.rs:235-253`](file:///home/tecnio/workspace/caditor/crates/caditor/src/scene.rs#L235-L253)
- **Severity**: **High**
- **Description**: `PickTable::best_hit` sorts candidate hits purely by `(priority, hit.offset_points)`, where `offset_points` is 2D screen-space pixel distance. Camera ray depth is not evaluated.
- **UX Impact**: In wireframe, transparent styles, or dense geometry, a back edge or vertex that happens to be 0.1px closer in screen space will be picked over the foreground edge facing the camera.
- **Recommendation**: Incorporate view-space Z-depth into candidate hit ranking: sort by `(priority, depth_bucket, offset_points)`.

---

### Finding 6.3: X-Ray Mode Treats Outer Faces as Occluding Horizons
- **Location**: [`viewport.rs:3144-3162`](file:///home/tecnio/workspace/caditor/crates/caditor/src/viewport.rs#L3144-L3162), [`display_style.rs:61-63`](file:///home/tecnio/workspace/caditor/crates/caditor/src/display_style.rs#L61-L63)
- **Severity**: **High**
- **Description**: In `seen_through`, `front` filters hits with `hides_what_lies_behind(*pickable)`. This unconditionally matches `Pickable::Face` even in X-Ray mode, setting `deepest_seen = front + slack` and discarding deeper hits. Furthermore, `picks_faces` is false in X-Ray mode.
- **UX Impact**: In X-Ray mode, users can neither pick outer faces nor pick internal or rear edges behind them.
- **Recommendation**: When in X-Ray mode, do not truncate `deepest_seen` by front faces (set `deepest_seen = f64::INFINITY`).

---

### Finding 6.4: Whole Body Selection Explodes into Thousands of Individual Faces
- **Location**: [`selection.rs:48-62, 570-580`](file:///home/tecnio/workspace/caditor/crates/caditor/src/selection.rs#L48-L62)
- **Severity**: **Medium**
- **Description**: `SelectionFilter::Bodies` resolves every individual face of a body and inserts thousands of `Pickable::Face` entries into `Selection`.
- **UX Impact**: Selecting complex parts causes noticeable frame drops during selection updates and undo registration.
- **Recommendation**: Represent bodies atomically as `Pickable::Body(BodyId)`.

---

### Finding 6.5: Collinear Segments Falsely Evaluated as Crossing Selection Boundaries
- **Location**: [`sketch_drag.rs:912-917`](file:///home/tecnio/workspace/caditor/crates/caditor/src/sketch_drag.rs#L912-L917)
- **Severity**: **Medium**
- **Description**: When two segments are collinear, all 2D cross-products evaluate to zero, which can register as crossing even if the segments are disjoint along the line.
- **UX Impact**: Collinear sketch lines lying outside the selection box can be falsely included in box or lasso selections.
- **Recommendation**: Add a 1D bounding box overlap check when collinearity is detected.

---

## 7. UI Shell, Keyboard Focus, Modals & Undo Safety

### Finding 7.1: Universal Escape Key Trap in Numeric Fields, Typed Points, & Active Tools
- **Location**: [`field.rs:263-266`](file:///home/tecnio/workspace/caditor/crates/caditor/src/field.rs#L263-L266), [`typed_point.rs:402-414`](file:///home/tecnio/workspace/caditor/crates/caditor/src/typed_point.rs#L402-L414), [`app.rs:603-622`](file:///home/tecnio/workspace/caditor/crates/caditor/src/app.rs#L603-L622), [`viewport.rs:4980-4987`](file:///home/tecnio/workspace/caditor/crates/caditor/src/viewport.rs#L4980-L4987)
- **Severity**: **Critical**
- **Description**: 
  1. `TextEdit::singleline` does not set `.lose_focus_on_escape(true)`.
  2. Reverting on Escape in `field.rs` only runs if `response.lost_focus()`.
  3. `app.rs` sets `keys_free = false` whenever `ui.ctx().text_edit_focused()`.
  4. Viewport tool cancellation requires `keys_free`.
- **UX Impact**: Pressing Escape while a numeric field or coordinate box is active does nothing. It does not revert the field, does not surrender focus, and cannot cancel the active tool. The user is trapped in the text field.
- **Recommendation**: Add `.lose_focus_on_escape(true)`, surrender focus explicitly on Escape, and allow Escape in `app.rs` to propagate tool cancellations even if a text edit has focus.

---

### Finding 7.2: Unbounded Document-Wide Undo in Sketch Mode Traps Cancellation
- **Location**: [`sketch_toolbar.rs:1003-1017`](file:///home/tecnio/workspace/caditor/crates/caditor/src/sketch_toolbar.rs#L1003-L1017), [`editing.rs:1010-1037`](file:///home/tecnio/workspace/caditor/crates/caditor/src/editing.rs#L1010-L1037)
- **Severity**: **Critical**
- **Description**: Undo in sketch mode is not scoped to the sketch session; it undos global document transactions past the sketch entry point. Once this occurs, `changes_since` returns `CancelRefusal::HistoryChanged`.
- **UX Impact**: The "Cancel sketch" button is permanently disabled, and the sketch session cannot be cleanly rolled back.
- **Recommendation**: Scope sketch undo to the sketch entry mark, or confirm before undoing past the session boundary.

---

### Finding 7.3: Multi-Step Sketch Cancellation Applies Synthetic Forward Transaction Destroying Redo
- **Location**: [`editing.rs:1039-1051`](file:///home/tecnio/workspace/caditor/crates/caditor/src/editing.rs#L1039-L1051)
- **Severity**: **High**
- **Description**: Sketch cancellation with multiple steps calls `discard`, which applies a new forward transaction rather than popping the undo stack.
- **UX Impact**: Applying a forward transaction clears the Redo stack (`self.redo.clear()`), and pressing Undo afterwards resurrects the cancelled sketch.
- **Recommendation**: Unify sketch cancellation with solid cancellation by using sequential `Action::Undo` calls back to the entry mark.

---

### Finding 7.4: Uncancellable Solid Tool Trap on History / Undo Mismatch
- **Location**: [`editing.rs:833-847, 1010-1035`](file:///home/tecnio/workspace/caditor/crates/caditor/src/editing.rs#L833-L847)
- **Severity**: **High**
- **Description**: If document history alters while a solid tool panel is open, `changes_since` returns `CancelRefusal::HistoryChanged` or `ChangedElsewhere`. When `cancel()` encounters this refusal, it shows a warning notice but leaves `self.solid` and `self.picking` active.
- **UX Impact**: The user is permanently stuck in the solid tool panel; clicking Cancel or pressing Escape displays a notification but cannot close the tool.
- **Recommendation**: Force-abort the solid tool session (`self.solid = None; self.picking = None;`) when a cancel refusal occurs.

---

### Finding 7.5: Command Palette Aggressive Focus Stealing and Missing List Navigation
- **Location**: [`palette.rs:433-458`](file:///home/tecnio/workspace/caditor/crates/caditor/src/palette.rs#L433-L458)
- **Severity**: **High**
- **Description**: `field.request_focus()` is called on every frame, preventing `Tab` navigation. Furthermore, the palette only listens for `ArrowUp` and `ArrowDown`, lacking `PageUp`, `PageDown`, `Home`, and `End` support.
- **UX Impact**: Users cannot navigate actions via Tab or navigate large command lists efficiently.
- **Recommendation**: Only request focus on initial palette open, and implement PageUp/PageDown/Home/End navigation.

---

### Finding 7.6: Saved Views Modal Closes Completely When Canceling Inline Renaming
- **Location**: [`saved_views.rs:196-204, 329-338`](file:///home/tecnio/workspace/caditor/crates/caditor/src/saved_views.rs#L196-L204)
- **Severity**: **High**
- **Description**: When editing a view name inline, pressing Escape cancels the rename, but `widgets::dialog` also catches Escape and closes the entire modal.
- **UX Impact**: Canceling an inline rename abruptly closes the Saved Views dialog.
- **Recommendation**: Consume the Escape event when inline renaming is active.

---

### Finding 7.7: Mouse Click Cursor Placement Overwritten by Unconditional `select_all`
- **Location**: [`field.rs:220-230`](file:///home/tecnio/workspace/caditor/crates/caditor/src/field.rs#L220-L230)
- **Severity**: **Medium**
- **Description**: Clicking a numeric field unconditionally triggers `select_all`, overwriting the clicked insertion position.
- **UX Impact**: Users clicking to edit a specific digit find the entire string selected, leading to accidental overwrites.
- **Recommendation**: Only trigger `select_all` on keyboard Tab navigation.

---

### Finding 7.8: Value Stepping Fails on Non-Metric Units
- **Location**: [`stepping.rs:1, 64-66`](file:///home/tecnio/workspace/caditor/crates/caditor/src/stepping.rs#L1)
- **Severity**: **Medium**
- **Description**: `step_value` hardcodes metric unit parsing (`mm`, `cm`, `m`). Imperial units (`in`, `ft`, `thou`) fail with `Refusal::NotPlain`.
- **UX Impact**: Users working in inches or mils cannot use Up/Down arrow keys to increment dimensions.
- **Recommendation**: Support document `LengthUnit` in `step_value`.

---

## 8. Feature Tree, Multi-Body & Parametric Recompute

### Finding 8.1: Absence of Active Body Context & Silent Collision Auto-Merging
- **Location**: [`bodies_tree.rs:44-75`](file:///home/tecnio/workspace/caditor/crates/caditor/src/bodies_tree.rs#L44-L75), [`solid_tools.rs:837-844`](file:///home/tecnio/workspace/caditor/crates/caditor/src/solid_tools.rs#L837-L844), [`extrusion_detection.rs:84-125`](file:///home/tecnio/workspace/caditor/crates/caditor/src/extrusion_detection.rs#L84-L125)
- **Severity**: **Critical**
- **Description**: Caditor has no concept of an active body. In `solid_tools::default_operation`, if an extrusion touches any existing body, it automatically assigns `BodyOperation::Add(reached_body)`.
- **UX Impact**: In multi-part assemblies (pins in holes, shafts in sleeves, lids on boxes), new parts silently fuse into adjacent bodies unless the user manually intervenes.
- **Recommendation**: Implement an Active Body concept, and prompt or default to `NewBody` when multi-body documents detect intersections.

---

### Finding 8.2: Zero-Height Fold Trap for Rollback Bar in Collapsed Groups
- **Location**: [`feature_tree.rs:262-266, 612-615`](file:///home/tecnio/workspace/caditor/crates/caditor/src/feature_tree.rs#L262-L266)
- **Severity**: **High**
- **Description**: Features inside collapsed folders have zero-height bounding boxes. Dragging the rollback bar over a collapsed folder cannot target any position inside the group; it jumps above or below the folder entirely.
- **UX Impact**: Dragging the rollback bar over folders is glitchy, and the bar renders on top of collapsed folder headers.
- **Recommendation**: Auto-expand collapsed folders when hovering the rollback bar over them.

---

### Finding 8.3: Feature Suppression Lacks Cascading Dependency Confirmation
- **Location**: [`feature_tree.rs:2087-2095`](file:///home/tecnio/workspace/caditor/crates/caditor/src/feature_tree.rs#L2087-L2095), [`caditor-document/src/tree.rs:61-76`](file:///home/tecnio/workspace/caditor/crates/caditor-document/src/tree.rs#L61-L76)
- **Severity**: **High**
- **Description**: Suppressing a feature does not cascade to dependent features, unlike deletion.
- **UX Impact**: Suppressing a base sketch leaves all downstream extrusions and fillets active, triggering a cascade of red recompute errors.
- **Recommendation**: Offer a "Suppress with dependents" confirmation dialog.

---

### Finding 8.4: Draft Preview Drops and Flickers on Intermediate Typing
- **Location**: [`feature_fields.rs:319-323`](file:///home/tecnio/workspace/caditor/crates/caditor/src/feature_fields.rs#L319-L323), [`model.rs:650-669`](file:///home/tecnio/workspace/caditor/crates/caditor/src/model.rs#L650-L669)
- **Severity**: **High**
- **Description**: Typing an intermediate invalid string (e.g. `"10."`, `"5+"`) causes `validate` to fail, immediately dropping the draft preview and reverting the viewport to the committed model.
- **UX Impact**: The 3D viewport visibly flickers on every intermediate keystroke while typing values.
- **Recommendation**: Retain the last valid draft preview during temporary syntax errors.

---

### Finding 8.5: Face Appearance and Topology Desynchronization During Draft Preview
- **Location**: [`scene.rs:1206-1229`](file:///home/tecnio/workspace/caditor/crates/caditor/src/scene.rs#L1206-L1229)
- **Severity**: **High**
- **Description**: Draft previews render using `bodies.draft().mesh`, but face appearances are calculated against the committed evaluation.
- **UX Impact**: Draft previews render with inverted colors or glitchy face materials until fully committed.
- **Recommendation**: Use `draft.evaluation` when assigning face looks to draft meshes.

---

### Finding 8.6: Absence of Visual Dependency Graph Indicators in Tree and Viewport
- **Location**: [`feature_tree.rs:1862-1933`](file:///home/tecnio/workspace/caditor/crates/caditor/src/feature_tree.rs#L1862-L1933)
- **Severity**: **Medium**
- **Description**: Feature dependencies are only visible through a nested context menu.
- **UX Impact**: Understanding parametric parent/child chains requires tedious right-clicking.
- **Recommendation**: Render visual connection brackets in the tree margin and highlight referenced geometry in the viewport.

---

## 9. Accessibility & Visual Contrast Compliance

### Finding 9.1: Critical WCAG 2.1 AA Contrast Failures Across Canvas Themes
- **Location**: [`scene_palette.rs:165-170, 1039-1099`](file:///home/tecnio/workspace/caditor/crates/caditor/src/scene_palette.rs#L165-L170)
- **Severity**: **Critical**
- **Description**: 
  - `STUDIO_CANVAS` vs `LIGHT_STANDARD.background_body`: Contrast ratio is **1.03:1**.
  - `BLUE_GREY_CANVAS` vs `background_body`: Contrast ratio is **1.01:1**.
  - `GREY_CANVAS` vs `background_body`: Contrast ratio is **1.36:1**.
  - Inactive sketch curves against backgrounds yield contrast between **1.2:1 and 1.8:1**.
  - WCAG 2.1 AA (SC 1.4.11) requires at least **3.0:1** for graphical user interface objects.
- **UX Impact**: Inactive solid bodies and reference sketches completely vanish into the background in Studio, Grey, and Blue-Grey presets.
- **Recommendation**: Adjust `background_body` and curve shades dynamically to ensure a minimum 3.0:1 contrast ratio against all canvas backgrounds.

---

### Finding 9.2: ViewCube Orientation Triad Axes Lack High-Contrast Halo
- **Location**: [`view_cube.rs:600-660`](file:///home/tecnio/workspace/caditor/crates/caditor/src/view_cube.rs#L600-L660)
- **Severity**: **Medium**
- **Description**: Triad axes (X: Red, Y: Green, Z: Blue) are rendered as bare 1.5px lines without an outline or background halo. On dark surfaces, the Blue axis drops below 1.5:1 contrast.
- **UX Impact**: Triad axes become illegible against dark backgrounds or dark CAD bodies.
- **Recommendation**: Render a 1px contrasting halo or shadow behind triad lines and labels.

---

## 10. Prioritized Action Plan & Remediation Roadmap

| Rank | Issue ID | Area | File | Summary of Remediation |
|---|---|---|---|---|
| **P0** | 3.1 | Modeling | `extrusion_detection.rs` | Support TwoSides/Symmetric; never default to Cut on reach timeout. |
| **P0** | 3.2 | Modeling | `extrusion_detection.rs` | Never silently convert finite extrusion distance to infinite `ThroughAll`. |
| **P0** | 4.1 | Gizmos | `reach_handles.rs`, `ray.rs` | Replace skew-ray intersection with camera-plane delta projection to prevent infinity jumps. |
| **P0** | 6.1 | Selection | `box_selection.rs` | Add vertex containment checks (`area.contains`) to crossing box selection. |
| **P0** | 7.1 | UI Focus | `field.rs`, `typed_point.rs` | Add `lose_focus_on_escape(true)` and surrender focus to eliminate universal Escape traps. |
| **P0** | 7.2 | History | `sketch_toolbar.rs`, `editing.rs` | Scope sketch undo to session mark; prevent undoing past sketch boundary. |
| **P0** | 8.1 | Multi-Body | `bodies_tree.rs`, `solid_tools.rs` | Implement Active Body concept; prevent silent collision auto-merging. |
| **P0** | 9.1 | A11y | `scene_palette.rs` | Enforce WCAG ≥3.0:1 contrast for background bodies and curves across all canvas themes. |
| **P1** | 1.1 | Sketching | `drawing.rs` | Retain multi-segment chain memory across tool switches. |
| **P1** | 1.2 | Sketching | `drawing.rs` | Decouple `remove_last` step-back from rigid string matching against `model.undo_label()`. |
| **P1** | 2.3 | Dimensions | `viewport.rs`, `sketch_toolbar.rs` | Route dimension placement through `ConstraintTrial` off-thread validation. |
| **P1** | 2.4 | Solver | `sketch_tools.rs` | Check 1D constraint freedom in `determines` rather than requiring full 2D fixity. |
| **P1** | 2.6 | Solver | `model.rs` | Validate drag-release join constraints through `ConstraintTrial` before commit. |
| **P1** | 2.10 | Solver | `reach.rs` | Fix finite difference probing cancellation and retain curvature signs in drag reach. |
| **P1** | 3.3 | Modeling | `extrusion_detection.rs` | Default to `NewBody` on edgewise non-manifold contact. |
| **P1** | 3.4 | Modeling | `solid_tools.rs` | Respect selected `Pickable::SketchRegion` instead of forcing `RegionChoice::All`. |
| **P1** | 3.6 | Modeling | `solid_panel.rs` | Add 3D viewport pick slot for solid target and cut bodies. |
| **P1** | 3.7 | Modeling | `blend_tools.rs` | Fix edge capture naming failure in fillets/chamfers. |
| **P1** | 3.9 | Modeling | `shell_tools.rs` | Retain custom face thickness overrides when toggling open faces. |
| **P1** | 3.10 | Modeling | `shell_panel.rs` | Activate `OpenSolid` mode when picking custom thickness faces. |
| **P1** | 3.11 | Modeling | `combine_tools.rs` | Preserve user click order when selecting bodies in the tree. |
| **P1** | 3.12 | Modeling | `combine_tools.rs` | Fix misleading recompute error when combine bodies are hidden. |
| **P1** | 4.2 | Gizmos | `turn_handles.rs` | Fix edge-on dead-zone freeze in revolution handles. |
| **P1** | 4.3 | Gizmos | `value_handles.rs` | Allow ValueHandles in orthographic projection when depth <= 0.0. |
| **P1** | 4.5 | Gizmos | `handle_snap.rs` | Fix snapping failure on faces parallel to drag direction. |
| **P1** | 5.1 | Navigation | `view_history.rs` | Add forward view stack to prevent destructive view history loss. |
| **P1** | 5.2 | Navigation | `viewport.rs` | Differentiate discrete mouse wheel ticks from touchpad gestures in laptop mode. |
| **P1** | 5.4 | Navigation | `view_cube.rs` | Require modifier-free arrow keys before ViewCube traps keyboard events. |
| **P1** | 6.2 | Picking | `scene.rs` | Incorporate camera Z-depth into best-hit picking ranking. |
| **P1** | 6.3 | Picking | `viewport.rs` | Bypass front-face occlusion horizon in X-Ray display mode. |
| **P1** | 7.3 | History | `editing.rs` | Roll back sketch cancellation using sequential Undo calls rather than forward transaction. |
| **P1** | 7.4 | Modals | `editing.rs` | Force-abort solid tool sessions when cancel refusal occurs. |
| **P1** | 7.5 | Palette | `palette.rs` | Cease per-frame focus stealing; add PageUp/PageDown list navigation. |
| **P1** | 7.6 | Modals | `saved_views.rs` | Consume Escape during inline renaming to avoid closing modal dialog. |
| **P1** | 8.2 | Tree | `feature_tree.rs` | Auto-expand collapsed folders when hovering rollback bar over them. |
| **P1** | 8.3 | Tree | `feature_tree.rs` | Add cascading dependency confirmation dialog to feature suppression. |
| **P1** | 8.4 | Recompute | `feature_fields.rs` | Retain previous valid draft during intermediate typing to eliminate viewport flicker. |
| **P1** | 8.5 | Recompute | `scene.rs` | Use `draft.evaluation` for face appearance mapping during draft previews. |
| **P2** | 1.3 | Shortcuts | `viewport.rs` | Restrict point step-back to Backspace; prevent Delete key from deleting selection during creation. |
| **P2** | 2.1 | Dimensions | `dimensioning.rs` | Replace interval containment with angular sectors in dimension orientation heuristic. |
| **P2** | 2.2 | Dimensions | `dimensioning.rs` | Allow placing Radius inside arc sector; support diameter dimensions on arcs. |
| **P2** | 2.5 | Dimensions | `dimensioning.rs` | Provide interactive prompt to convert overconstraining dimensions to driven references. |
| **P2** | 2.7 | Snapping | `tracking.rs` | Forward user inference toggle settings into entity dragging. |
| **P2** | 2.8 | Snapping | `snap.rs` | Prioritize active sketch curves over background 3D body vertices. |
| **P2** | 2.9 | Snapping | `snap.rs` | Add branch disambiguation to multi-crossing intersection snaps. |
| **P2** | 2.11 | Toolbar | `sketch_toolbar.rs` | Route `ToggleConstraintActive` through `Action::Trial`. |
| **P2** | 2.12 | Tidying | `tidy_panel.rs` | Validate automated tidying batches through `ConstraintTrial`. |
| **P2** | 3.5 | Modeling | `solid_panel.rs` | Preserve user-selected regions when switching from "All" to "Chosen". |
| **P2** | 3.8 | Modeling | `blend_panel.rs` | Allow reassigning target body in blend panels. |
| **P2** | 3.14 | Modeling | `datum_tools.rs` | Offer to insert datum below referenced feature automatically. |
| **P2** | 3.15 | Modeling | `pattern_panel.rs` | Format fractional pattern spacing as clean decimal measure. |
| **P2** | 4.4 | Gizmos | `value_gauges.rs` | Decouple minimum dimension limit from screen-space snap step. |
| **P2** | 4.6 | Gizmos | `reach_handles.rs` | Allow forward/backward extrusion handles to collapse smoothly to zero. |
| **P2** | 5.3 | Navigation | `viewport.rs` | Apply zoom inversion setting uniformly across wheel scroll and pinch gesture. |
| **P2** | 5.5 | HUD | `canvas_guidance.rs` | Wrap guidance prompt text and push chips below HUD elements when width is constrained. |
| **P2** | 5.6 | HUD | `canvas_guidance.rs` | Exclude pointer tooltips from corner HUD reservation list. |
| **P2** | 6.4 | Selection | `selection.rs` | Represent bodies atomically as `Pickable::Body` instead of expanding to all faces. |
| **P2** | 6.5 | Selection | `sketch_drag.rs` | Add 1D bounding box overlap check for collinear segment selection. |
| **P2** | 7.7 | Input | `field.rs` | Only trigger `select_all` on Tab navigation, preserving clicked cursor position. |
| **P2** | 7.8 | Input | `stepping.rs` | Support document length units in arrow key value stepping. |
| **P2** | 8.6 | Tree | `feature_tree.rs` | Render visual dependency graph connector lines in tree and viewport. |
| **P2** | 9.2 | A11y | `view_cube.rs` | Add 1px contrasting halo/stroke behind ViewCube triad axes. |
| **P3** | 1.4 | Shortcuts | `viewport.rs` | Require confirmation or threshold before `Escape` terminates sketch session. |
| **P3** | 1.5 | Graphics | `annotations.rs` | Include curve bounding boxes in annotation extent calculation. |
| **P3** | 2.13 | UI | `viewport.rs` | Report limiting constraint name in blocked drag tooltip. |
| **P3** | 2.14 | Snapping | `body_snap.rs` | Add distance/frustum culling to background body snap projections. |
| **P3** | 7.9 | Lifecycle | `editing.rs` | Display notification explaining why empty sketch with dependents was retained. |
