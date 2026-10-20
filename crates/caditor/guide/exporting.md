# Exporting

## Bodies

{command:file.export} writes the bodies you choose as:

- **STEP**, for other CAD programs, with the colours and opacity of bodies and faces, and threads.
- **STL**, for 3D printing, binary or text.
- **3MF**, for 3D printing, with a thumbnail of the model, showing its
  [reference images](reference-images), and each thread as metadata on its body, which programs
  that do not know it ignore.
- **OBJ** and **glTF** (`.glb`), for other 3D programs.

{command:file.export_again} writes the same bodies again to the file the last export wrote, in the
same format and with the same choices, without asking for a file: handy after each change while
printing. It asks before replacing only when the file was changed by something else since, or for
every configuration, when one of their files was. It is offered until another model is opened.

Each kind of file dialog (opening, importing, exporting models, drawings, parameters) starts in the
folder it was last used with while caditor runs. The export dialogs also keep their format, options
and folder after caditor is closed: the next export of a model, a drawing or an image starts as the
last one ended. [Preferences](preferences) › General › Exports can keep them only while caditor
runs, or forget them.

{command:file.open_in_slicer} sends the bodies straight to OrcaSlicer for printing: it writes them
as 3MF to a folder of caditor's own, without asking for a file, and starts OrcaSlicer with it. With
bodies selected in the view or chosen in the feature tree, only those are sent; with none, the
bodies chosen in the Export dialog. The Export dialog has the same button beside Export…, using
the resolution and bodies chosen there.
caditor finds OrcaSlicer where it is usually installed (on the path, from Flathub, or an AppImage in
Applications, AppImages, Downloads or `/opt`; on Windows in Program Files). When it is elsewhere,
name its program or AppImage in [Preferences](preferences) › General › Exports. The files are kept
for a week.

{command:file.open_in_fusion_app} sends the bodies straight to the Fusion 360 app on this computer:
it writes them as STEP to a folder of caditor's own, without asking for a file, and opens Fusion 360
with it as a new design. Like the OrcaSlicer button it sends the selected bodies, else the ones
chosen in the Export dialog. It needs no sign-in and no cloud features; the Export dialog has the same
button, and the format and resolution chosen there do not apply to it. Fusion 360 has to be
installed, and on Linux something has to be set up to open `fusion360://` links. The files are kept
for a week.

With cloud features on, the Export dialog also has **Upload to Fusion 360**, which sends the bodies
chosen for export as STEP to your Fusion 360 folder and opens the file in your browser (see
[Cloud](cloud)). The format and resolution chosen in the dialog do not apply to it.

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
