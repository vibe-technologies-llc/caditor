---
paths:
  - "crates/caditor-render/**"
---

# Rendering (`caditor-render`)

- Owns the wgpu device and surface, the camera and the viewport. Depends on neither winit nor the
  document: takes any `Arc<dyn WindowTarget>` and draws a `Scene` (shaded meshes, lines, markers,
  triangle fills, grid) built by the app.

## Frames

- `begin_frame` takes the window's current size (reconfiguring the surface when it differs or was
  outdated), draws the 3D viewport into its rect and returns `FrameStart::Ready` with a `Frame`
  whose encoder the app draws the UI into; `submit` presents it.
- An acquire that timed out or was occluded is `Hidden`; an outdated or lost surface `Skipped`.
  The app then stops drawing (UI and workers keep running) until the window is shown again
  (`Occluded(false)`, resize, focus, cursor entering) or five seconds pass, so a window hidden on
  Wayland does not block the UI thread for the acquire timeout every frame.
- Skipped or failed frames retry after 16 ms, doubling up to a second.

## Devices (`gpu.rs`)

- `open_device` asks for the low-power adapter first (unless `WGPU_POWER_PREF` says otherwise), so
  a discrete GPU is not woken for a CAD window; then every other adapter that can present to the
  window: integrated before discrete, virtual and software, Vulkan before GL.
- Each adapter is asked for a device with its own limits (and adapter-specific format features),
  then default, then WebGL2-level limits that keep its texture and buffer sizes, before the next.
- The surface is clamped to the device's largest texture side; the multisample levels offered
  (`gpu::offered_msaa`: those both the surface format and `Depth32Float` support, with resolve)
  are read from the adapter's format features only when the device may use them.
- Nothing needs storage buffers, so downlevel and GL devices draw everything (a test renders on
  WebGL2 limits).

## Graphics settings (`settings.rs`)

- `GraphicsSettings` (vsync, `Msaa` level, `Shading`) is given to `Renderer::new` and applied live
  by `Renderer::set_graphics`, which changes only what differs.
- Vsync picks the present mode from the surface's capabilities (`settings::present_mode`): `Fifo`
  when on; when off, `Mailbox`, else `Immediate`, else `Fifo`. A change sets `needs_reconfigure`,
  so the next `begin_frame` reconfigures through `resize`, as for an outdated surface; a recreated
  surface reads its present modes again.
- MSAA uses the offered level closest to the one asked for (`Msaa::closest`: fewest doublings
  away, ties towards more samples, Off always offered). A change rebuilds the viewport's pipelines
  and drops its scene targets (`ViewportRenderer::set_sample_count`), keeping mesh buffers and
  picking; the pick pass is always single-sampled.
- Shading is a flag in the view uniform (`light.w`), so switching costs nothing.
- `Renderer::graphics_info` is a `GraphicsInfo`: adapter name, backend, driver, the levels
  offered, and the level and vsync actually in use, which the app shows in Preferences.

## Device loss

- `DeviceLoss::watch` registers the device-lost callback (which also wakes the app through the
  `Wake` given to `Renderer::new`) and the uncaptured-error handler, which logs.
- The next `begin_frame` after a loss opens a new device on the same surface (or a new one when
  that fails), reconfigures it and rebuilds the `ViewportRenderer` (pipelines, mesh buffers, pick
  targets, growable buffers), bumping `Renderer::generation`; a pick in flight polls `Failed`.
  The current `GraphicsSettings` are applied to the new device: its present mode from them, and
  the MSAA level closest to the one asked for among those the new device offers.
- A `Frame` remembers its generation; `submit` drops one from an older generation or drawn while
  the device is lost. A failed reopening is an error for that frame, retried later.

## Precision

- Positions are converted relative to the eye in f64 before the f32 cast, and the view matrix is
  rotation only, so geometry far from the origin stays exact.
- A `ShadedMesh` stores f32 positions relative to its own centre; the eye-to-centre offset is
  computed in f64 each frame.

## Meshes

