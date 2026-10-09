# Primitives

Primitives are ready-made bodies sized by numbers: {command:model.box}, {command:model.cylinder},
{command:model.sphere}, {command:model.torus}, {command:model.cone}, {command:model.wedge} and
{command:model.prism}. They are in the Model menu under Primitives and in {command:palette}.

## Placing one

With a plane or a flat face selected, the primitive stands there: on a face at its middle, added to
that face's body. With nothing selected it starts at the origin and asks you to click a plane or
face to put it on; Escape leaves it where it is.

## The panel

- **Shape** switches to another primitive.
- **Placed on**, **Position** X and Y and **Starts at** (a corner, the base's centre or the
  centre) place it.
- The sizes; typing one previews it. {command:model.reverse_direction} grows the shape the other
  way from its place, unless it is centred there.
- **Result**: New body, Add, Remove or Intersect, as for [Extrude](extrude). Switching to a cut
  turns it to run into the body.
