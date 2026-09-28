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

Features are added only once they meet both bars; an unpolished feature is not shipped. An item
is implemented when it works end to end in the app, not when the APIs exist. Implemented items
and resolved decisions are removed from this file, and a milestone disappears once it is empty.
Git history is the record of what was done.

## Open decisions

These need an answer before the milestone that depends on them starts.

- [ ] **Distribution** (blocks M8): Flatpak, AUR and/or plain release binaries.

## M5: Solid modelling

`caditor-kernel` already has the geometry, topology, validation, tessellation, planar profiles,
the extrude and revolve builders, frozen face and edge names, intersections, point
classification and booleans. Nothing outside the kernel uses it yet.

- [ ] Solid features in the document and file format: extrude, revolve and pocket or cut from
      a sketch's regions, with new body, add, remove and intersect operations and the body
      state chained through the tree, a failed feature skipped rather than blocking the rest
- [ ] Solid rendering in `caditor-render`: shaded triangle meshes with pickable faces and edges
- [ ] Solid modelling in the app: extrude and revolve tools, region choice, a property panel
      for each feature, and face and edge selection
- [ ] References to generated faces and edges that resolve through their names, choosing among
      the fragments of a split face by its neighbours, so that edits upstream do not rewire
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
