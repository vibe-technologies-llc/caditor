# Loft

{command:model.loft} makes a body that passes through two or more closed profiles drawn on
sketches, one after another: a round duct turning into a square one, a bottle's neck, a transition
piece, a pyramid or cone ending at a point. It is in the Solid group's menu of the ribbon, the Model
menu, the command palette and the right-click menu of selected sketch curves. Where one profile is
carried along a path, use [Sweep](sweep) instead.

Draw each profile in a sketch of its own, the sketches one after another along the way the loft
runs (on parallel planes, or tilted). Select a region or a curve of each sketch in the order the
loft passes through them, then choose Loft. Selecting a sketch point, datum point, corner or the
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
  another region of a chosen sketch takes that region instead, and Escape or Stop choosing stops.
- **Ends at** is the last profile, or a point the loft closes to, as a pyramid or cone does. Use
  selected or Choose in the view takes a corner, round edge, sphere or torus, sketch point or datum
  point; the cross ends it at its last profile again.
- **Sides**: **Smooth** runs one smooth skin through every profile; **Ruled** joins each profile
  to the next with straight-lined sides, so its faces break at every profile. Between two profiles
  both are the same.
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
- Guide rails, a centreline and tangent conditions at the ends are not available yet.
