# Bodies

The Bodies section above the features lists the bodies standing at the end of the model, by their
own name or the name of the feature that made them. Click one to choose its feature in the tree;
Ctrl-click and Shift-click choose several, as in the feature tree. The eye hides or shows it.

An extrusion, revolve or sweep set to make **A body each** (its **Separate pieces** row) lists one
body per separate piece, named after the feature and the sketch curve that drew the piece, such as
"Extrude 1 (Line 5)". Each has its own eye, colour, material and name, and is selected, measured,
exported and used by later features on its own; hiding the feature itself hides all of them, and
hiding everything but one piece hides the feature's other pieces too. Deleting a curve of a piece and
drawing it again keeps the piece the same body, with its look and the features using it: the curve
the piece is named after may then be another of its curves.

## Groups

Groups keep a long list of bodies in order, and they can sit inside each other. Choose bodies in
the list or select them in the view, then use {command:model.group_bodies}: they go into a new group
named "Group 1", "Group 2" and so on, ready for you to type its name. Grouping some of the bodies of
a group makes a group inside it; grouping every body of a group, as clicking its row does, puts the
group itself inside the new one.

A group's row shows how many bodies it holds. Click it to choose all of them, the eye hides or shows
them together, double-click the name to rename it, and the arrow folds it away. Its **⋯** menu, also
on a right-click, has:

- **Rename group** ({command:model.rename_body_group}).
- **Ungroup** ({command:model.ungroup_bodies}), which moves what the group held up one level.
- **Move to group**, which puts the whole group inside another one, or **Out of every group**.

A body's right-click menu has {command:model.group_bodies}, **Move to group** (for every chosen body
when the body is among them) and {command:model.move_out_of_body_group}, which takes it up one level.
That command also moves a chosen group up a level. Groups only organise the list: they never change
the model, and every change to them can be undone.

## Colour, material and name

{command:model.body_appearance}, or the paint bucket on a body's row, opens its card:

- Colour swatches or a hex colour, and an **Opacity** for see-through bodies. The **Custom
  colour** swatch, the plus at the end of the row, opens a square for saturation and brightness, a
  strip for hue, a preview of the new colour beside the current one and a hex field that follows
  them. The colour is applied, as one change you can undo, when you let go of the mouse button;
  with the keyboard, the arrow keys move within the square or the strip (Shift for bigger steps)
  and Enter applies. Colours chosen this way are kept, the latest first, as swatches before the
  plus.
- **Finish**, how the surface takes the light: **Plain** shades the body as every body without a
  finish; **Matte**, **Satin**, **Glossy**, **Rubber** and **Powder coat** give it the sheen of
  plastic, paint or rubber, and **Anodized**, **Brushed metal** and **Polished metal** make it
  reflect a studio around it as metal does, tinted by its colour. The line under the list says
  what the chosen finish looks like. High contrast draws every body plainly, lightening a dark
  colour until its edges and highlighted faces stand out on it, and a hovered or selected face
  shows its highlight colour without a finish.
- **Material**, which sets a density and the material's finish (steel and aluminium brushed metal,
  the plastics satin, matte or glossy), or your own **Density** in g/cm³.
- **Body name**.
- With faces of the body selected, a second row of swatches, ending in its own Custom colour, colours just those faces.

The density gives the body's mass in [Measure](measure).

## Renaming and removing

{command:model.rename_body} names the body. {command:model.remove_body} adds a Remove feature that
takes the body away from the model from that point on; deleting or suppressing that feature brings
it back.
