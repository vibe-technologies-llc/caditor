# Working with an AI assistant

An AI assistant such as Claude can read and build a model through the Model Context Protocol
(MCP). `caditor mcp MODEL` starts caditor without a window, serving that one model to the
assistant that started it over standard input and output. The model's file need not exist yet: a
new name starts an empty model, written by the assistant's first save.

## Connecting an assistant

Add caditor to the assistant's MCP servers as a command, with the model as its argument. For Claude
Code that is `claude mcp add caditor -- caditor mcp /home/me/parts/bracket.caditor`; other
clients take the same command and argument in their settings. Each server works on one model, so
start one per model.

caditor refuses to serve a model that is open in a caditor window, or one whose recovery journal
holds unsaved changes from an earlier session: open it in caditor to restore or discard them
first, so two programs never change one model at once.

## What the assistant can do

- Read the [feature tree](feature-tree) with each feature's state and, for one that failed, the
  same reason and remedy the tree shows.
- Read the [parameters](parameters) and set one, or add one, with any
  [expression](expressions).
- Read each body's volume, surface area, centre of mass and bounding box, worked out exactly as
  [Measure](measure) does, the faces of a body with their kind, size and a stable key, and its
  edges with their kind, length, radius and the faces they lie between.
- Measure the distance between two points, faces or edges, and the angle between them when they
  have one.
- Read a sketch: its plane, entities, constraints and dimensions, the degrees of freedom left and
  the constraints that conflict or repeat each other, as the [sketch status](sketch-status) names
  them.
- Draw a picture of the model from a standard view, as the command line's `--export` of a `.png`
  does, to look at what it made. This needs a graphics adapter.
- Add a [sketch](sketches) on a principal plane, a flat face, a datum plane or a plane of a
  coordinate system, draw points, lines, polylines, rectangles, circles, [arcs](arcs) (tangent
  arcs too), slots, [polygons](polygon) (on a side as well), [splines](spline),
  [conics](conic) (their rho may be a parameter), ellipses and elliptical arcs in it, a tangent
  arc leaving a curve drawn in the same call as well, add [constraints](constraints) and
  [dimensions](dimensions), and later remove them, turn them off or on, or change a dimension's
  value.
- Change a sketch's geometry with the sketch tools: [trim and extend](trim-and-extend),
  [offset](offset), [mirror](sketch-mirror), repeat in rows or around a point as the
  [sketch patterns](sketch-patterns) do, round or cut corners with
  [Sketch fillet](sketch-fillet), and move, rotate or scale entities, which the sketch's
  constraints hold as a drag does. It can also [project](project-and-intersect) edges, faces and
  corners of a body, curves of other sketches, axes, datums and the origin into a sketch,
  following their source as it changes, and intersect a body, a face or a plane with the sketch
  plane. The same tool switches curves to construction geometry and back, splits a curve at a
  point, breaks curves at their crossings (see [splitting and breaking](sketch-editing)),
  draws a [tangent circle](tangent-circle), a [blend curve](blend-curve) and a [spur gear](gear),
  gear pair or sprocket, and copies geometry with its constraints within a sketch or from
  another, as [copy and paste](move-and-copy) do.
- Add an [extrusion](extrude) or a [revolve](revolve), which decide whether to add, cut or start
  a new body as their tools do unless told. An extrusion can run up to the next face, a plane or a
  curved face, go both ways from the sketch with each side ending in a distance, through all or
  up to a face, start off its plane, run along an axis instead of square, taper, use only the
  regions around chosen points and make a thin wall (or take it off again); a revolve can turn
  both ways or up to a plane, use chosen regions and make a thin wall.
- Drill a [hole](hole) at every free point of a sketch, plain, counterbored, countersunk or stepped,
  round or a slot, flat or with a drill point, sized by a diameter, by the circles drawn or by a
  metric screw size with a clearance, tapped or insert fit, its tapped thread modelled or
  cosmetic.
- [Sweep](sweep) a sketch along a path, twisted or scaled along it, and run a [pipe](pipe) along
  one (sketch curves, body edges or a [helix](helix)), [loft](loft) through sketches, closed back to
  the first, with square ends or ending in a point, through whole sketches or single regions, wind a [coil](coil), add a [rib](rib) and
  [thread](thread) a round face.
- [Split faces](split-face) (carrying the curves square, along an axis or wrapped), [delete faces](delete-face), [split](split) a body along a plane, a
  sketch or another body, and [scale](scale) a body.
- Round or bevel edges with a [fillet or chamfer](fillet-and-chamfer), hollow a body with a
  [shell](shell) (some walls thicker), move faces with [Offset face](offset-face), and add a box, cylinder, sphere,
  torus, cone, wedge or prism as the [primitives](primitives) do, on a plane, a flat face or a
  datum plane.
- [Mirror](mirror) a body, holes, cuts and bosses of one body or chosen faces, and
  [pattern](patterns) a body or those features in a
  row, a grid, around an axis, along the curves of a sketch or at its points, leaving chosen
  copies out; [combine](combine) bodies and [move or copy](move-and-copy) them, or
  [mate](mate) a body onto another, a plane, an axis or a point.
- Add [datum](datums) planes, axes and points in each of the ways their tools offer (through
  points, midway, touching a curved face, where lines cross and so on), a
  [coordinate system](coordinate-systems) and a [helix](helix).
- Change any of those features afterwards, as its card in the app does: the assistant gives only
  what changes, such as a fillet's radius or a pattern's count, and the rest stays as it was.
- Rename, suppress, reorder and delete features in the [feature tree](feature-tree). Deleting a
  feature others use needs the assistant to say whether they go too or stay, failing.
- [Import](importing) a STEP model, an STL, OBJ or 3MF mesh, or a DXF or SVG drawing into a new
  sketch on a plane or into an existing one, in a unit and scale, recentred and without chosen
  layers.
- [Undo and redo](undo), save, save under another name and export.

Every change the assistant makes is one step of the model's undo history, named after what it did,
and answers with every feature that fails after it in the words the tree uses. Features, bodies and
sketch entities are named by their ids or names and faces and edges by keys made from what made
them, so a reference still means the same thing after the model recomputes.

## Files

Nothing reaches the disk until the assistant saves or exports. Saving writes the model's own file
the way [Save](saving) does: atomically, keeping the version it replaces in the
[version history](version-history). Changes not yet saved are kept in the recovery journal as the
window keeps them, so a crash loses at most seconds; open the model in caditor to recover them.

Saving under another name writes a new `.caditor` file, never one that already exists, and the
assistant goes on working on that file from then on; the original stays as it was last saved.

Imports, exports and saving under another name reach only the model's folder and the folders
below it, unless the server was started with `--folder DIR`, once for each further folder it may
read and write. The server reaches no network.

Long work such as recomputing or measuring reports its progress to the assistant, and the
assistant can cancel it.
