---
paths:
  - "crates/caditor-render/**"
---

# Rendering (`caditor-render`)

- Owns the wgpu device and surface, the camera and the viewport. Depends on neither winit nor the
  document: takes any `Arc<dyn WindowTarget>` and draws a `Scene` the app builds (shaded meshes, a
  grid, `Arc<Batch>`es of lines, markers and triangle fills).
- The UI shares the frame's encoder, so nothing in the viewport may invalidate it: allocations run
  inside out-of-memory and validation error scopes (`gpu::scoped`) and a refusal degrades alone.

## Frames

- `begin_frame` reconfigures the surface when the size differs or it is outdated, draws the
  viewport and returns `FrameStart::Ready` with a `Frame` whose encoder the app draws the UI into;
  `submit` presents it.
- `Hidden` (occluded) makes the app stop drawing until the window is shown again or a probe timer
  fires, so a hidden Wayland window does not block the UI thread on the acquire timeout every
  frame. `Skipped` (timeout, outdated or lost surface) is a retry, never a reason to stop.
- A `SurfaceValidation` acquire climbs a ladder, one rung per consecutive failure: reconfigure,
  recreate the surface, configure conservatively, open a replacement device.
- Refused scene targets step multisampling down a level (to Off, the preference untouched) and
  draw again; a refused mesh or batch is dropped alone and not retried until it leaves the scene
  or its anchor moves; refused pick targets disable picking. Each is one `RenderFault`
  (`Renderer::take_faults`), shown as a notice.

## Devices and settings

- `open_device` tries the adapter `AdapterPreference` asks for, power saving (the low-power one) by
  default and unless `WGPU_POWER_PREF` says otherwise, so a
  CAD window does not wake a discrete GPU, then every other adapter that can present, ranked by
  `adapter_rank`. Each adapter gets its own limits, then defaults, then WebGL2-level ones.
- Nothing uses storage buffers, so downlevel and GL devices draw everything. The surface is
  clamped to the largest texture side and is a plain 8-bit format, never a float or snorm one an
  HDR setup lists first. Offered MSAA levels (`gpu::offered_msaa`) need surface and
  `Depth32Float` support with resolve.
- `GraphicsSettings` (vsync, `Msaa`, `Shading`, `AdapterPreference`) is applied live by
  `Renderer::set_graphics`, which changes only what differs; a changed adapter preference opens a
  new device the way device loss does (frames are skipped until it answers, and a failure keeps
  the old device); `graphics_info` reports what is actually in use. MSAA uses the
  offered level closest to the one asked for (`Msaa::closest`); a change rebuilds pipelines and
  scene targets but keeps mesh buffers and picking. The pick pass is always single-sampled.
  Shading is a uniform flag, so switching is free.
- Device loss: `DeviceLoss::watch` wakes the app. The next `begin_frame` opens a new device on a
  worker thread (same surface, else a new one); until it answers frames are `Skipped`. The answer
  is installed on the UI thread, rebuilding the `ViewportRenderer` with the current settings and
  bumping `Renderer::generation`. A `Frame` from an older generation, or drawn while the device
  is lost, is dropped by `submit`; a pick in flight polls `Failed`.

## Precision

- Model positions are converted relative to a nearby point in f64 before the f32 cast and the view
  matrix is rotation only, so geometry far from the origin stays exact.
- A `ShadedMesh` stores positions relative to its own centre, with the eye offset computed in f64
  each frame. Batches store positions relative to an anchor (the eye when uploaded), whose offset
  the view uniform carries; it stays while the eye is within `reanchor_reach`, the larger of `REANCHOR_DISTANCES` view
  distances and the distance at which f32 rounding of an offset stays under `ANCHOR_ERROR_PIXELS`
  of a pixel at the current zoom; beyond that the next frame re-anchors and uploads every batch
  again.

## Meshes

- A `MeshInstance` is an `Arc<ShadedMesh>` plus a `FaceStyle` (colour, pick id) per face. Buffers
  upload once per `Arc` and drop when the mesh leaves the scene; a mesh past `max_buffer_size` is
  split into parts that each fit.
- Per-face styles live in an `Rg32Uint` texture (`StyleLayout`) read by face index in the vertex
  shader, rewritten only when they differ, so hover and selection cost nothing in geometry.
- Faces are lit two-sided and write depth, hiding edges and sketches behind them in view and
  picking alike (everything but `Layer::Front`). A face without a pick id writes id 0 with its
  depth in the pick pass, not discarded. Enhanced shading scales highlight and rim with the face
  colour's luminance so dimmed and tinted bodies stay dark and keep their hue (offscreen test).

## Depth, buffers and layers

- Reverse-Z, infinite far plane, `Depth32Float`, multisampled at the level in use (resolved into
  the surface, never stored). The UI is drawn on the resolved surface after the 3D pass.
- Each batch has a `GpuBatch` slot of `GrowableBuffer`s. A slot uploads only when its `Arc`
  differs or the anchor moved, so an idle frame or a camera move writes no vertices. A batch past
  `max_buffer_size` draws only its first whole primitives (logged once).
- Draw order: every batch's lines, then markers, then fills. Translucent fills sort back to front
  by centroid depth across all batches, front-layer fills last (`FillOrder`).
