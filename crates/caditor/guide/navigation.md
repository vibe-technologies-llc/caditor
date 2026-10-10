# Moving around the view

## With the mouse

How the mouse turns the view depends on the input mode chosen in [Preferences](preferences) ›
Navigation:

- **caditor**: right-drag orbits, middle-drag pans and the wheel zooms.
- **Laptop**: two fingers orbit, Alt and two fingers pan, pinching zooms.
- **Fusion 360**, **FreeCAD** and **Blender** copy those programs' mouse buttons; right-drag
  still orbits in each.

A right-click that does not move opens the view's [context menu](selection) instead of turning
the view. The bottom right corner of the view shows the buttons of the mode you chose. Zooming goes toward
the pointer. Orbit and zoom speeds and the zoom direction are in Preferences too; the zoom
direction also turns round Blender's Ctrl+middle-drag zoom.

Double-click the middle button to fit the view, as {command:view.fit} does. This works in every
input mode, since none of them uses the middle button's double-click for anything else.

## With the keyboard

- {command:view.orbit_left}, {command:view.orbit_up} and the other arrows orbit;
  {command:view.pan_left} and the other Shift+arrows pan. Holding a key glides the view on
  smoothly, and the orbit speed in Preferences sets how far each step turns.
- {command:view.zoom_in} and {command:view.zoom_out} zoom.
- {command:view.fit} frames the selection, the rows chosen in the tree, or everything.
- {command:view.isometric}, {command:view.front}, {command:view.top} and the other standard views
  turn the view smoothly to look from that side.
- {command:view.look_at_face} looks straight at what is selected: a flat face, a principal or
  datum plane or a sketch from its front, a round face along its axis and a straight edge or axis
  along it. Inside a sketch {command:view.look_at_sketch} faces its plane again. Pressed while the
  view already looks that way, either turns the view a quarter turn.

A key pressed while the view is still turning carries on from where the view is going, so
pressing {command:view.front} and then {command:view.fit} ends looking from the front.

## Going back

{command:view.previous} goes back to the view before the last change: a standard or saved view,
a fit, looking at a face or sketch, opening a sketch, or an orbit, pan or zoom with the mouse or
keys. Press it again to keep going back; caditor remembers the last 32 views while the model is
open. Finishing a sketch leaves the view facing it, so one press of {command:view.previous}
afterwards brings back the view you had before you opened the sketch.

## The view cube

Click a face, edge or corner of the cube to look from there; hovering one shows its name and, for
the standard views, its keys. Drag the cube to orbit the view about its centre. It takes keyboard
focus like a button: the arrow keys then step to the neighbouring view.

Under the cube, Fit all frames everything (Fit selection when something is selected), and the
house button beside it goes to the Isometric view.

## Projection and saved views

{command:view.toggle_projection} switches between perspective and orthographic. Choose
{command:view.automatic_projection} to stay in perspective but turn orthographic in a standard
view.

{command:view.save} keeps the current view in the model under a name; {command:view.saved_views}
shows, renames, replaces or deletes them. {command:view.set_home} redefines what the Isometric view
shows for this model.

See also [display styles and hiding](display).
