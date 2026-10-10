# caditor features

What caditor cannot do yet: missing tools, feature options and exchange formats. An entry here is
new capability; a wrong result or a refusal of something that should work belongs in `BUGS.md`.

Entries are tagged and ordered as `ROADMAP.md` describes.

## Modelling

- [high · hard] Bodies cannot be edited directly beyond offsetting faces: no moving, deleting or
  replacing a face and no deleting a fillet or chamfer by its faces. An imported STEP body has
  no feature history, so today it can only be cut, joined, filleted or shelled; a wall too thick, a
  hole in the wrong place or a fillet to remove means remodelling it from scratch. Direct edits
  become features of their own, named from the faces they move, so they stay parametric and
  undoable.
- [medium · hard] Offset face moves planes, cylinders, cones, spheres and tori only: a spline,
  extrusion or revolution face becomes its offset surface (which needs a surface fitted within
  tolerance), and a fillet moved beside a plane that stays is refused rather than re-blended.
- [medium · hard] An end up to the next face or a curved face follows curved or several faces
  only on one side and without an offset (two sides or an offset need one flat face). An end
  cannot end on a whole body (where the profile last leaves it); an extrusion along a direction
  takes no taper, up to next or curved face; a revolve turns up to a face or plane only when it
  holds the axis, never up to a curved face or the next face it meets; and a hole stops at the
  next face only where that face is flat.
- [medium · hard] Blends: ends at steps and T-junctions refused, and a lofted chain (an edge that is
  no line along its faces nor a circle about their axis, and the tangent chain holding it) refused
  where it ends at a rounded corner; a lofted chain's feet are not checked against other blends'
  feet; no variable radius; a round corner only for three convex straight edges meeting at three
  planes (other corners mitre). Missing as shapes
  of their own: a full-round fillet across a narrow face between two others, a fillet sized by
  chord length, a fillet that runs by a rule over every edge of a kind, setback corners where three
  fillets meet, a tangency weight, and a curvature-continuous (G2) fillet.
- [medium · hard] Shell: no spline, extrusion or revolution faces, only flat faces open, one
  thickness for the whole body and always inward: no thickness per face, and no wall growing
  outward or to both sides of the faces.
- [medium · hard] Sweep along a path and loft between profiles: the kernel has only extrusion and
  revolution, so both need new kernel operations first. A sweep takes a profile and a path (a
  chain of edges or sketch curves), kept square to the path or parallel to the profile, with an
  optional guide rail, taper and twist; a loft takes two or more profiles or faces (a point may
  end it), open or closed back to the first, with optional rails or a centreline and tangent or
  curvature-continuous conditions at its ends.
- [medium · hard] Patterns stop short of what other modellers repeat: a curve pattern follows only
  the curves of one sketch, never a chain of model edges (a 3D path, the copies then turning with
  its tangent and normal), and a pattern repeats features or whole bodies but never chosen faces.
  A feature pattern or mirror never repeats a fillet, chamfer or shell (`repeatable_on` takes only
  extrusions, revolves and primitives adding or removing, and holes), so a filleted boss repeated
  by its features comes out with bare copies, and filleting every copy afterwards leaves the copies
  a raised count adds unfilleted.
  Every copy is the original's tool placed again, never recomputed where it lands (a copy of an
  extrusion up to next stops where the original did, not on the face it meets), and one pattern
  cannot chain a shift, a turn and a mirror.
- [medium · hard] Rib and web from an open profile: a rib extrudes parallel to the sketch plane
  and a web square to it, each thickened and run on to the nearest faces of the body.
- [low · medium] Some values still have no handle (`value_gauges.rs`): an end Up to next has no
  face to stand its Past the face arrow on, and a chamfer's second distance or angle, a hole's
  counterbore, countersink, slot, thread and Past the face sizes, a wedge's top length, a prism's
  sides and a curve or point pattern's count and spacing are only typed. A fillet's arrow assumes the edge's
  faces turn away from each other where it stands (the coedge sense), so on a concave edge its
  foot is off the rounded face, and a chamfer of two distances or a distance and an angle places
  its foot as an equal one would. Each would be a further `Measured` with its `Gauge`.
