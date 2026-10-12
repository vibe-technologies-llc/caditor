# Thread

{command:model.thread} threads a cylindrical face: a bolt's shaft or a hole's bore. Select one round
face, then choose Thread. Its designation goes into STEP, 3MF, glTF and OBJ exports, as drawings
and machinists expect.

It starts modelled, as the ISO metric coarse size nearest the face, right-handed and as long as the
face. The panel sets:

- **Face**: the face threaded; **Use selected** (or **Choose in the view**) moves the thread to
  another round face, keeping the size while it fits.
- **Form**: **Modelled** cuts the helical groove into the body, so a print or a machined part comes
  out threaded; **Cosmetic** only draws the thread and names it. A modelled metric thread follows
  the ISO basic profile (its minor diameter for both bolt and nut, whatever the class); pipe and
  trapezoidal threads get flat crests and roots. A taper pipe thread is always cosmetic.
- **Standard**, **Size** and **Class**; the side, internal or external, is read from the face.
- **Hand**: right or left.
- **Length**: the full face, or a depth, starting from either end.

A modelled thread shows as the cut itself. A cosmetic one shows in the view as a circle at its start,
dashed lines along it and a dashed circle at its end. For a tapped hole made with the [Hole](hole)
tool, set the thread in the hole itself.

## Faster editing

Cutting a thread takes time: an M4 bolt 20 mm long takes most of a second each time the model
recomputes, so a model with dozens of tapped holes can take tens of seconds.
{command:view.toggle_cosmetic_threads} (View menu) shows every modelled thread as cosmetic while
you edit, and the Thread panel says when it is on. Exports still cut them: the export dialog's
**Threads** choice is **Modelled** by default and recomputes them in the background, or **As
shown** to export them as the view shows them.
