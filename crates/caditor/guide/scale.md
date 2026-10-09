# Scale

## Scale body

{command:model.scale} scales one body by a factor about a centre. Select the body, then choose the
command; it starts at a factor of 1. The panel sets the **Factor**, a plain number above zero, and
the **Centre**, its three coordinates measured in the world or in a
[coordinate system](coordinate-systems).

## Scale model

{command:model.scale_model} scales the whole model at once, by changing its values rather than
adding a feature. Give the factor and the centre, and choose whether to scale only the values
typed as numbers or the parameters too. A note afterwards says what changed, what was left to
follow parameters, which holes lost their standard size and which threads to check. Undo takes it
back in one step.
