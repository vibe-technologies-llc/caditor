# Scale

## Scale body

{command:model.scale} scales one body by a factor about a centre. Select the body, then choose the
command; it starts at a factor of 1. The panel sets the **Factor**, a plain number above zero, and
the **Centre**, its three coordinates measured in the world or in a
[coordinate system](coordinate-systems).

**Centre at** fills in those coordinates: **Use selected** (or **Choose in the view**) takes a
selected corner, round edge, sphere, sketch point or datum point, and **Body centre** the middle of
the body's box as it was before scaling.

## Scale model

{command:model.scale_model} scales the whole model at once, by changing its values rather than
adding a feature. Give the factor and the centre, and choose whether to scale only the values
typed as numbers or the parameters too. A note afterwards says what changed, what was left to
follow parameters, which holes lost their standard size and which threads to check. Undo takes it
back in one step.

The principal planes, axes and origin never move. When the centre is off one that a feature uses
(a mirror across the YZ plane, a revolve or circular pattern about the Z axis, a coordinate system
at the origin), the dialog lists those features before you scale, and the same change adds datums
at the top of the tree standing where that geometry lands: **Origin, scaled** and the coordinate
system **Axes and planes, scaled**, or a plane such as **XY plane, scaled** for a sketch that
projects one. The features then use those datums, so the whole model keeps together.
