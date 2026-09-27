# caditor roadmap

## Direction

caditor is a parametric CAD application for Linux. The project is judged on two things before
anything else:

1. **User experience.** Modelling should feel direct and predictable, without the failure modes
   that FreeCAD is known for: broken references after an upstream edit, workbench and mode
   juggling, opaque errors and a UI that freezes. The concrete requirements are in
   `.claude/rules/ux.md`.
2. **Never losing work.** Crashes and data loss are catastrophic failures. The concrete
   requirements are in `.claude/rules/reliability.md`.

Features are added only once they meet both bars; an unpolished feature is not shipped. Each
milestone below is done when its items work end to end in the app, not when the APIs exist.

## Open decisions

These need an answer before the milestone that depends on them starts.

- [ ] **Geometry kernel** (blocks M5): a native Rust B-rep kernel (own, `truck`, or
      Fornjot's), or bindings to OpenCascade. Bindings conflict with the forbid-unsafe policy
      and bring in OCC's own failure modes.
- [ ] **Constraint solver** (blocks M4): our own solver in `caditor-sketch`, or an existing one.
      Whatever is chosen must be able to report degrees of freedom and point to the constraints
      in conflict.
- [ ] **File format** (blocks M3): the serialisation format and container. It has to support
      versioning, an append-only recovery journal and partial loading.
- [ ] **Distribution** (blocks M8): Flatpak, AUR and/or plain release binaries.

## M0: Foundation

- [x] `crates/*` workspace with layered geometry, sketch, document, render and app crates
- [x] winit window, wgpu viewport and egui overlay
- [x] Stable, never-reused `EntityId` and `FeatureId`
- [x] Workspace lints: forbid unsafe code, deny panicking calls, `ahash` and `parking_lot` only
- [ ] CI running build, clippy, tests and `rust-formatter --check`

## M1: Viewport

- [ ] Camera with one navigation model (orbit, pan, zoom to cursor) and smooth transitions
- [ ] Depth buffer and MSAA; f64 model to f32 GPU conversion that stays precise at large coordinates
- [ ] Grid, origin axes and principal planes
- [ ] GPU picking; hover highlight and selection
- [ ] View cube or equivalent orientation widget; fit to selection or to everything

## M2: Document core

- [ ] Every document mutation goes through commands, with full undo and redo and no exceptions
- [ ] Parameter expressions with units that any numeric field accepts
- [ ] Dependency graph between features, with incremental recompute
- [ ] Recompute on a background worker; the UI never blocks and long work can be cancelled
- [ ] Per-feature failure isolation: a failing feature keeps its last good result and gives a
      plain-language reason

## M3: Persistence

- [ ] Versioned file format with atomic save (temporary file, fsync, rename, fsync of the
      directory)
- [ ] Crash-recovery journal, with an offer to restore on startup
- [ ] Panic hook that flushes the journal
- [ ] Partial load of damaged files, with a report of what could not be recovered
- [ ] Recent files and a warning about unsaved changes on close

## M4: Sketcher

- [ ] Entities: point, line, arc, circle, then splines
- [ ] Constraints: coincident, horizontal and vertical, parallel and perpendicular, tangent,
      equal, distance, angle and radius
- [ ] Solver with live degree-of-freedom count and colouring by constraint state
- [ ] Diagnosis of conflicting and redundant constraints that names the entities involved
- [ ] Drawing tools with snapping and inferred constraints; editable on-canvas dimensions that
      accept expressions

## M5: Solid modelling

- [ ] Kernel integration, per the open decision above
- [ ] Extrude, revolve and pocket or cut from sketches
- [ ] Persistent naming for generated faces and edges, so that edits upstream do not rewire
      downstream features
- [ ] Fillet, chamfer and shell
- [ ] Sketch on a face and datum planes and axes

## M6: Interop

- [ ] STL and 3MF export
- [ ] STEP import and export
- [ ] DXF import into sketches

## M7: Polish

- [ ] Command palette and customisable keyboard shortcuts
- [ ] Preferences: units, theme and navigation sensitivity
- [ ] Accessibility: keyboard-only operation, scalable UI and readable contrast
- [ ] Onboarding: sample models and first-run hints

## M8: Release

- [ ] Packaging, per the open decision above
- [ ] Release process and changelog
