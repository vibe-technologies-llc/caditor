# caditor features

What caditor cannot do yet: missing tools, feature options and exchange formats. An entry here is
new capability; a wrong result or a refusal of something that should work belongs in `BUGS.md`.

Entries are tagged and ordered as `ROADMAP.md` describes.

## Modelling

- [high · hard] Replace face moves a round face onto a round target only about its own axis (or, for
  a cylinder, a parallel one of its radius): a cylinder onto a coaxial cone, whose angle needs
  `Taper`'s hinge to move too, a target off its axis at another radius, a flat face onto a curved
  target (a sketch's extrusion, a spline) are refused, as are round faces that would merge. Move face
  carries the chosen faces rigidly: a fillet or chamfer along a moved face's edge must be chosen with
  it (it is not re-blended to the new angle), a moved face that was tangent to one that stays is
  refused. Delete face closes a gap only from the faces around it: not a face with several loops
  that are not whole loops of the faces around it (a hole breaking out through a side), a riser
  chosen with fillets along its foot, nor a gap that only faces further away could close.
- [medium · hard] Offset face moves planes, cylinders, cones, spheres and tori only: a spline,
  extrusion or revolution face becomes its offset surface (which needs a surface fitted within
  tolerance), and a fillet moved beside a plane that stays is refused rather than re-blended.
- [medium · hard] An end cannot end on a whole body (where the profile last leaves it); a revolve
  turns up to a datum or principal plane only when it holds the axis; and an end offset on curved
  or several faces moves the cut back end along the extrusion or hole, or about the revolve's
  axis (not along the face's normal, so a slanted face keeps no slant), and a tapered extrusion
  refuses it. The drag arrow for such an offset stands only where the face reached is flat.
- [medium · hard] Blends: ends at steps and T-junctions refused, and a lofted chain (an edge that is
  no line along its faces nor a circle about their axis, and the tangent chain holding it) refused
  where it ends at a rounded corner; a lofted chain's feet are not checked against other blends'
  feet; no variable radius; a round corner only for three convex straight edges meeting at three
  planes (other corners mitre); a fillet sized by chord length takes the faces meeting at one angle
  along its edges and one angle across them (no variable radius). Missing as shapes
  of their own: a full-round fillet across a narrow face between two others, setback corners where three
  fillets meet, a tangency weight, and a curvature-continuous (G2) fillet.
- [medium · hard] Shell: no spline, extrusion or revolution faces, as walls or openings (the kernel
  has no fitted offset surface); a curved face (cylinder, cone, sphere, torus) opens only on the
  body's outside where sharp edges part it from the closed faces, since a smooth or void opening
  needs the opening prism only flat faces give.
- [medium · hard] Sweep and loft: a sweep carries a sketch's regions along a chain of sketch
  curves or model edges, twisted and scaled evenly along it or through a table of twist and scale
  by fraction of the path (smooth through its rows; the table of a closed path is untried), but
  takes no guide rail, mitres only sharp corners between two lines and only when neither twisted
  nor scaled (other sharp corners are refused: no round corners), and sweeps no thin wall of open
  curves. A loft passes through closed profiles without holes (sketches, or flat faces of
  bodies through a projected sketch) in order, smooth or ruled, open or closed back to its first,
  ending at a point or leaving its end profiles square, but takes no guide rails or centreline, no
  end tangent to a neighbouring face of a body or curvature-continuous ends, and no square end at a
  middle profile.
- [medium · hard] Few helix pitch laws: Helix gives a sweep or pipe a helix (tapered or not) or a
  flat spiral of one pitch, a pitch changing evenly along it, or through a table of pitches by
  turn, and a helix a compression spring's closed ends (end turns at an end pitch, changing to the
  pitch over chosen turns) with any of them, but a table cannot be given a final pitch as well (its
  last row says where the pitch ends). A sweep along a changing pitch keeps the section's lean
  to the axis it starts with, so a round wire is a hair out of round where the pitch differs from
  the end's. Turns cannot touch (a closed coil, a profile as tall as the pitch swept along a helix
  or as wide as it along a flat spiral, or a pipe as thick as a flat spiral's pitch, needs a pitch
  a hair larger), so a closed end's turns stand a hair apart. A pipe along a flat spiral of
  changing pitch is judged by a bound that holds wherever its turns lie (the narrowest pitch
  against the stretch of the widest at the inner radius), so it can refuse a pipe that would just
  fit where the pitch is widest at the outside.
- [medium · hard] Patterns stop short of what other modellers repeat: a pattern repeats features or
  whole bodies but never chosen faces.
  A feature pattern or mirror repeats a shell only when nothing else changes the body between the
  shell and it (`repeat_shell.rs` hollows again from the body before the shell, so later features
  would be lost), refusing otherwise.
  Only extrusion and hole ends are found again where each copy lands (`landing.rs`); every other
  repeated feature is its tool placed again (a revolve up to a plane, a sweep, pipe or loft, a
  modelled tapped hole), and one pattern cannot chain a shift, a turn and a mirror.