- [low · medium] Turning handles (a revolve's angle arrows, a move's rings, a plane's and a circular
  pattern's turn arrows) step but never snap (`Manipulating::snaps`), and a drag snaps to flat
  faces only, by their plane, while a curved face under the pointer gives nothing. Snapping a turn
  to the angle that brings a corner or edge middle into its plane, and a line handle to where it
  meets a curved face, would finish Fusion's drag-to-geometry.
- [low · medium] A hole's placement handles (`place_handles.rs`) stand only on a lone free point
  (`hole_tools::lone_point`): a hole of several points, or one measured from edges or centred on
  one (`hole_placement.rs`), drags nothing. Each free point could carry its own square, and each
  edge distance an arrow along the edge's normal committing to its `Distance` through
  `manipulator::Held`. Place by takes only edges of bodies, not another sketch's curves, datum
  axes or the origin.
- [low · medium] Mirror faces closes an opening only in one plane or on one elementary face beside
  it: an opening on an extrusion, revolution or spline face, one running all the way around a
  round face (a collar or a groove), or one spanning several curved faces is refused, since
  closing it needs a freeform patch, a ring face or a surface fitted across it.
- [low · hard] Split face wraps closed outlines only within once round a cylinder or cone, and a
  wrapped open chain may not run right round and back to the edge of the faces it started from:
  a helical stripe drawn as one outline running round more than once, a band whose ends meet
  after one turn and a chain turning back after going round all need a tool that crosses the seam
  of the unrolled window in one piece (its two edges glued into one solid, the caps then whole
  rings), which the window's separate regions cannot build; a sketch of two or more open chains
  (a stripe between two helices) is refused too. Spheres stay refused in words: a sphere has no
  flat unrolling, so wrapping onto one first needs a chosen projection (stereographic, equal area
  or along its axis) and the distortion it brings, a design decision before any kernel work.
- [low · hard] Scale is uniform: a body cannot be stretched by different factors along the three
  axes (a plane stays a plane, but a cylinder becomes an elliptical one, which the kernel's
  surfaces do not have).
- [low · hard] No silhouette split: dividing a body's faces along its outline seen from a chosen
  direction (a split face whose tool is that outline), so the parting line of a moulded or cast
  part can be a face boundary for a draft to start from.
- [medium · hard · blocked by: the sweep feature] No helix or spiral curve and no modelled threads:
  springs, coils and threaded holes and shafts cannot be modelled with real thread geometry (the
  cosmetic Thread feature covers drawing and exchange). The sweep feature (same list) needs
  the helix, and the thread feature then offers a modelled form beside the cosmetic one. A coil
  feature would be the ready tool for springs: revolutions or height and pitch, a round or square
  section, inside or outside the axis, and a taper angle.
- [medium · hard · blocked by: the sweep feature] No pipe: a round, square or triangular section
  swept along a path sketch, solid or with a wall thickness, with sharp or rounded corners, as the
  ready tool for tubing, handrails and cable runs.
- [medium · hard · blocked by: direct face edits ("Bodies cannot be edited directly")] Draft angle
  on existing faces: a fixed angle from a plane, a split at a parting line with an angle on each
  side, and an angle per face, following tangent faces as one chain.
- [low · hard · blocked by: sketch text ("Tools missing" under Sketching)] Emboss or deboss sketch
  text, or any sketch profile, onto a face, flat or curved (wrapped around it), raised or
  recessed by a depth.
- [low · hard · blocked by: draft angle (above), rib and web (above)] No plastic-part features:
  the screw boss with its ribs, a lip and groove along a seam, snap fits (hook, loop, groove) and a
  rest (a flat seat on a curved face), which moulded parts need.

## Sketching

- [medium · hard] Body items (`body_snap.rs`) are snapped to by drawing and grabs and picked by
  Select and Smart dimension, but they are not acquired for tracks, never pulled by the held snap
  (Alt) and never crossed by a direction or track, since they are not in the sketch until a point
  lands on them: acquiring one would need tracks that hold a point the shape has yet to project.
  Nor do the keyboard highlight commands reach them (`Pickable::BodyItem` is not a scene
  pickable), so picking a body item for a dimension or constraint needs the pointer.
- [medium · hard] Tools missing: a pattern of sketch geometry along a path (copies tied to the
  path would need a vector-equality or along-the-curve spacing the solver lacks), and text (a
  font, a height, bold and italic, set along a curve, its letters becoming closed regions that
  extrude).
- [medium · hard] A spline is only the control points it was drawn with: a point has no tangent or
  curvature handle to set the direction and pull of the curve there, which would be stored as
  constraints on the point rather than as positions so the solver and dimensions keep reading
  them; the degree cannot be chosen; and no point can be inserted or removed while keeping the
  shape.
- [medium · hard] A spur gear (`sketch.md`, Spur gears) is drawn once: its curves are free sketch
  geometry, not tied to the values in the Spur gear panel (expressions using parameters are read
  when it is drawn), so changing the module, teeth or pressure angle, or a parameter they use,
  means deleting it and drawing it again, and nothing holds its shape when one of its points is
  dragged. Keeping it parametric needs a gear stored in the sketch (its expressions and the curves
  it made, saved in the file) that recompute draws again from the evaluated values, holding its
  curves fixed like projected geometry; the teeth changing the number of curves is the hard part.
  A pair of meshing gears at a centre distance following from the same parameters is missing, and
  so is a sprocket for roller chain (ISO 606 pitch and roller diameter, tooth count) in the same
  tool.
- [medium · hard] The centre of an outline of odd sides and a slanted track place a point without
  a constraint keeping it there, as the sketch has no centroid or point-on-a-direction constraint;
  an odd outline with arcs has no centre at all. A drag snaps only its handle (the moving point
  nearest the press), not whichever moving point comes near a target.
- [medium · hard] Splines cannot be trimmed or extended (`TrimError::Spline`). Offset takes
  one chain at a time, leaves the free ends of an open chain sliding along their curves and cannot
  offset splines; a sketch fillet cannot round a spline and drops equal lengths and midpoints of the
  lines it shortens, as trim does.
- [low · medium] Sketch fillet and chamfer join two curves at their crossing only for lines, arcs
  and circles (`Sketch::join_at_crossing`): an elliptical arc that does not already end at the
  other curve is `NotCrossable`, though `intersect` finds its crossings; carrying its end round
  its ellipse as extend does would let it join too.
- [low · hard] An ellipse's offset is a free fit-point spline that does not follow the ellipse:
  holding it would need a constraint keeping each fit point on the ellipse's normal at its own
  parameter (a `Distance` from the ellipse lets every fit point slide along the offset and would
  put a dimension on each), a new constraint kind with its solver form, file record and glyph.
  An ellipse also cannot be offset within a chain of lines and arcs, whose joints would need the
  spline to meet them.
- [low · hard] No reference image: a photo or scan cannot be placed on a sketch plane, scaled by two
  points (or calibrated by a known distance), given an opacity, locked and traced, as a part
  copied from an existing object or a drawing needs.
- [medium · hard · blocked by: the sweep feature ("Sweep along a path" under Modelling)]
  Sketches lie on one plane, so a sweep path or rail that bends in space (a cable run, a handle)
  cannot be drawn. A 3D sketch of lines, arcs and splines, with points placed by typed coordinates
  in space or moved off the sketch plane, would be the path; with it a sketch curve projected onto
  a curved face along a direction or to the nearest point, and the curve where two faces meet. The
  solver then holds 3D points and directions and the constraints that mean something there
  (coincident, horizontal and vertical in space, parallel, perpendicular, distance, fix).
- [low · hard · blocked by: vector hidden-line removal ("Technical drawings")] Project takes edges,
  corners and the boundaries of faces, but not the outline of a body seen square to the sketch
  plane: the silhouette lines of a cylinder lying along the plane or the circle of a sphere cannot
  be brought in, so a sketch following a body's outline is drawn by hand.

## STEP import and export

- [low · easy] STEP styling is read and written in every form the standard and the tests
  cover (`step-read.md`), but has been checked only against a handful of real files with plain
  colours. Read files with see-through bodies and faces and with copies coloured on their own from
  SolidWorks, CATIA, Creo, NX, Fusion and FreeCAD (`STEP_CORPUS`), and support any styling the
  import report names as not understood.
- [low · hard] No IGES import or export, though older CAM software and many suppliers still exchange
  it.

## Drawing import and export

- [low · hard] SVG text comes in as outlines in Inter (upright and italic): vertical writing
  modes and shaping beyond pair kerning (ligatures, marks, right-to-left scripts) are not applied,
  which needs a shaping engine (rustybuzz) and bidirectional reordering rather than the per-letter
  layout `lettering.rs` does, and letters of different glyphs that overlap are not merged into one
  outline (`overlap.rs` merges each glyph alone; merging across letters needs the profile built
  over the whole text). `textPath`'s `method` and `spacing` are ignored, and `sub`, `super` and
  percentage baseline shifts use fixed shares of the font size rather than the font's own
  subscript and superscript metrics.

## Mesh import and export

- [medium · hard] Mesh import (STL, OBJ, 3MF) repairs simple defects and joins flat areas into
  planar faces, but curved areas stay faceted, a sealed hollow is filled, non-manifold edges and
  gaps of more than `MAX_FILLED_HOLE_EDGES` edges lose their whole shell, and a mesh of more than
  `MAX_FACETED_FACES` faces after joining (most scans and organic prints) is refused. An imported
  solid is stored as STEP text like any import, and the source mesh is not kept. What remains:
  - Rebuild real faces: group triangles into regions and recognise planes, cylinders, cones,
    spheres and tori within a tolerance the import chooses from the mesh's own noise (adjustable
    with a live preview), fit splines to what remains, and join them into a B-rep whose faces and
    edges are named, so a flat side takes a sketch, a hole edge a fillet and a bore its axis.
  - Fall back gracefully: what cannot be fitted stays faceted but still part of a valid solid that
    booleans, shells and direct edits (Modelling) accept, so the import never fails as a
    whole over a bad region.
  - Stay quick on scans and printer files of millions of triangles, in the background with
    progress and Cancel, and keep the source mesh in the model so the conversion can be redone at
    another tolerance later without breaking what references its faces.
  - Units: STL has none, so the import guesses from the size (a part 0.05 mm across is likely in
    metres) and offers a scale before committing, rather than leaving it to the scale item.

