# Text

{command:sketch.text} writes text into the sketch in Inter, the font caditor carries.
Its letters are closed outlines, so [Extrude](extrude) raises them from a face or cuts them into
it like any other profile, their holes (the middle of an `o` or a `B`) left open. Find it in the
corner menu of the Polygon button, the Sketch menu or {command:palette}.

While it is active the Text panel opens with the text's values:

- **Text**: the letters. Press Enter for a new line, set below the one before it; a letter Inter
  lacks is left out and the panel names it.
- **Height**: how tall a capital letter stands above the baseline, such as `5 mm` or `size`.
- **Angle**: how far the baseline turns from the sketch's horizontal axis, such as `90 deg` for
  text running upward.
- **Align**: **Left**, **Centre** or **Right**. The text's point is where the first line's
  baseline starts, is centred or ends, and every other line lines up about the same point.
- **Bold** and **Italic**: Inter's bold weight and its own italic letters.

Height and angle take units and expressions using your [parameters](parameters), and stay tied to
them: change the parameter and the letters follow.

## Placing it

- Click where the text is anchored, on its first line's baseline: on a point, the text stands on it; anywhere else
  it gets a new point there. The letters follow the pointer as a preview.
- The panel's button, or Enter in the view, writes it at the one selected point, else at the
  origin.
- From the keyboard, highlight a point with {command:view.highlight_next} and choose it with
  {command:view.activate_highlighted}.

The point the text stands on is an ordinary sketch point: constrain it, dimension its distance
from the origin or from other geometry, or fix it, and the text moves with it. Writing the text is
one change that {command:edit.undo} takes back.

## Changing it

Select the text by clicking one of its letters, then choose {command:sketch.text}: the panel
shows its values, and its button updates the text in one change, keeping its point and every
constraint on it. **Write new text instead** leaves the selected text as it is and writes another.

Deleting the text deletes its point too, unless a curve also uses that point; deleting the point
deletes the text.

## What it draws

Each letter's outline is made of lines and curves from the font, with the overlapping strokes a
font may draw (in Inter's `e`, `t` or `k`) merged into one outline, so every letter is a region of
its own. The text is not made of sketch curves: its letters cannot be trimmed, dimensioned or
constrained, only the point it stands on. Letters of different characters that overlap, which
Inter's spacing avoids, are not merged.
