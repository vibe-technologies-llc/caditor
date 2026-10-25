# Display styles and hiding

## Display styles

The display picker at the top of the 3D view, which names the style in use, or View › Display
style chooses how bodies are drawn:

- **Shaded with edges**, the default.
- **Shaded with hidden edges dashed**: edges behind faces show dashed.
- **Shaded without edges**: edges appear only when hovered or selected.
- **Wireframe**: edges only; faces are neither drawn nor picked.
- **Hidden lines removed**: flat white faces with edges over them, like a drawing.
- **X-ray**: see-through faces with edges on top. Choosing it again turns it off and returns to the
  style it replaced.
- **Rendered**: bodies lit by a sun with shadows and bounced light, showing their colour, finish
  and material; see [Rendered view](rendered-view).

The style is kept while caditor runs and also shapes exported images.

## Hiding and showing

- {command:view.hide_selection} hides the features owning the selection and the rows chosen in
  the [feature tree](feature-tree), a modifying feature's row standing for its body. The eye on a
  row hides or shows that feature, and {command:view.toggle_visibility} every chosen row.
- {command:view.hide_others} hides everything but the selection and the chosen rows, and
  {command:view.show_all} brings everything back.
- {command:view.toggle_sketches}, {command:view.toggle_datums} and {command:view.toggle_bodies}
  hide or show every item of a kind at once.
- {command:view.toggle_principal} hides the principal planes, axes and origin.

Hidden items are not drawn, picked or counted when fitting the view. Hiding is part of the model
and can be undone.

## Principal planes only when needed

While {command:view.principal_when_needed} is on, as it is at first (Preferences › Appearance › In
the 3D view), the principal planes, axes and origin show while no body is shown, and once one is
they fade aside. They fade back while something needs them, and only the kind it needs:

- the planes while you pick the plane of a new sketch, a mirror plane, a split, a draft's neutral
  plane, a shape's or picture's place, or intersect in a sketch;
- the axes while you pick a revolve's, pattern's, move's or coil's axis, and the axes and origin
  while you project in a sketch;
- the origin while you pick a point, such as a pattern's base or a scale's centre;
- everything while you measure, edit a datum or coordinate system, or pick a datum's references;
- the planes or axes listed while a list of planes or axes in a panel is open.

One you select or point at in the [feature tree](feature-tree) shows while it is.

Each one also remembers what you chose, saved with the model and undone like any other change:

- **Show** (an eye in the feature tree, {command:view.toggle_principal} or {command:view.show_all})
  keeps it shown, bodies or not.
- **Hide** (an eye, {command:view.toggle_principal} or {command:view.hide_selection}) keeps it
  hidden. A hidden one comes back only while a tool asks you to click one of its kind, never just
  because a tool uses it.
- {command:view.principal_only_when_needed}, also in the right-click menu of their rows, returns
  them all to showing only when needed.

Turn the setting off and the ones you have not hidden always show.

They fade in and out rather than popping, and a plane you look at edge-on, as two of them are in
the Top, Front and Right views, fades away so it covers neither the axes nor clicks meant for
them; one you have selected stays.

## View aids

{command:view.toggle_glyphs} hides the constraint marks of the edited sketch, keeping dimensions.
{command:view.toggle_centres_of_mass} marks the centre of mass of each shown body; [Measure](measure)
reads distances to it.

## Dimensions on the model

{command:view.toggle_dimensions} shows sizes on the model without opening a sketch or a panel:

- The dimensions of every shown sketch, laid out on its plane.
- For the feature under the pointer, the selected one and the rows chosen in the
  [feature tree](feature-tree): its own values and the dimensions of the sketch it sweeps, even
  while that sketch is hidden. An extrusion's distance runs along its reach, a revolve's angle as
  an arc about its axis, a hole's diameter across its rim and its depth down its axis, an offset
  face's distance out from the face, and a fillet's radius, a chamfer's distances or angle and a
  shell's thickness point at the face they made. A primitive's sizes, a move's offsets, a thread's
  depth and a pattern's counts and spacings sit beside the feature.

Labels never pile up: where the labels of several sketches and features would cover one another,
the ones of the feature you point at or choose stay and the others give way until you zoom in.

Double-click a label, or highlight it with {command:view.highlight_next} and press Enter, to
change it in place. The field takes units, expressions and parameter names as the feature's panel
does, and the change is one step to undo. A value held by a parameter reads `name = value`: type
`name = 30 mm` to change the parameter, or a plain value to stop using it.
[Section view](section-view) cuts the bodies at planes to look inside them.
