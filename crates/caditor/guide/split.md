# Split body

{command:model.split} cuts a body in two along a plane, a face, a sketch curve or another
body. Select the body, and with it what to split along; with nothing else, it splits along the YZ
plane. A face is carried on past its edges: a flat face splits along its whole plane, and a
cylindrical, conical, spherical or toroidal face along its whole surface, so a shaft's round side
cuts a plate it does not reach through; a freeform face is refused in words. It has no ribbon
button; use the Model menu, {command:palette} or the body's right-click menu.

The part on the side the plane, curve or face faces stays in the body (outside a shaft's side,
inside a bore) and the rest becomes a new body, so both pieces carry on; **Keep the other side**
(or {command:model.reverse_direction}) swaps them. The panel's **Split along** list holds the
principal planes, earlier bodies and sketches of one open curve, and {command:model.split_along_selected} takes the
selection: a plane or face, a sketch curve, or an edge of another body to split along that whole
body.

To divide faces without cutting the body, use [Split face](split-face).
