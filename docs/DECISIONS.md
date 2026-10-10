# caditor decisions

Open decisions: large directions the project has not committed to, and smaller questions an
implementer cannot settle alone. Each is recorded in the rules file or `docs/` that covers it once
decided, and removed from here.

Entries are tagged and ordered as `ROADMAP.md` describes.

## Scope decisions

These are open: each is a large direction the project has not committed to, and each needs a
decision recorded in `docs/` before work starts.

- [high · hard · blocked by: a scope decision recorded in `docs/`] Assemblies: a model is one part
  of several bodies, with no components, instances of another model file, joints or mates, exploded
  views or bill of materials, so a product of several parts cannot be put together or checked for
  fit. Decide whether caditor stays a part modeller, or how assemblies reference part files while
  keeping references stable across edits. The same decision covers deriving: bringing the bodies,
  sketches or parameters of another model file into this one, linked so they update when that file
  changes, which a skeleton model driving several parts, or a part fitted to its neighbour, needs.
- [medium · hard · blocked by: a scope decision recorded in `docs/`] Surface modelling: no surface
  bodies, so no thicken, offset surface, trim, extend, patch or knit to a solid, which shaped
  consumer parts and repairing open STEP imports need. Also stitch and unstitch, boundary fill
  (a solid from the cell several surfaces and bodies enclose), ruled and sweep or loft surfaces,
  and freeform (T-spline) shaping, which Fusion keeps in its own environment.
- [low · hard · blocked by: a scope decision recorded in `docs/`] Sheet metal: no flanges, bends
  with a bend allowance, or flat patterns, though laser-cut and bent parts are a common use; flat
  patterns would go out through the DXF export of a flat face.

## Open questions

- [low · easy] Trim cuts at the reference axes as at any curve (`Cutter::Axis`), so a circle
  centred on the X axis, as a lever's pivot on the origin or a slot along the axis is, loses only
  the quarter between a tangent line's touching point and the axis when its inside is picked: the
  first click leaves an arc ending on the axis, held there by a `Coincident` with it, and a second
  click takes the rest. The same slot drawn off the axis trims in one click per end with every end
  joined to its line. Open decision for `trim.rs`: whether an axis should cut only a curve that no
  other curve cuts, or never.
- [low · medium] Open decision on kept measurements: a failed or suppressed measurement fails the
  features using its value, as a failing feature's dependents do; whether they should instead keep
  its last reading (the measured parameter's stored value already holds it) is undecided.