- Model geometry draws over reference geometry (datum planes, axes) through a per-`Layer` depth
  bias, model fills (sketch regions) over the faces they lie on.
- `Layer::Front` draws over everything whatever its depth, in view and picking alike (the app
  puts the edited sketch there). `layered_depth` halves every depth into the far half of the range
  (an exact scaling) and moves front geometry into the near half, where biases stay wide enough
  for the coarser floats (fills under lines under markers).

- `Scene::translucent_meshes` draw after the opaque meshes and before lines with alpha blending and
  no depth write (`translucent_meshes` pipeline), so edges and what lies behind show through; they
  are not drawn in the pick pass, so they never occlude or take a pick.

## Lines, markers and sizes

- Sizes are logical points: `ViewportFrame::pixels_per_point` goes into the view uniform and
  shaders scale line widths, marker diameters and the grid by it.
- `Stroke::Dashed` carries the distance along the curve at its start, so dashes
  (`DASH_PERIOD_POINTS`) run on across a polyline's segments at any zoom and interface size. The
  pick pass draws dashed lines whole, so a gap still picks its curve.
- A marker or line whose colour has no alpha draws nothing but is still picked, so pickable points
  and edges can stay invisible until hovered or selected.

## Projection

- `camera::Projection` is perspective or orthographic. Orthographic shows at every depth the scale
  perspective shows at its target, so switching keeps the on-screen size. The eye stays
  `distance` in front of the target (relative-to-eye precision unchanged) and the depth range is
  finite, centred on the target; `View::reaching` widens it to the scene's bounds so geometry
  behind the eye draws and the grid fades before the range ends.
- Orthographic depth is linear, so shaders turn a layer's depth bias into a fixed offset
  (`ORTHOGRAPHIC_DEPTH_BIAS`); a factor would push edges far through faces.
- Fitting puts the eight corners of the bounds inside the field of view with a margin, exactly in
  both projections (no bounding sphere), so a wide flat part fills the view; bounds of no size
  keep the distance and recentre. Grid spacing follows the view distance, not the eye's height
  (`grid_minor_spacing` gives the app what the shader draws).
- `View::unproject` gives nothing for a non-finite depth or result, so a NaN read back from a pick
  never becomes a hit; `Camera::orbit`, `pan` and `zoom` ignore non-finite pivots, anchors and
  drags.

## Picking

- Renders a window of `PICK_RADIUS_POINTS` around the cursor (`PickWindow`, physical pixels from
  the scale). ID and depth targets are `R32Uint` (depth as f32 bits, since GL does not always
  render to float targets), read back asynchronously so hover never blocks the UI thread.
- Hits report their distance from the cursor in points (`offset_points`, for the app's pick
  tolerances) and their world position (orbit pivot, pan grab point, zoom anchor).
- `poll_pick` says `Pending`, `Ready` or `Failed` (failed readback, or a pick whose frame was
  dropped before `submit`); the app asks again after a failure.
- Reference-layer pick fills (principal and datum planes) are drawn first in a pass of their own
  and everything else over them: a translucent plane owns a pixel only where no face, line,
  marker or model or front fill covers it, and a face seen through a plane is picked.
  Front-layer geometry is picked through any face, as it is drawn.

## Image export (`image.rs`)

- `Renderer::render_image` draws an `ImageRequest` offscreen, independent of the window, at most
  `MAX_IMAGE_SIDE` a side in tiles of `TILE_SIDE` (or the largest texture side), so no size the
  app offers can pass the texture limit. Each tile writes the view uniform with
  `image::tile_transform`, the same clip-space transform the pick window uses, so line widths and
  grid fades stay those of the whole image; one submit per tile, since uniform writes land at the
  next submit. Memory is not yet bounded: every tile's readback buffer is held (`docs/TODO.md`).
- It reuses the window's `ViewportRenderer` when the surface is `Rgba8Unorm` or `Bgra8Unorm`, else
  a throwaway one in `Rgba8Unorm`. Output is written unconverted, as on screen, so pixels are sRGB.
- Errors are `ImageError` (`Busy` while another image runs, `OutOfMemory`, `Refused`,
  `DeviceLost`). `poll_image` never waits on the GPU; `ImageReadback::into_image` is `Send`, meant
  for a worker, and turns the premultiplied colour of a transparent clear into straight alpha.
- `OffscreenRenderer` opens a device without a surface and draws the same `ImageRequest` into an
  `Rgba8Unorm` target, waiting for the result: the headless `--export` of a PNG (`app.md`) uses it.
- The renderer draws whatever scene it is given; leaving out highlights and the grid is the app's
  choice (`app-files.md`).

## Navigation

- Right-drag orbits (turntable around world Z, stopping at the poles; a rolled view turns level),
  middle-drag or Shift+right-drag pans, wheel and pinch zoom toward the point under the cursor.
  View cube and fit changes animate.
- The input mode (`preferences::InputMode`, chosen in Preferences › Navigation) is caditor, the
  mouse scheme above, or Laptop, for a touchpad: two-finger scroll orbits, Alt and scroll pans (egui
  turns Shift and scroll into horizontal-only scrolling, so Alt keeps both axes), pinch and
  Ctrl+scroll zoom, and Alt-drag orbits and Shift+Alt-drag pans, the Alt press never starting a
  selection. The right and middle buttons keep working in both modes.
