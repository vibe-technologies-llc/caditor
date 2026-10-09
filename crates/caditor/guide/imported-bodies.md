# Imported bodies

A STEP model or a mesh brought in with {command:file.import} becomes one import feature per body;
see [importing](importing). Imported bodies can be filleted, cut and combined like any other.

Open an import in the tree to see where it came from and to place it: **Placed in** the world or a
[coordinate system](coordinate-systems), turns about X, Y and Z, moves along them, and a
**Scale**.

- {command:file.reload_import} reads the same file again, after it changed.
- {command:file.replace_import} puts a body from another file in its place.

Either keeps the placement, and features using the body find its faces again by name.
