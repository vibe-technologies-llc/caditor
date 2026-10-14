# Loft

{command:model.loft} makes a body that passes through two or more closed profiles drawn on
sketches, one after another: a round duct turning into a square one, a bottle's neck, a transition
piece, a pyramid or cone ending at a point. It is in the Solid group's menu of the ribbon, the Model
menu, the command palette and the right-click menu of selected sketch curves. Where one profile is
carried along a path, use [Sweep](sweep) instead.

Draw each profile in a sketch of its own, the sketches one after another along the way the loft
runs (on parallel planes, or tilted), or use a flat face of a body as a profile. Select a region or
a curve of each sketch, or the flat faces, in the order the loft passes through them, then choose
Loft. A flat face becomes a hidden sketch on that face, named after the loft ("Loft 1 profile 1"),
holding the face's edges projected, so the profile follows the face when the body changes; with
faces of one body selected the loft adds to that body. Selecting a sketch point, datum point, corner or the
centre of a round edge as well makes the loft end at that point. With fewer than two profiles
selected Loft opens choosing them in the view; until it has two (or one and a point) its row in
the tree says "Waiting for profiles" rather than failing.

A sketch with a single closed shape gives that shape. A sketch holding several shapes needs one
chosen: select the region wanted (a region is chosen by clicking inside it while nothing is open).

## Panel

- **Profiles** lists the profiles in the order the loft passes through them. The arrows move one
  earlier or later and the cross leaves it out. Use selected lofts through the selected regions or
  sketches instead; Choose in the view starts choosing: each click on a region or curve of a sketch
  adds that profile at the end, a click on a profile already chosen leaves it out, a click on
  another region of a chosen sketch takes that region instead, a click on a flat face of a body
  adds that face, and Escape or Stop choosing stops.
- **Closing**: **Open** stops at the last profile or end point; **Closed** runs on from the last
  profile back into the first, making a ring with no ends (a torus-like handle, a closed duct). A
  closed loft needs three profiles or more and has no end point or square ends.
- **Ends at** is the last profile, or a point the loft closes to, as a pyramid or cone does. Use
  selected or Choose in the view takes a corner, round edge, sphere or torus, sketch point or datum
  point; the cross ends it at its last profile again.
- **Sides**: **Smooth** runs one smooth skin through every profile; **Ruled** joins each profile
  to the next with straight-lined sides, so its faces break at every profile. Between two profiles
  both are the same unless an end is square.
- **Start** and **End** (Smooth, open lofts): **Free** leaves the end profile however the profiles
  around it lead; **Square** leaves it square to its plane, as a straight wall would, so a loft
  from the flat top of an extruded block carries the block's sides on without a crease. A square
  end shows a **Start weight** or **End weight**: how far the loft keeps square before turning
  towards the next profile, 1 by default, larger keeping square longer; it takes expressions. End
  is not offered when the loft ends at a point.
- **Result**: **New body**, **Add to body**, **Remove from body** or **Intersect with body**, and
  the **Body** it changes. Removing cuts a tapered pocket or a transition into the body.

## How profiles are matched

- Each profile is divided into as many pieces as the profile with the most curves, so a square (4
  lines) lofts to a triangle (3 lines) by halving its longest side. A circle or ellipse is divided
  evenly, starting at the point facing the first corner of its neighbour, so a circle lofted to a
  square runs straight from each quarter to a corner.
- Corresponding pieces are found by turning each profile until its corners lie nearest its
  neighbour's, and profiles drawn facing the other way are turned round, so the sides never twist
  by accident. To twist on purpose, draw the profile turned.
- The faces are named after the first profile's curves ("Loft 1 side from Line 2"), so later
  features keep hold of them when later profiles change.

## Limits

- A profile must be one closed outline without holes; cut holes after the loft.
- Profiles must lie one after another: two in the same place, or one lying along the way the loft
  runs, fail naming them.
- Sides that would run into each other (profiles turned too far against each other) fail with what
  to change.
- Guide rails, a centreline and curvature-continuous ends are not available yet, and a square end
  leaves its profile square to the profile's plane only, not tangent to a neighbouring face of a
  body.
