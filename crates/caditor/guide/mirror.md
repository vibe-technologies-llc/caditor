# Mirror body

{command:model.mirror} adds a mirrored copy of a body. Select the body (a face, edge or vertex of
it), and with it the plane to mirror across: a principal or datum plane or a flat face. With no
plane selected, it mirrors across the YZ plane.

The panel chooses the plane from a list or from the selection, and **Keep the original** decides
whether the original stays.

## Mirroring features

With extrusions, revolves, holes or primitives chosen in the tree, Mirror mirrors those features
on the same body instead, such as a hole on one side copied to the other.

## Mirroring faces

A body with no features to choose, such as an imported one, can still have a pocket or boss
copied. Select every face around it, such as a pocket's walls and floor or a boss's sides and top,
and use {command:model.mirror_faces}. The faces are closed across their opening, flat or, for
a pocket cut into a round face or a boss standing on one, along that face's curve, and the
region they bound is cut from the body on the other side of the plane where they bound a cavity,
or joined to it where they bound material. A plane or datum plane selected with the faces is the
plane to mirror across; otherwise it is the YZ plane.

The mirror follows the faces: when an earlier change moves or resizes them, the copy changes too.
The faces must leave openings that each lie in one plane or on one cylinder, cone, sphere or torus
face beside them, without running all the way around it; otherwise the mirror fails and says to
choose every face around the pocket or boss.

In the panel, **Stop mirroring this face** leaves a face out, and **Mirror the selected faces**
({command:model.mirror_selected_faces}) mirrors the faces selected in the view instead of what
the mirror held.

To mirror sketch geometry, use [Mirror in a sketch](sketch-mirror).
