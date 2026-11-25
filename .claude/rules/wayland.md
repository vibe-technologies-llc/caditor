---
paths:
  - "crates/caditor-wayland/**"
  - "crates/caditor/src/file_drops.rs"
---

# Wayland

winit 0.30 reports no `HoveredFile` or `DroppedFile` on Wayland and egui-winit pins it, so the
Wayland protocol work winit leaves out (dropped files, the window's export for portal dialogs) is
done on winit's own connection by `caditor-wayland`.

## `caditor-wayland`, the Wayland boundary

- The third crate allowed `unsafe` (with `caditor-zstd` and `caditor-windows`):
  `unsafe_code = "deny"` with `#[expect(unsafe_code, reason = "…")]` per item and the clippy lints
  listed in its manifest (`conventions_tests.rs` checks both). Root is `#![cfg(unix)]`; it offers
  only safe functions.
- `WindowConnection::of_window` (`connection.rs`, the only `unsafe`) takes winit's `Window` as an
  `Arc`, never any `HasWindowHandle`, since the proof below is about winit's type, and returns
  `Ok(None)` unless both handles are Wayland's. It wraps winit's `wl_display` in a guest
  `wayland-client` `Connection` (`Backend::from_foreign_display`, the system backend,
  `dependencies.md`) and the `wl_surface` as a proxy, and holds the window `Arc` as long as they
  live. `DropTarget` and `ToplevelExport` start from it, each with its own event queue and globals.
- Why that is sound (winit 0.30.13, `platform_impl/linux/wayland/window`): the Wayland `Window`
  owns its `WindowState`, which holds a clone of the event loop's `Connection`, and the system
  backend calls `wl_display_disconnect` only when its last clone drops; the `wl_surface` is
  smithay-client-toolkit's, destroyed only when the `Window`'s toolkit window drops. So while the
  `Arc` lives, the display stays connected and the surface stays alive, whatever happens to winit's
  event loop. Each owner declares its `WindowConnection` last and the connection before the `Arc`,
  so its event queue, proxies and guest connection are dropped (their objects destroyed on the
  still connected display) before the window can go. A winit upgrade rereads those files.
- Queues are never read on a thread of their own: winit's loop reads the socket and sorts events
  into every queue; the app dispatches them from `about_to_wait` (`DropTarget::events`,
  `ToplevelExport::handle`) and flushes what they asked.

## The window as the portal dialogs' parent

- `ToplevelExport` (`export.rs`) binds `zxdg_exporter_v2` (`wayland-protocols`' `unstable` feature,
  `dependencies.md`) and exports the window's toplevel. `handle()` returns `wayland:<handle>`, the
  form the portal's `parent_window` takes, once the compositor's handle event has arrived (`None`
  before). No protocol: `AttachError::NoExporter`.
- The export lives as long as the window, since destroying `zxdg_exported_v2` invalidates the
  handle; it is destroyed with the `Session`.
- `window_export.rs` polls it from `about_to_wait` and then hands it to
  `portal::own_dialogs(ParentWindow::Wayland(..))`; until then, and when the export is missing or
  fails (logged), dialogs get the empty parent. On X11 and Windows `WindowExport` does nothing.

## Files dragged onto the window

- `DropTarget` binds `wl_data_device_manager` (up to version 3) and a data device per seat, later
  seats included. Offers named by a `selection` (the clipboard) are destroyed at once;
  `smithay-clipboard` keeps its own device.
- `Tracker` (`tracker.rs`, pure, unit-tested over plain offer values) turns `enter`, `leave`,
  `drop` and finished reads into `Step`s that `DropTarget` performs. An `enter` on another surface
  or with no offer is discarded; on ours an offer without `text/uri-list` is refused, otherwise it
  is accepted with copy as the only action (move would let the file manager delete the files) and
  the list is asked for at once, as XDND does on X11, so the card can name the files. `motion` is
  ignored: the card sits at the view's centre on every platform.
- Each list is read through a pipe on a `dragged-files` thread (`reading.rs`), within `PATIENCE`
  and `MOST_BYTES`, which wakes the app; a late answer for a departed offer carries an old
  `Ticket` and is ignored. `uri_list::file_paths` keeps the `file:` URIs on this machine (no host,
  `localhost` or this host's node name), percent-decoded to bytes so any file name survives, and
  skips comments and other schemes. A list naming no file refuses the offer.
- A drop uses the list read while hovering, waits if it is still being read, and rereads if the
  read failed. A drop with paths finishes the offer, anything else discards it.
- `file_drops.rs` turns `DropEvent`s into the winit events X11 gives (`HoveredFile` per path,
  `HoveredFileCancelled`, `DroppedFile` per path), handled through `handle_window_event`, so
  egui's `hovered_files`, the card and `Files::dropped` behave as on X11. A failed dispatch logs,
  clears any card and gives up on drops for the session. On X11 and Windows `FileDrops` does
  nothing.
- By hand: run caditor on a Wayland session and drag files from a file manager; the outline and
  card appear once the list is read, leaving clears them, a drop opens or imports as on X11.
  `WAYLAND_DEBUG=client` shows the `wl_data_device` traffic, and for the export
  `zxdg_exporter_v2#N.export_toplevel` then `zxdg_exported_v2#M.handle("...")`.
