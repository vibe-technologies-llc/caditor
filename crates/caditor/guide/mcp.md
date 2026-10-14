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
  [Measure](measure) does, and the faces of a body with their kind, size and a stable key.
- Read a sketch: its plane, entities, constraints and dimensions, the degrees of freedom left and
  the constraints that conflict or repeat each other, as the [sketch status](sketch-status) names
  them.
- Draw a picture of the model from a standard view, as the command line's `--export` of a `.png`
  does, to look at what it made. This needs a graphics adapter.
- Add a [sketch](sketches) on a principal plane, a flat face, a datum plane or a plane of a
  coordinate system, draw points, lines, polylines, rectangles, circles, arcs, slots, polygons,
  splines and ellipses in it, add [constraints](constraints) and [dimensions](dimensions), and
  later remove them, turn them off or on, or change a dimension's value.
- Add an [extrusion](extrude), which decides whether to add, cut or start a new body as the
  Extrude tool does unless told, and a [hole](hole) at every free point of a sketch.
- [Undo and redo](undo), save and export.

Every change the assistant makes is one step of the model's undo history, named after what it did,
and answers with every feature that fails after it in the words the tree uses. Features, bodies and
sketch entities are named by their ids or names and faces by keys made from what made them, so a
reference still means the same thing after the model recomputes.

## Files

Nothing reaches the disk until the assistant saves or exports. Saving writes the model's own file
the way [Save](saving) does: atomically, keeping the version it replaces in the
[version history](version-history). Changes not yet saved are kept in the recovery journal as the
window keeps them, so a crash loses at most seconds; open the model in caditor to recover them.

Exports go only into the model's folder and the folders below it, unless the server was started
with `--folder DIR`, once for each further folder it may write to. The server reaches no network.

Long work such as recomputing or measuring reports its progress to the assistant, and the
assistant can cancel it.
