---
paths:
  - "crates/caditor-wayland/**"
  - "crates/caditor/src/file_drops.rs"
---

# Wayland

winit 0.30 reports no `HoveredFile` or `DroppedFile` on Wayland and egui-winit pins it, so the
Wayland protocol work winit leaves out is done on winit's own connection by `caditor-wayland`.

## `caditor-wayland`, the Wayland boundary

- The third crate allowed `unsafe` (with `caditor-zstd` and `caditor-windows`):
  `unsafe_code = "deny"` with `#[allow(unsafe_code)]` on each item, and the workspace clippy lints
  listed in its manifest (`conventions_tests.rs` checks both). Its root is `#![cfg(unix)]`, so it is
  empty on Windows. It offers only safe functions.
- `WindowConnection::of_window` (`connection.rs`, the only `unsafe`) takes the window as an `Arc`
  of anything with window and display handles and gives `None` unless both are Wayland's. It wraps
  winit's `wl_display` in a guest `wayland-client` `Connection` (`Backend::from_foreign_display`,
  the system backend, `dependencies.md`) and the window's `wl_surface` as a proxy on it, and holds
  the window `Arc` for as long as they live, which is what makes the two calls sound. Anything
  else that needs winit's connection (the `xdg-foreign` export the portal dialogs want, `TODO.md`)
  starts from it with its own event queue and globals.
- A guest connection's objects and queue are destroyed when it is dropped, which needs winit's
  display still connected: the owner is dropped with the `Session` in `exiting`, before winit's
  event loop.
- Its queue is never read on a thread of its own: winit's loop reads the socket, which sorts the
  events into every queue, and calls `about_to_wait` after each wake, where the app dispatches the
  pending events (`DropTarget::events`) and flushes what they asked.

## Files dragged onto the window

- `DropTarget` binds `wl_data_device_manager` (up to version 3) and a data device for every seat,
  seats added or removed later included. Offers named by a `selection` (the clipboard) are
  destroyed at once; `smithay-clipboard` keeps its own device for the clipboard.
- `Tracker` (`tracker.rs`, pure and unit-tested over plain offer values) turns `enter`, `leave`,
  `drop` and finished reads into `Step`s that `DropTarget` performs. An `enter` on another surface
  or with no offer is discarded; on ours an offer without `text/uri-list` is refused, otherwise it
  is accepted with copy as the only action (move would let the file manager delete the files) and
  the list is asked for at once, as XDND does on X11, so the card can name the files. `motion` is
  ignored, since the card sits at the view's centre on every platform.
- Each list is read through a pipe on a `dragged-files` thread (`reading.rs`), within `PATIENCE`
  and at most `MOST_BYTES`, which wakes the app when done; a late answer for an offer that has left
  carries an old `Ticket` and is ignored. `uri_list::file_paths` keeps the `file:` URIs on this
  machine (no host, `localhost` or this host's node name), percent-decoded to bytes so any file
  name survives, and skips comments and other schemes. A list naming no file refuses the offer.
- A drop uses the list read while hovering; if it is still being read the drop waits for it, and if
  that read failed it is read again. A drop with paths finishes the offer, anything else discards
  it.
- `file_drops.rs` turns `DropEvent`s into the winit events X11 gives (`HoveredFile` per path,
  `HoveredFileCancelled`, `DroppedFile` per path) and the app handles them through
  `handle_window_event`, so egui's `hovered_files`, the card and `Files::dropped` behave exactly
  as on X11. A failed dispatch logs, clears any card and gives up on drops for the session; on
  X11 and Windows `FileDrops` has nothing to do.
- Checking by hand: run caditor on a Wayland session and drag files from a file manager; the
  outline and card appear once the list is read, leaving the window clears them, and a drop opens
  or imports as on X11. `WAYLAND_DEBUG=client` shows the `wl_data_device` traffic.
