# Command line

`caditor [MODEL]` starts caditor and opens a model. A `.dxf` drawing or a STEP file is imported
instead, as if dropped on the window.

## Exporting without a window

`caditor --export OUT MODEL` opens no window: it recomputes the model and writes every body that
built to `OUT`, in the format its extension names: `.step` or `.stp`, `.stl`, `.3mf`, `.obj`,
`.glb` or `.png`. MODEL may also be a STEP file.

- `--resolution coarse`, `standard` or `fine` sets the mesh quality of STL and 3MF.
- `--size 1920x1080` sets the size of a PNG, which is drawn like an exported image, with the
  model's [reference images](reference-images), and needs a graphics adapter.
- `--configuration NAME` exports the model in that [configuration](configurations) instead of
  the active one; the model file is not changed.
- `--all-configurations` writes every configuration to a file of its own beside `OUT`, named as
  Export's Every configuration names them: `--export bracket.stl` writes "bracket M4.stl",
  "bracket M8.stl" and so on, never `OUT` itself.

The exit status is 0 when everything exported, 2 when a feature failed (its body is exported as
last computed, with a warning) or a configuration could not be exported while others were, and 1
with nothing written when the model has no body, has no such configuration or cannot be read.

`caditor --help` lists the options and `caditor --version` prints the version.

## Serving an AI assistant

`caditor mcp MODEL` opens no window either: it serves MODEL to an AI assistant over the Model
Context Protocol until the assistant closes it. [Working with an AI assistant](mcp) says how to
connect one and what it can do; `--folder DIR` lets it export into DIR as well.
