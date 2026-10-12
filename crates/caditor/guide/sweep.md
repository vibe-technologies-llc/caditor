# Sweep

{command:model.sweep} carries the closed regions of a sketch along a path into a body, or cuts
them into one: a pipe, a handrail, a gasket round a lid, a groove along an edge. It is in the
Solid group of the ribbon, the Model menu and the right-click menu of selected sketch curves.

The profile is a sketch with closed regions; the path is a chain of curves of another sketch
(lines, arcs, splines, ellipses, construction lines too) or of edges of a body, joined end to end
without forks. Select the profile's regions or curves and the path's curves or edges together,
then choose Sweep. With only a profile selected, or nothing (the last sketch with regions not yet
swept), Sweep opens with no path and starts choosing it in the view.

The path is followed from its end nearest the profile's plane, the profile moving as that end
moves, so draw the path starting on the profile. A closed path (a loop) needs no ends: the sweep
starts where the path crosses the profile's plane and closes on itself.

## Panel

- **Sketch** and **Regions** choose the profile, as for [Extrude](extrude).
- **Path** names the curves or edges followed. Use selected follows the selected ones instead;
  Choose in the view starts choosing: each click on a curve or edge adds it to the path, a second
  click leaves it out, and Escape stops. The cross clears the path.
- **Orientation**: **Square to path** turns the profile with the path, keeping its angle to it as a
  bent pipe does; **Parallel** only moves it along the path, keeping it parallel to its sketch.
- **New body**, **Add**, **Remove from body** or **Intersect with body**, as for an extrusion.

## Corners and limits

- Square to path follows tangent curves smoothly. A sharp corner between two straight lines is
  mitred: both sides meet on the plane halving the corner. Any other sharp corner fails, saying to
  round it with a fillet in the path's sketch, or to keep the profile parallel.
- A profile larger than a bend of the path, reaching past the centre the path bends around, fails
  naming the curve; so does a sweep that runs into itself.
- Parallel needs the path to keep rising out of the profile's plane; a path turning back along it
  fails, so a closed path is swept square to it.
- A closed path out of one plane may bring the profile back turned; such a sweep fails, asking for
  a path in one plane.

A curve of the path cannot be deleted while a sweep follows it; deleting the whole path sketch
leaves the sweep failing until a path is chosen again.
