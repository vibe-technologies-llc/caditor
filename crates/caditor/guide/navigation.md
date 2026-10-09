# Moving around the view

## With the mouse

How the mouse turns the view depends on the input mode chosen in [Preferences](preferences) ›
Navigation:

- **caditor**: right-drag orbits, middle-drag pans and the wheel zooms.
- **Laptop**: two fingers orbit, Alt and two fingers pan, pinching zooms.
- **Fusion 360**, **FreeCAD** and **Blender** copy those programs' mouse buttons; right-drag
  still orbits in each.

Zooming goes toward the pointer. Orbit and zoom speeds and the zoom direction are in Preferences
too.

## With the keyboard

- {command:view.orbit_left}, {command:view.orbit_up} and the other arrows orbit;
  {command:view.pan_left} and the other Shift+arrows pan.
- {command:view.zoom_in} and {command:view.zoom_out} zoom.
- {command:view.fit} frames the selection, the rows chosen in the tree, or everything.
- {command:view.isometric}, {command:view.front}, {command:view.top} and the other standard views
  turn the view smoothly to look from that side.
- {command:view.look_at_face} looks straight at the one selected flat face, and inside a sketch
  {command:view.look_at_sketch} faces its plane again.

## The view cube

Click a face, edge or corner of the cube to look from there. It takes keyboard focus like a
button: the arrow keys then step to the neighbouring view.

## Projection and saved views

{command:view.toggle_projection} switches between perspective and orthographic. Choose
{command:view.automatic_projection} to stay in perspective but turn orthographic in a standard
view.

{command:view.save} keeps the current view in the model under a name; {command:view.saved_views}
shows, renames, replaces or deletes them. {command:view.set_home} redefines what the Isometric view
shows for this model.

See also [display styles and hiding](display).