- [medium · hard] Draft rebuilds only planes and cylinders or cones coaxial with the pull: a round
  face about another axis (a fillet along a top edge, `DraftCause::OffPull`), a sphere, torus,
  spline, extruded or revolved face (`DraftCause::NotDraftable`) is refused, since each needs its
  surface rebuilt about the neutral plane by something other than a tilt or a coaxial taper (a
  variable-radius blend, a swept or lofted surface); and a parting line is only the neutral plane,
  so a stepped parting line along chosen edges or a split face's line (each side drafted from its
  own hinge) is not offered.
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
- [low · medium] Emboss is repeated by a pattern or mirror of features as a rigid copy (a wrapped
  one is not wrapped again onto the surface the copy lands on), and refuses a sphere or other
  doubly curved face (wrapping onto one needs the chosen projection of the wrapped split above).
  Its depth on a cone is measured square to the slope, and a drafted wrapped one leans a wall
  running round the surface by the ratio of the radii at its top and the face more or less than the
  angle (an anisotropic offset would make it exact).
- [low · hard] No plastic-part features:
  the screw boss with its ribs, a lip and groove along a seam, snap fits (hook, loop, groove) and a
  rest (a flat seat on a curved face), which moulded parts need.

## Sketching

- [medium · hard] Body items (`body_snap.rs`) are snapped to by drawing and grabs, pulled by the
  held snap and picked by Select and Smart dimension, from the pointer or the highlight keys, but
  they are not acquired for tracks and never crossed by a direction or track, since they are not in
  the sketch until a point lands on them: acquiring one would need tracks that hold a point the
  shape has yet to project.
- [medium · hard] A projected edge (`projection.rs`) that an upstream trim, split or break cut into
  pieces with a gap between them fails the sketch in words rather than projecting each piece as a
  curve of its own: a projection is one sketch curve with the points it was made with, and the
  recompute that places it cannot add the curves for the other pieces. Offering them from Update
  references (the pieces projected as new curves, the first keeping the id constraints and
  dimensions hang on) needs that healing to add sketch geometry, which it does not yet.
- [medium · hard] Tools missing: a pattern of sketch geometry along a path (copies tied to the
  path would need a vector-equality or along-the-curve spacing the solver lacks), and sketch text
  set along a curve (`sketch.md`, Text): text runs straight from its point at an angle, while
  laying each letter's middle on a sketch curve turned to its tangent, as SVG `textPath` does,
  would need the layout to follow the solved curve and a placement along it the solver can hold.
  Text is also set in Inter alone: no other typeface.
- [medium · hard] A spline's points take only tangent handles, and only on fit-point splines: a
  point has no curvature handle (a second-derivative row would join `HandledLayout`'s derivative rows, but a handle line carries only direction and pull, so the curvature needs its own dimension or vector, a new constraint kind with file record and glyph); a
  point cannot be removed while keeping the shape (only approximately possible); and a point cannot be inserted into a conic
  keeping the shape (its two halves would be rational arcs with weights of their own, which no
  spline kind but the conic holds).
- [low · hard] A fit-point spline whose kept stretch holds a fit point carrying a constraint, a tangent
  handle or another curve is still refitted through its points by a trim, split or break (the
  constraints on a point go with it when the control spline replaces it), so its shape near the cut
  shifts slightly.
- [medium · hard] Sketches lie on one plane, so a sweep path or rail that bends in space (a cable
  run, a handle) cannot be drawn; only a chain of model edges gives the sweep one. A 3D sketch of
  lines, arcs and splines, with points placed by typed coordinates in space or moved off the
  sketch plane, would be the path; with it a sketch curve projected onto a curved face along a
  direction or to the nearest point, and the curve where two faces meet. The solver then holds 3D
  points and directions and the constraints that mean something there (coincident, horizontal and
  vertical in space, parallel, perpendicular, distance, fix).
- [low · hard] The offset of an ellipse, elliptical arc or spline is a free fit-point spline that
  does not follow the original: holding it would need a constraint keeping each fit point on the
  original's normal at its own parameter (a `Distance` from the curve lets every fit point slide
  along the offset and would put a dimension on each), a new constraint kind with its solver form,
  file record and glyph; the free ends of an open spline's offset slide along it for the same
  reason. Such a curve also cannot be offset within a chain of lines and arcs, whose joints would
  need the spline to meet them.

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
  over the whole text). `textPath` bends letters only as `method="stretch"` does by cutting each
  outline into pieces and mapping their control points, so a sharp corner in the path inside a
  letter's reach is bent at the cut nearest it rather than at the corner, `stretch` ignores
  `rotate` and `spacing`, and `spacing="auto"` is the chord layout of one baseline per letter, not
  an optimised fit of the glyph shapes; the font's variable subscript and superscript offsets (the
  `MVAR` table) are not read, so a heavy weight shifts as the regular one does.

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

