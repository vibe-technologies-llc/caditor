# Hole

{command:model.hole} drills holes at the free points and circles of a sketch.

- With one flat face selected, it puts a hole on the face, on solid material away from its edges,
  in a hidden sketch. While it is open, drag the square at its centre to move it on the face, or
  an arrow to move it along one direction; the arrow at the bottom of a blind hole drags its
  depth and the one at its rim, between the two moving arrows, its diameter (making the size
  Custom); a counterbore has arrows for its diameter at the mouth and its depth on the wall, a
  countersink for its diameter and, down the bore wall where the cone ends, its angle (drag it
  deeper for a narrower cone), a slot for its length at one tip and its angle at the other, a
  tapped hole with a thread length for how deep the thread goes, and one drilled Up to next or Up to
  face for how far it goes past the face; zoomed out far enough that they would lie on the moving
  arrows they are left out until you zoom in, the panel's fields still setting them. The square stops on a round
  edge's centre or an edge's middle under the pointer, and on a corner; hold Ctrl to drag freely.
  With the pointer on a handle, type a value (or x, y for the square) and press Enter. Its panel's
  **Placed on** row moves it too: **Choose in the view** and click where the hole goes on any flat
  face, or type its **Position X** and **Position Y** on the face. A position typed with
  [parameters](parameters), such as `width / 2`, keeps following them; such a hole is then moved
  from its fields only, and a plain number typed there lets go of the expression.
- **Place by** ties the hole to the body: **From an edge** measures it from a straight edge
  (select the edge first, or choose it in the view) with a **Distance** you can type, and a
  second edge across the first fixes it, so a hole 8 mm from two sides stays there when the body
  changes. **Concentric** centres it on a round edge, a circle or an arc, and it follows that
  edge. Each edge can be let go again from its row. While the hole is open, a hole measured from
  an edge has an arrow at its centre pointing away from the edge, whose drag changes that
  **Distance** (it stops a step short of the edge, and a distance named or driven by a
  [parameter](parameters) keeps its name or says what drives it), one arrow for each edge.
- **Add another hole** under **Placed on** drills more holes of the same size and depth: click
  each spot on the face, or highlight the face with the keyboard to put one where there is most
  room, and press Escape when done. Each hole is listed with its own position or edges and can
  be removed. A hole of several points has a square on each free point: drag it to move just that
  hole, or type its x, y with the pointer on it; a point whose position follows a parameter does
  not drag and says which parameter drives it. **Edit the sketch** under **Sketch** opens the
  hidden sketch holding the points.
- With one curved face selected (the round wall of a cylinder, a cone or a sphere), it drills
  square into the face and asks you to click where the hole goes. Drag the square or an arrow at
  its centre to slide it over the face, staying square to it, or use **Choose in the view** on
  **Placed on** to put it somewhere else.
- With a sketch selected or being edited, it drills at each of its loose points and circles.

A new hole starts from the diameter and blind depth last used on one, kept between sessions
(values naming a parameter the model lacks fall back to 6 mm and 10 mm).

The panel sets:

- **Size**: Custom, or a metric screw size with its **Fit** (close, normal, loose, or tapped with
  the thread named, its form, class, hand and length). A tapped hole's **Thread form** is
  **Modelled** for a new hole, cutting the thread into each bore, or **Cosmetic**, only drawn.
- **Style**: Plain, Counterbore, Countersink or Stepped, with the head sizes.
- **Diameter**, or **Sized by** the sketch's circles.
- **Shape**: Round, or Slot with a length and angle.
- **Depth**: Blind with a depth, Through all, Up to next (the next flat face along each hole's
  axis) or Up to face (a face or plane you select), the last two with an offset past the face; a
  blind round hole can have a 118° drill point.

Choosing a size fills in exact values; typing any size makes the hole Custom again, and the
drills are previewed as you type. {command:model.reverse_direction} drills the other way, like
**Reverse direction**. A tapped hole
shows its thread like a [thread](thread): as the cut when modelled, drawn when cosmetic or while
modelled threads are shown as cosmetic.
