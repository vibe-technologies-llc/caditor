# Pipe

{command:model.pipe} runs a ready-made section along a path: tubing, a handrail, a cable run, a
frame of square tube, a groove of a triangular cutter. Unlike a [Sweep](sweep) it needs no
profile sketch, only the path and a few sizes. It is in the Solid group's menu of the ribbon, the
Model menu, the command palette and the right-click menu of selected sketch curves. A spring or a
helical groove winds around an axis instead: that is a [Coil](coil).

The path is a chain of curves of a sketch (lines, arcs, splines, ellipses, construction lines too)
or of edges of a body, joined end to end without forks, open or closed. Select its curves or edges,
then choose Pipe. With nothing selected Pipe opens with no path and starts choosing it in the view;
until it has one, its row in the tree says "Waiting for a path" rather than failing.

## Panel

- **Path** names the curves or edges followed. Use selected follows the selected ones instead;
  Choose in the view starts choosing: each click on a curve or edge adds it to the path, a second
  click leaves it out, and Escape or Stop choosing stops. The cross clears the path.
- **Section**: **Round**, **Square** or **Triangle** (equilateral). A square or triangle lies with a
  side along the plane of the path's sketch (for edges, along the horizontal), the triangle's point
  away from it.
- **Diameter**, **Width** or **Side length** sizes the section.
- **Wall**: **Solid** fills the section; **Hollow** leaves a wall of the **Wall thickness** round an
  empty middle, as a tube. The wall must be thinner than half the diameter or width, or a third of
  the triangle's height.
- **Bend radius** rounds every sharp corner of the path into a bend of that radius, measured to the
  middle of the pipe, as a pipe bender would: between straight lines, and also where a line, arc,
  spline or edge meets another curve. Zero leaves the corners sharp: between two straight lines
  both sides then meet on the plane halving the corner, as mitred tube does.
- **Result**: **New body**, **Add to body**, **Remove from body** or **Intersect with body**, and
  the **Body** it changes. Removing makes a channel or groove of the section. A pipe follows one
  path without gaps, so it is always one piece and has no Separate pieces setting.

Every size takes units and expressions, so a pipe can follow a parameter such as `tube_d`.

While the pipe is open, arrows in the view drag its sizes: one across the section at the start of
the path for the diameter, width or side length, one pointing inward from its outer edge for the
wall thickness (only on a hollow pipe), and one at the middle of the first rounded corner for the
bend radius (only when the path has a corner between two straight lines). Each drag is one
undoable change and follows the same steps and snapping as other arrows; Ctrl drags freely, and
typing a number while holding or hovering an arrow sets the value as the panel field would,
parameters included.

## Corners and limits

- Arcs and splines of the path are followed smoothly. Where the curves at a corner lie in one
  plane, as curves of a sketch do, the bend is a true arc touching both, the pipe there a section
  of a ring; where they turn out of a plane (edges of a curved body), the bend is a smooth curve
  of about that radius.
- With a bend radius of zero, a sharp corner where an arc or spline meets another curve fails,
  saying to give the pipe a bend radius or round the corner with a fillet in the path's sketch.
- A bend radius smaller than the section reaches out from the middle, or one needing more than the
  length of the curves at a corner, fails naming the curves and saying what to change. When two
  corners use up a whole curve between them, the bends meet directly.
- A pipe running into itself fails; so does a closed path out of one plane that would bring the
  section back turned.

A curve of the path cannot be deleted while a pipe follows it. The pipe's faces are named after it
("Pipe 1 outside", "Pipe 1 inside", "Pipe 1 start face"), so later features keep hold of them when
its sizes change.
