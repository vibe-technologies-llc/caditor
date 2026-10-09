# Display styles and hiding

## Display styles

View › Display style chooses how bodies are drawn:

- **Shaded with edges**, the default.
- **Shaded with hidden edges dashed**: edges behind faces show dashed.
- **Shaded without edges**: edges appear only when hovered or selected.
- **Wireframe**: edges only; faces are neither drawn nor picked.
- **Hidden lines removed**: flat white faces with edges over them, like a drawing.
- **X-ray**: see-through faces with edges on top.

The style is kept while caditor runs and also shapes exported images.

## Hiding and showing

- {command:view.hide_selection} hides the features owning the selection, and the eye on a row of
  the [feature tree](feature-tree) hides or shows that feature.
- {command:view.hide_others} hides everything but the selection, and {command:view.show_all}
  brings everything back.
- {command:view.toggle_sketches}, {command:view.toggle_datums} and {command:view.toggle_bodies}
  hide or show every item of a kind at once.
- {command:view.toggle_principal} hides the principal planes, axes and origin.

Hidden items are not drawn, picked or counted when fitting the view. Hiding is part of the model
and can be undone.

## View aids

{command:view.toggle_glyphs} hides the constraint marks of the edited sketch, keeping dimensions.
{command:view.toggle_centres_of_mass} marks the centre of mass of each shown body; [Measure](measure)
reads distances to it.
[Section view](section-view) cuts the bodies at planes to look inside them.