## Technical drawings

- [medium · hard] Vector hidden-line removal: the hidden-lines-removed display style hides edges
  by the depth buffer, which gives pixels, not curves. A drawing view needs each edge of a body
  split where faces cover it, as 2D curves in the view's plane with their visible and hidden
  pieces (silhouettes of curved faces included), computed off the UI thread and cancellable.
- [medium · hard · blocked by: vector hidden-line removal (above)] No 2D drawings
  at all: no sheet with a title block, no projected front, top, side and isometric views of the
  bodies, no section or detail views, no dimensions or notes taken from the model, and no PDF, SVG
  or DXF output of a sheet, though parts made for a workshop need one. Views update with the model
  and their dimensions refer to edges by name, so they survive edits as features do.
- [medium · hard · blocked by: 2D drawings (above)] Annotations beyond plain dimensions and notes:
  centre marks and centrelines on holes and round edges, hole and thread callouts read from the
  hole and thread features, ordinate and baseline dimensions, tolerances (plus and minus, limits,
  ISO 286 fits), geometric tolerances with datum features (ISO 1101), surface texture symbols
  (ISO 21920) and weld symbols (ISO 2553), and a title block filled from the model properties.
- [low · medium · blocked by: 2D drawings (above)] Views and sheets beyond the standard ones:
  auxiliary views square to an inclined face, broken views shortening a long part, broken-out and
  half sections, cropped views, first- or third-angle projection, ISO 5457 sheets from A4 to A0 and
  several sheets per drawing, a revision table, and section hatching chosen per material.

