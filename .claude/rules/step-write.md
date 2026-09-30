---
paths:
  - "crates/caditor-step/src/write/**"
  - "crates/caditor-step/src/lib.rs"
---

# STEP writer

- `caditor-step` speaks STEP (ISO 10303-21, AP214 `AUTOMOTIVE_DESIGN`); it depends only on the
  kernel and geometry crates, and only `caditor-file` uses it.

## Structure

- `write_step` writes named kernel solids as one product, named after the model, or after the body
  when there is only one. Its `ADVANCED_BREP_SHAPE_REPRESENTATION` holds one `MANIFOLD_SOLID_BREP`
  per lump, or a `BREP_WITH_VOIDS` whose voids are `ORIENTED_CLOSED_SHELL`s of inverted faces.
- Shells are told apart by the sign of their meshed volume (only bodies with several shells are
  meshed); a body whose shells cannot be sorted is refused as `WriteError::Shells`.
- Millimetres and radians, uncertainty `LINEAR_RESOLUTION`, no author or organisation.

## Geometry

- Every kernel surface and curve has an exact STEP form: planes, cylinders, spheres and tori as they
  are; cones with a negative half angle on a flipped axis; extrusions and revolutions as
  `SURFACE_OF_LINEAR_EXTRUSION` and `SURFACE_OF_REVOLUTION`; B-splines with knot runs (rational ones
  as the complex entity); intersection curves as the cubic B-spline of their Hermite segments over
  the edge.
- Face `same_sense` is the face sense, since the kernel's normals are STEP's.

## Text and verification

- Reals print as the shortest round-tripping decimal with a point. Text escapes quotes, backslashes
  and non-ASCII: `\X2\` within the Basic Multilingual Plane, `\X4\` beyond it.
- The output was checked against OpenCascade (valid, closed, same volume) for every kind of face.
