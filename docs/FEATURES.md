# caditor features

What caditor cannot do yet: missing tools, feature options and exchange formats. An entry here is
new capability; a wrong result or a refusal of something that should work belongs in `BUGS.md`.

Entries are tagged and ordered as `ROADMAP.md` describes.

## Modelling

- [high · hard] Replace face puts flat faces onto a plane only (a principal, datum or coordinate
  system plane or another flat face): a curved face, or a curved target (a sketch's extrusion,
  another body's cylinder or cone, which needs the shell's offsets to change a radius as well as
  carry a face rigidly), is refused, as are two faces meeting along an edge that would both land on
  the target and a neighbour squeezed to nothing between the face and the target (levelling a step
  with its riser standing), which both need the faces either side merged into one. Move face carries the chosen faces rigidly: a fillet or chamfer along a
  moved face's edge must be chosen with it (it is not re-blended to the new angle), a moved face that
  was tangent to one that stays is refused.
  Delete face closes a gap only from the faces around it: not a face whose sides lie beyond parallel
  faces (a step's riser), one face of a tangent chain whose ends meet other blends (the chain must go
  whole), a face with several loops that are not whole loops of the faces around it (a hole breaking
  out through a side), nor a gap that only faces further away could close.
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
- [medium · hard] Shell: no spline, extrusion or revolution faces, as walls or openings (the kernel
  has no fitted offset surface); a curved face (cylinder, cone, sphere, torus) opens only on the
  body's outside where sharp edges part it from the closed faces, since a smooth or void opening
  needs the opening prism only flat faces give.
- [medium · hard] Sweep and loft: a sweep carries a sketch's regions along a chain of sketch
  curves or model edges, twisted and scaled evenly along it, but takes no guide rail, no twist or
  scale law other than even, mitres only sharp corners between two lines and only when neither
  twisted nor scaled (other sharp corners are refused: no round corners), and sweeps no thin wall
  of open curves. A loft passes through closed profiles without holes (sketches, or flat faces of
  bodies through a projected sketch) in order, smooth or ruled, open or closed back to its first,
  ending at a point or leaving its end profiles square, but takes no guide rails or centreline, no
  end tangent to a neighbouring face of a body or curvature-continuous ends, and no square end at a
  middle profile.
- [medium · hard] No flat spiral or varying helix: Helix gives a sweep or pipe a helix of one
  pitch (tapered or not, started at an angle round its axis or at a picked point) to follow, but a
  flat spiral (a clock spring) has no tool at all, and a helix cannot change its pitch along the
  way, so a closed-end spring's closed and ground ends cannot be swept. Turns cannot touch (a
  closed coil, or a profile as tall as the pitch swept along a helix, needs a pitch a hair larger).
- [medium · hard] Patterns stop short of what other modellers repeat: a curve pattern follows only
  the curves of one sketch, never a chain of model edges (a 3D path, the copies then turning with
  its tangent and normal), and a pattern repeats features or whole bodies but never chosen faces.
  A feature pattern or mirror repeats a shell only when nothing else changes the body between the
  shell and it (`repeat_shell.rs` hollows again from the body before the shell, so later features
  would be lost), refusing otherwise.
  Every copy is the original's tool placed again, never recomputed where it lands (a copy of an
  extrusion up to next stops where the original did, not on the face it meets), and one pattern
  cannot chain a shift, a turn and a mirror.
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
- [low · medium] Emboss gives the walls no draft, is repeated by a pattern or mirror of features
  as a rigid copy (a wrapped one is not wrapped again onto the surface the copy lands on), and
  refuses a sphere or other doubly curved face (wrapping onto one needs the chosen projection of
  the wrapped split above). Its depth on a cone is measured square to the slope.
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
- [medium · hard] A spline is only the control points it was drawn with: a point has no tangent or
  curvature handle to set the direction and pull of the curve there, which would be stored as
  constraints on the point rather than as positions so the solver and dimensions keep reading
  them; the degree cannot be chosen; a point cannot be removed while keeping the shape (only
  approximately possible); and a point can be inserted keeping the shape only in an open control
  spline of four points or more, not in a closed one (it needs a closed spline with knots of its
  own), a conic or one of fewer points.
- [medium · hard] A sketch fillet cannot round a spline (the corner would need a centre rolled
  along the spline's offset and the spline cut at the touching
  parameter by blossoming, as trim cuts it). A trimmed fit-point spline is
  refitted through its remaining points rather than kept exactly, and one cannot be split or broken
  at a point (its knots follow its points, so no fit spline holds the piece).
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

- [low · medium] Onshape's folders are not browsed: the documented REST API lists documents only,
  by view (`/documents` with `filter`, `owner` and `parentId`), and a folder tree needs the
  web client's own `globaltreenodes` endpoints, which are not part of the published API.
  Drawings, blobs and Part Studio parts are not offered either, only whole Part Studios and
  assemblies.
- [low · medium] The rendered view's bounced light (`render.md`, the light pass) gathers from a
  reflective shadow map, so it reaches only `BOUNCE_REACH` of the light frame and comes only from
  surfaces the sun sees, and its visibility test still lets bounced light through a wall edge-on to
  both the sun and the cross light (one whose normal points up the sun's slope) and one behind more
  surfaces than `BOUNCE_LAYERS` sun layers and the cross map's one layer hold.
- [low · medium] A user cannot read an OpenEXR picture as the rendered view's surroundings
  (`image`'s `exr` feature is pure Rust but brings `pulp` and `paste`, which `cargo deny`
  refuses), nor give a material texture, bump or clear-coat maps, and the visible floor
  (`render.md`) takes no ambient occlusion from the bodies standing on it.
- [medium · medium · blocked by: a live Fusion 360 and Autodesk account to confirm the open
  behaviour against (`DECISIONS.md`)] Export and open in Fusion 360 (`cloud.md`) opens only the
  uploaded file's web page, through `browser::open` on every platform. What remains:
  - Fusion's desktop URL handler on Windows (`fusion360://`, the target Autodesk's "Open in Fusion"
    links use; its exact parameters, likely the item's lineage URN and hub, are unconfirmed) started
    through a new `caditor_windows` call, never a shell, with the web page as the fallback when the
    handler is missing or refuses and a notice saying so. Linux has no desktop Fusion and keeps the
    web page.
  - Onshape behind the same command: `Provider::send` for Onshape fills `Sent`'s item, version and
    web link (its document and element ids; the documented document URL is its open target), the
    remembered destination is a workspace, and `Service` words the command for it.
  - An `Untitled` model is sent as `Untitled.step` without asking for a name first, as Send the
    model here does; the plan was to ask in the dialog.
  - The Export dialog's button beside Open in OrcaSlicer: `ExportCommand` is handled in `Files`,
    which has no way to start the cloud send, so the dialog needs an outcome the app routes to
    `CloudCommand::OpenInFusion`.
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

