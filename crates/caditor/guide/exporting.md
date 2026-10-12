# Exporting

## Bodies

{command:file.export} writes the bodies you choose as:

- **STEP**, for other CAD programs, with the colours and opacity of bodies and faces, and threads.
- **STL**, for 3D printing, binary or text.
- **3MF**, for 3D printing, with a thumbnail of the model and each thread as metadata on its body,
  which programs that do not know it ignore.
- **OBJ** and **glTF** (`.glb`), for other 3D programs.

{command:file.export_again} writes the same bodies again to the file the last export wrote, in the
same format and with the same choices, without asking for a file: handy after each change while
printing. It asks before replacing only when the file was changed by something else since, or for
every configuration, when one of their files was. It is offered until another model is opened.

Each kind of file dialog (opening, importing, exporting models, drawings, parameters) starts in the
folder it was last used with while caditor runs.

A model with modelled [threads](thread) offers **Threads**: **Modelled** (the default) cuts them
into the exported bodies, recomputing them in the background while they are shown as cosmetic;
**As shown** exports them as the view shows them. The command line always exports them modelled.

Meshed formats take a resolution. The export waits for the model to finish recomputing and warns
when a feature failed, since bodies then export as last computed. It runs in the background,
cancellable from the status bar. Closing caditor while a model or image export runs waits for it
and closes when it is done, asking first about changes made while it waited; the dialog offers to
cancel the export and close, or not to close.
A drawing, parameter file or template being written is waited for the same way; it cannot be
cancelled, so the dialog offers to close anyway if the writing takes long.
[Model properties](templates) such as the title and author go into the formats that have room for
them.

## Images

{command:file.export_image} saves the 3D view as a PNG at any size, with the view's background or a
transparent one, in the colours the view shows, light or dark. Hover, selection and labels are
left out.

## Drawings

{command:file.export_sketch} and {command:file.export_face} write sketches and flat faces as DXF or
SVG for laser cutting or drawing programs. Lay them out side by side, or nested on a sheet of a
width you give, with or without dimensions and names. Every dimension is written: those along one
line stack outward as on the canvas, and a label that would overlap another is moved clear of it. {command:file.keep_drawing_construction}
also writes construction geometry, on a dashed layer of its own.

## Parameters

{command:file.export_parameters} writes the [parameters](parameters) to a CSV file.

Models can also be exported without opening a window; see the [command line](command-line).