## Application

- [medium · easy] The command line exports only the active configuration: `caditor --export`
  (`cli.rs`, `headless.rs`) takes no configuration, though the window already exports one file
  per configuration (`configuration_export.rs`, `ConfigurationJob`), so size variants kept as
  configurations cannot be exported from a script or CI. Add `--configuration NAME` and
  `--all-configurations`, naming files as the window does; this is not the scripting entry below,
  which needs a language and safety decision this does not.
- [medium · hard] Pasting features cannot carry a feature that picks faces or edges of another
  copied feature (a fillet copied with its extrusion): face and edge names are digests over the
  feature id, so the copy is left out with the reason. Renaming them needs each picked face or edge
  found again in the copy's recomputed result (by matching it in the original's) before the paste
  is applied. Pasted features also take no group, and a copy from another model keeps none of its
  references outside the copied set.
- [low · medium] The configurations table cannot leave or enter the model as a spreadsheet:
  parameters round-trip through CSV with a preview (`caditor-file` `parameters.rs`,
  `Document::plan_parameter_import`), but configurations, the variant families usually kept in a
  spreadsheet (fastener or enclosure sizes), are typed cell by cell in their dialog. A CSV of
  rows by configured values (parameter expressions, suppression, body colour) planned and applied
  as one checked transaction would close the pair; the column header format and unknown names
  need settling first.
- [low · hard] No automation: nothing can be driven by a script or macro, as Fusion's scripts and
  add-ins do, to make repetitive geometry, run a batch over files or add a tool. An interface
  would go through `Action`s and `Transaction`s like the UI, so scripts cannot break the model's
  rules, and would need a decision on the language and on safety (a script cannot reach files
  or the network unasked).
- [low · hard] One document per process.
- [low · hard] No localisation.