- A `MeshInstance` is an `Arc<ShadedMesh>` (faces of points with normals) plus a `FaceStyle`
  (colour, pick id) per face. Vertex and index buffers upload once per `Arc` and drop when the mesh
  leaves the scene (a frame with no viewport to draw keeps them).
- A mesh whose vertices or indices would pass the device's `max_buffer_size` is split by triangles
  into parts that each fit (`split_into_parts`).
- Per-face styles live in an `Rg32Uint` texture (8-bit RGBA colour, then pick id), filled row by
  row up to the device's largest texture side (`StyleLayout`; later faces take the last style) and
  read by face index with `textureLoad` in the vertex shader (no storage buffer).
- Each frame writes only the eye's offset to the mesh centre (with face count and row width) to a
  small uniform; styles are written only when they differ from the last ones, so hover and
  selection cost nothing in geometry.
- Faces are lit two-sided. `Shading::Standard`: a key light above and left of the camera, a
  headlight and a small specular term. `Shading::Enhanced`: a hemisphere ambient (brighter for
  normals towards world +Z), the key light, a weaker fill light below and right of the camera (the
  uniform's `fill_light`), a smaller headlight, a sharp Blinn-Phong highlight with a broad sheen,
  and a rim term towards grazing angles; highlight and rim scale with the face colour's luminance,
  so dimmed and tinted bodies stay darker and keep their hue (an offscreen test holds both). Normals
  are the mesh's own. They write depth, hiding edges and sketches behind them in view and picking alike
  (everything but the `Front` layer); a face without a pick id writes id 0 with its depth in the
  pick pass (`fs_mesh_pick`), not discarded.

## Depth, buffers and layers

- Reverse-Z with an infinite far plane and `Depth32Float`, multisampled at the anti-aliasing level
  in use (4x by default; multisampled colour is resolved into the surface and discarded, never
  stored); with MSAA off the viewport draws straight into the surface. The UI is drawn on the
  resolved surface after the 3D pass. The front layer and line depth biases work per sample, and an
  offscreen test draws and picks front geometry at every offered level.
- Line, marker and fill vertices use `GrowableBuffer`s: grow to the next power of two, shrink after
  300 uploads using under a quarter. They never pass `max_buffer_size`: a larger scene draws only
  its first whole lines, markers and fill triangles (logged once) rather than invalidating the
  frame's encoder, which the UI shares.
- The surface is `Bgra8Unorm` or `Rgba8Unorm` when offered (never a float or snorm format an HDR
  setup lists first), else the first non-sRGB one.
- Model geometry draws over reference geometry (datum planes, axes) through a per-`Layer` depth
  bias, and model-layer fills (sketch regions) over the faces they lie on.
- `Layer::Front` draws over everything else whatever its depth, in view and picking alike (the app
  puts the edited sketch there). Every line, marker and fill vertex carries an `in_front` flag:
  `layered_depth` in the shader halves every depth into the far half of the range (an exact scaling,
  so no precision is lost) and moves front geometry into the near half, one draw per primitive kind
  as before. Front geometry stays depth tested among itself, with depth biases wide enough to
  survive the coarser floats of the near half (fills under lines under markers); front fills sort
  after every other fill.

## Lines

- A `Line` has a `Stroke`; `Stroke::Dashed` carries the distance along its curve at its start.
  `vs_line` scales it by the segment's on-screen length per model unit into points, so `fs_line`
  draws dashes of `DASH_PERIOD_POINTS` (60% drawn) running on across a polyline's segments at any
  zoom and interface size. The pick pass draws dashed lines whole, so a gap still picks its curve.

## Projection

- `camera::Projection` (held by the `Camera`, carried by each `View`) is perspective (30° vertical
  field of view) or orthographic.
- Orthographic shows at every depth the scale perspective shows at its target (half height
  `distance · tan 15°`), so switching keeps the model's on-screen size; zoom still changes
  `distance`. The eye stays `distance` in front of the target (relative-to-eye precision
  unchanged), but the depth range is finite, centred on the target.
  - `View::reaching` widens it to the scene's bounds (the app passes `BuiltScene::everything`); it
    spans at least forty distances either way, so geometry behind the eye is drawn and the grid
    fades before the range ends.
- Rays start at the near plane along the view direction; picks are placed by `View::unproject`
  (point at a pixel and view depth, either projection). Fitting uses the tangent of the half angle,
  not its sine; grid spacing follows the distance, not the eye's height.
- The view uniform flags orthographic views: shaders light faces from the view direction and turn
  each layer's depth bias into a fixed depth offset (`ORTHOGRAPHIC_DEPTH_BIAS`), since depth is
  linear there and a factor would push edges far through faces.

## Sizes on screen

- Logical points: `ViewportFrame::pixels_per_point` (egui's: window scale times interface size)
  goes into the view uniform; shaders scale line widths, marker diameters and the grid's line width
  and fade by it, like the app's snapping and annotations.

## Picking

- Renders a window of `PICK_RADIUS_POINTS` (7.5) around the cursor, sized in physical pixels from
  the scale (`PickWindow`; targets and readback recreated when it changes).
- ID and depth targets, both `R32Uint` (depth as the bits of its f32, since GL does not always
  render to float targets), read back asynchronously so hover never blocks the UI thread.
- Hits report their distance from the cursor in points (`offset_points`, compared against the
  app's pick tolerances) and their world position (navigation's orbit pivot, pan grab point, zoom
  anchor).
- `poll_pick` says `Pending`, `Ready` or `Failed`: a failed readback (targets and buffer are then
  remade) or a pick whose frame was dropped before `submit` (the next `begin_frame` abandons it).
  The app asks again after a failure.
- Reference-layer fills (principal and datum planes) are drawn first in a pass of their own,
  nearest winning, and everything else over them: a translucent plane owns a pixel only where no
  face, line, marker or model or front fill covers it, and a face seen through a plane is picked.
  Front-layer lines, markers and fills are picked through any face, as they are drawn.

## Image export (`image.rs`)

- `Renderer::render_image` draws an `ImageRequest` (size, view, scene, pixels per point,
  `Background::Viewport` or `Transparent`) offscreen, independent of the window: at most
  `MAX_IMAGE_SIDE` (8192) pixels a side, in square tiles of `TILE_SIDE` (2048, or the device's
  largest texture side if smaller), so no size the app offers can pass the device's texture limit
  and memory stays bounded. Each tile writes the view uniform with `image::tile_transform`, the
  same clip-space scale and offset the pick window uses, so line widths and grid fades stay those
  of the whole image; one submit per tile, since uniform writes land at the next submit.
- It reuses the window's `ViewportRenderer` (its pipelines at the anti-aliasing level in use, its
  shading, its uploaded meshes) when the surface is `Rgba8Unorm` or `Bgra8Unorm`; any other
  surface format draws through a throwaway `ViewportRenderer` in `Rgba8Unorm` at the offered level
  closest to the one in use. Tile targets and readback buffers are its own; the window's scene
  targets are untouched. The shader output is written unconverted, as on screen, so the pixels are
  sRGB.
- Encoding and submitting run inside out-of-memory and validation error scopes
  (`ImageError::OutOfMemory`, `Refused`); a lost device, before or while reading back, is
  `DeviceLost`. One image at a time (`Busy`).
- Every tile's buffer (rows padded to `COPY_BYTES_PER_ROW_ALIGNMENT`) is mapped asynchronously;
  `poll_image` says `Idle`, `Pending`, `Ready(ImageReadback)` or `Failed`, so the UI thread never
  waits on the GPU. `ImageReadback::into_image` is `Send` and meant for a worker: it assembles the
  tiles, swaps BGRA to RGBA and turns the premultiplied colour a transparent clear leaves into
  straight alpha (opaque pixels unchanged).
- The renderer draws whatever scene it is given; leaving out highlights and the grid is the app's
  choice (`app-files.md`).

## Navigation

- One model: right-drag orbits (turntable around world Z, tilting about the horizontal, stopping at
  the poles; a rolled view such as one facing a tilted sketch turns level at twice the orbit rate).
  Middle-drag or Shift+right-drag pans; wheel and pinch zoom toward the point under the cursor.
- View cube and fit changes animate. Fitting bounds of no size keeps the distance and recentres.
