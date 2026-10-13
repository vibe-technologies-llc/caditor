# Pipe

{command:model.pipe} runs a ready-made section along a path: tubing, a handrail, a cable run, a
frame of square tube, a groove of a triangular cutter. Unlike a [Sweep](sweep) it needs no
profile sketch, only the path and a few sizes. It is in the Solid group's menu of the ribbon, the
Model menu, the command palette and the right-click menu of selected sketch curves.

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
- **Bend radius** rounds every sharp corner between two straight lines of the path into a bend of
  that radius, measured to the middle of the pipe, as a pipe bender would. Zero leaves the corners
  sharp: both sides then meet on the plane halving the corner, as mitred tube does.
- **Result**: **New body**, **Add to body**, **Remove from body** or **Intersect with body**, and
  the **Body** it changes. Removing makes a channel or groove of the section.

Every size takes units and expressions, so a pipe can follow a parameter such as `tube_d`.

## Corners and limits

- Arcs and splines of the path are followed smoothly. Only corners between two straight lines are
  bent round; a sharp corner where an arc or spline meets another curve fails, saying to round it
  with a fillet in the path's sketch.
- A bend radius smaller than the section reaches out from the middle, or one needing more than the
  length of the lines at a corner, fails naming the lines and saying what to change. When two
  corners use up a whole line between them, the bends meet directly.
- A pipe running into itself fails; so does a closed path out of one plane that would bring the
  section back turned.

A curve of the path cannot be deleted while a pipe follows it. The pipe's faces are named after it
("Pipe 1 outside", "Pipe 1 inside", "Pipe 1 start face"), so later features keep hold of them when
its sizes change.
