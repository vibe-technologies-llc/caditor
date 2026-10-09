# Thread

{command:model.thread} marks a cylindrical face as threaded: a bolt's shaft or a hole's bore. Select
one round face, then choose Thread. The thread is not modelled as a helix: it is drawn on the face
and its designation goes into STEP, glTF and OBJ exports, as drawings and machinists expect.

It starts as the ISO metric coarse size nearest the face, right-handed and as long as the face. The
panel sets:

- **Face**: the face threaded; **Use selected** (or **Choose in the view**) moves the thread to
  another round face, keeping the size while it fits.
- **Standard**, **Size** and **Class**; the side, internal or external, is read from the face.
- **Hand**: right or left.
- **Length**: the full face, or a depth, starting from either end.

The thread shows in the view as a circle at its start, dashed lines along it and a dashed circle at
its end. For a tapped hole made with the [Hole](hole) tool, set the thread in the hole itself.