## Technical drawings

- [medium · hard] Drawings have nothing measured on them: a drawing (`drawings.md`) lays out
  front, top, right, left and isometric views, full section views on principal planes and detail
  views on an ISO sheet with a title block and exports PDF, SVG or DXF, but has no dimensions or
  notes taken from the model (dimensions should refer to edges by `EdgeName`, so they survive
  edits as features do), no views placed or moved by hand (section and detail views always stand
  in a column right of the others, and a detail's circle is set by numbers in the panel, not
  dragged on the sheet), and the sheet is previewed only inside the Drawings panel rather than in a
  view of its own the size of the window.
- [low · medium] Views and sheets beyond the standard ones and full principal sections: section
  planes other than the principal ones (through a datum plane or a face, or offset along the
  parent's own axes), stepped (offset) and aligned sections, half and broken-out sections,
  auxiliary views square to an inclined face, broken views shortening a long part, cropped views,
  several sheets per drawing, a revision table, a title block whose cells the user lays out, and
  section hatching chosen per material (today one 45° pattern, alternating direction per cut body).
- [medium · hard · blocked by: dimensions on drawings (above)] Annotations beyond plain dimensions
  and notes: centre marks and centrelines on holes and round edges, hole and thread callouts read
  from the hole and thread features, ordinate and baseline dimensions, tolerances (plus and minus,
  limits, ISO 286 fits), geometric tolerances with datum features (ISO 1101), surface texture
  symbols (ISO 21920) and weld symbols (ISO 2553).

## Application

- [low · medium] Onshape's folders are not browsed: the documented REST API lists documents only,
  by view (`/documents` with `filter`, `owner` and `parentId`), and a folder tree needs the
  web client's own `globaltreenodes` endpoints, which are not part of the published API.
  Drawings are not offered either, only Part Studios (whole or by part) and assemblies.
- [low · medium] The rendered view's bounced light (`render.md`, the light pass) gathers from a
  reflective shadow map, so it reaches only `BOUNCE_REACH` of the light frame and comes only from
  surfaces the sun sees, and its visibility test still lets bounced light through a wall behind
  more surfaces than `BOUNCE_LAYERS` sun layers and the cross and slope maps' one layer each hold.
- [low · medium] A user cannot read an OpenEXR picture as the rendered view's surroundings
  (`image`'s `exr` feature is pure Rust but brings `pulp` and `paste`, which `cargo deny`
  refuses), nor give a material texture, bump or clear-coat maps.
- [medium · medium · blocked by: a live Fusion 360 and Autodesk account to confirm the open
  behaviour against (`DECISIONS.md`)] Upload to Fusion 360 and open online (`cloud.md`) opens only the
  uploaded file's web page, through `browser::open` on every platform. What remains:
  - Fusion's desktop URL handler on Windows (`fusion360://`, the target Autodesk's "Open in Fusion"
    links use; its exact parameters, likely the item's lineage URN and hub, are unconfirmed) started
    through a new `caditor_windows` call, never a shell, with the web page as the fallback when the
    handler is missing or refuses and a notice saying so. Linux has no desktop Fusion and keeps the
    web page.
  - Onshape behind the same command: `Provider::send` for Onshape fills `Sent`'s item, version and
    web link (its document and element ids; the documented document URL is its open target), the
    remembered destination is a workspace, and `Service` words the command for it.
  - Confirm against a live Fusion that Data Management returns `links.webView` on an
    `items:autodesk.core:File` item and version (the reading in `fusion.rs` is tested only against
    the documented shape), whether opening an uploaded STEP item creates a design by import, and
    whether Autodesk offers a public call that converts a stored STEP into a Fusion design server
    side; if it does, the command could open a design rather than an import.
- [medium · hard · blocked by: the in-app assistant connection decision (`DECISIONS.md`)] An
  assistant cannot work on the model open in the window: the headless server owns its own
  document. Connecting to the running app needs a transport the app listens on (off by default),
  the user's approval of each client, a status pill naming the connected client with a Disconnect,
  its changes landing in the user's undo history interleaved with their own, and its tool calls
  queued like `queued.rs` actions while the model recomputes, never blocking the UI thread.
- [low · hard] No automation: nothing can be driven by a script or macro, as Fusion's scripts and
  add-ins do, to make repetitive geometry, run a batch over files or add a tool. An interface
  would go through `Action`s and `Transaction`s like the UI, so scripts cannot break the model's
  rules, sharing the MCP server's `Session` layer (`mcp.md`), and would need a decision on the
  language and on safety (a script cannot reach files or the network unasked).
- [low · hard] One document per process.
- [low · hard] No localisation.

