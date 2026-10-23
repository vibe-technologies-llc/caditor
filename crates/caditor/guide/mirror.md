# Mirror body

{command:model.mirror} adds a mirrored copy of one or more bodies, leaving the originals as they
are. Select the bodies (a face, edge or vertex of each), or choose them in the tree, and with them
the plane to mirror across: a principal or datum plane or a flat face. With no plane selected, it
mirrors across the YZ plane. However many bodies are selected, they all go into one mirror, so its
plane and Result apply to every one of them. With nothing selected, it starts on the last body
and lets you click the bodies to mirror in the view.

The panel chooses the plane from a list of the principal, datum and coordinate system planes
before it, or from the selection or the view. **Bodies** lists what the mirror reflects: **Stop
mirroring this body** leaves one out, **Use selected** adds the bodies selected in the view, and
**Choose in the view** lets you click bodies to add them or, clicked again, leave them out, until
Escape. **Result** decides what each mirrored copy becomes:

- **New body**: a body of its own beside its original, listed under Bodies and named after it
  ("Mirror body 1 (image of Bracket)"), which you can move, colour, fillet or combine like any
  other body.
- **Add to body**: joined to its original, making one body, such as the second half of a
  symmetrical part.
- **Replace body**: put in its original's place, so only the mirrored copy remains.

A new body that later features use cannot be turned back into part of its original, and a mirror
whose images later features use keeps making a body of each; the panel says which features use
them. Leaving a body out of the mirror fails the features that used its image, saying which.

## Mirroring features

With extrusions, revolves, holes, embossed outlines or primitives chosen in the tree, Mirror mirrors those features
on the same body instead, always adding to it, such as a hole on one side copied to the other. Fillets, chamfers and
a shell chosen with them round, bevel or hollow the mirrored copy the same way, as in a
[pattern](patterns).

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
