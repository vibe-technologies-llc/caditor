# Command line

`caditor [MODEL]` starts caditor and opens a model. A `.dxf` drawing or a STEP file is imported
instead, as if dropped on the window.

## Exporting without a window

`caditor --export OUT MODEL` opens no window: it recomputes the model and writes every body that
built to `OUT`, in the format its extension names: `.step` or `.stp`, `.stl`, `.3mf`, `.obj`,
`.glb` or `.png`. MODEL may also be a STEP file.

- `--resolution coarse`, `standard` or `fine` sets the mesh quality of STL and 3MF.
- `--size 1920x1080` sets the size of a PNG, which is drawn like an exported image and needs a
  graphics adapter.

The exit status is 0 when everything exported, 2 when a feature failed (its body is exported as
last computed, with a warning), and 1 with nothing written when the model has no body or cannot be
read.

`caditor --help` lists the options and `caditor --version` prints the version.
