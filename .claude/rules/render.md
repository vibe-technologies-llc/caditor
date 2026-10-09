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
  `adapter_rank`: device type, then `BACKEND_ORDER` (Vulkan, OpenGL; Direct3D 12 first on
  Windows). `WGPU_BACKEND` limits the backends tried. Each adapter gets its own limits, then defaults, then WebGL2-level ones.
- Nothing uses storage buffers, so downlevel and GL devices draw everything. The surface is
  clamped to the largest texture side and is a plain 8-bit format, never a float or snorm one an
  HDR setup lists first. Offered MSAA levels (`gpu::offered_msaa`) need surface and
  `Depth32Float` support with resolve.
- `GraphicsSettings` (vsync, `Msaa`, `Shading`, `AdapterPreference`) is applied live by
  `Renderer::set_graphics`, which changes only what differs; a changed adapter preference opens a
  new device the way device loss does (frames are skipped until it answers, and a failure keeps
  the old device); `graphics_info` reports what is actually in use. MSAA uses the
  offered level closest to the one asked for (`Msaa::closest`); a change rebuilds the scene
  targets and only the colour pipelines (the pick pass is always single-sampled, so its pipelines
  stay), keeps mesh buffers and picking, and sets the old level's pipelines aside, so going back to
  a level used before builds nothing (offscreen test).
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
- New meshes upload across frames under one byte budget a frame (`MESH_UPLOAD_BYTES_PER_FRAME`,
  shared by the four mesh caches): each frame packs and writes the next whole vertices and indices
  into buffers made at the start, so the frame that first shows a large body never stalls (about
  2 ms at worst instead of 8 to 11 ms for 39 MB in a release build). A mesh is drawn only once it
  is complete, never half; while any mesh of a cache is still uploading, the meshes the cache drew
  before that are no longer in the scene stay drawn (the old result of a recomputed body), at their
  last styles with their pick ids withdrawn, since the app's pick table no longer knows them, so
  they hide what is behind them in the pick pass but pick nothing. `Renderer::is_uploading` says
  whether frames must follow (`app.md`); image export uploads whatever is left at once.
- The ignored `frame_costs_of_drawing_a_large_scene` test times the UI thread's share of a frame
  for a scene of lines, markers, fills and four 245,000-triangle meshes: idle, with the camera
  moving, hovering with a pick and a face restyled every frame, with the batch replaced every
  frame, and with new meshes shown every 20 frames, uploaded whole and under the budget.
- A `MeshInstance` may carry a `placement` (a `RigidTransform`) drawing the mesh moved and turned
  without a new upload: the placement uniform, rewritten only when the placement or the eye moved,
  holds the turned axes and the placed centre relative to the eye (worked out in f64), and
  `vs_mesh` turns positions and normals by them. An `Arc` appears at most once in a scene, since
  buffers, styles and placement are kept per mesh.
- A mesh whose placed bounds lie wholly beyond one side of the clip volume is not drawn
  (`culling::ClipWindow`, the eight placed corners against the clip planes in f64), tested against
  the window in the main pass, the pick window in the pick pass and each tile in image export.
- `ShadedMesh::divide` makes another mesh in which every face is split into one face per class a
  caller's classifier gives each triangle (from its corners' positions and normals), with the
  source face and the area of every piece; analysis colouring uses it, since styles are per face.
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
- Faces carry a slope-scaled pipeline bias (`FACE_DEPTH_BIAS`, two slopes back in reverse-Z), so
  an edge beyond a face seen at a grazing angle is not eaten where the face's depth races across
  the line's width; reference fills (their own `reference_fills` pipeline, chosen per draw by
  `FillDraw::behind_faces`, and `pick_reference_fills`) and the grid sit behind faces on their plane
  by a larger slope bias (`BEHIND_FACES_DEPTH_BIAS`) and a factor (`BEHIND_FACES`,
  `GRID_DEPTH_BIAS`), so a face lying on the XY plane never speckles with the grid or a principal
  plane (offscreen tests).
- `Layer::Hidden` lines draw after the model's lines with a depth test of `Less` and no depth write
  (`hidden_lines` pipeline), so they show only where a nearer face covers them; they sit after the
  other lines in the batch's buffer (`OrderedLines`), outside `line_count`, so the pick pass never
  draws them.
- `Layer::Front` draws over everything whatever its depth, in view and picking alike (the app
  puts the edited sketch there). `layered_depth` halves every depth into the far half of the range
  (an exact scaling) and moves front geometry into the near half, where biases stay wide enough
  for the coarser floats (fills under lines under markers).

- `Scene::flat_meshes` draw right after the opaque meshes with the same depth writes and pick pass
  but `fs_color`, so each face shows its style's colour exactly, unlit (the hidden-line style).
- `Scene::overlay_meshes` draw right after the translucent ones, blended, with no depth test or
  write and never in the pick pass, so they show through whatever covers them (the cut preview).
- `Scene::translucent_meshes` draw after the opaque meshes and before lines with alpha blending and
  no depth write (`translucent_meshes` pipeline), so edges and what lies behind show through. The
  pick pass draws them with `fs_pick`, which discards faces without a pick id, so an unpickable
  see-through face (X-ray) never occludes or takes a pick while one carrying an id picks as usual.

## Lines, markers and sizes

- Sizes are logical points: `ViewportFrame::pixels_per_point` goes into the view uniform and
  shaders scale line widths, marker diameters and the grid by it.
- `Stroke::Dashed` carries the distance along the curve at its start, so dashes
  (`DASH_PERIOD_POINTS`) run on across a polyline's segments at any zoom and interface size. The
  pick pass draws dashed lines whole, so a gap still picks its curve.
- A marker or line whose colour has no alpha draws nothing but is still picked, so pickable points
  and edges can stay invisible until hovered or selected. Such ones are uploaded after the drawn
  ones of their batch (`shown_first`, order kept within each) and the colour pass draws only the
  drawn ones, so body vertices and the edges of a style without them cost the colour pass nothing.
- Markers at one place in one layer have equal depths, which `GreaterEqual` passes, so they draw in
  batch order: a smaller unpicked marker after a larger one makes a ring that still picks whole
  (the app's hollow sketch points). `BACKGROUND`, the canvas clear colour, is public so the app
  tests its scene colours against it.

## Projection

- `camera::ProjectionMode` is what the user chooses and the `Camera` keeps: perspective,
  orthographic or automatic, which is perspective and turns orthographic while the viewpoint looks
  square at a coordinate axis (within `SQUARE_TOLERANCE_DEGREES`, the six front, back, top, bottom,
  left and right views, not the isometric one). `Camera::view` resolves it against the current
  viewpoint, so a `View` only ever holds the effective `camera::Projection`, perspective or
  orthographic. Orthographic shows at every depth the scale
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
  dropped before `submit`); the app asks again after a failure. `is_pick_answered` polls the device
  without drawing and says whether `poll_pick` would answer, so a pick in flight needs no frames.
- Reference-layer pick fills (principal and datum planes) are drawn first in a pass of their own
  and everything else over them: a translucent plane owns a pixel only where no face, line,
  marker or model or front fill covers it, and a face seen through a plane is picked.
  Front-layer geometry is picked through any face, as it is drawn.
- `Scene::hits_through` (`through.rs`) answers on the CPU what lies under a pixel at every depth,
  for the app's pick list: faces of the opaque, flat and translucent meshes (placed, a ray test
  per triangle after one against the placed bounds) and pickable fills the ray meets, and
  pickable lines and markers within `PICK_RADIUS_POINTS` of it, each id once with its nearest
  offset. It lists front-layer geometry first, then the rest nearest first, then reference fills,
  so the order follows what the pick pass would let win; `Layer::Hidden` and overlay meshes are
  never listed, as they are never picked.

## Image export (`image.rs`)

- `Renderer::render_image` draws an `ImageRequest` offscreen, independent of the window, at most
  `MAX_IMAGE_SIDE` a side in tiles of `TILE_SIDE` (or the largest texture side), so no size the
  app offers can pass the texture limit. Each tile writes the view uniform with
  `image::tile_transform`, the same clip-space transform the pick window uses, so line widths and
  grid fades stay those of the whole image; one submit per tile, since uniform writes land at the
  next submit.
- Memory is a small multiple of one band (a row of tiles), never the image: `ImageTiles` (kept
  by the `Renderer`) draws tiles in rows from the top into at most `READBACK_BUFFERS` readback
  buffers, and `ImageBands` (`Send`, for the writer's thread) waits for each tile by polling the
  device every millisecond (never wgpu's blocking wait, which on GL holds the context the UI
  thread draws with), copies it into the band, returns the buffer and wakes the app, whose next
  frame's `advance_image` draws the next tiles into the returned buffers. A finished band is
  straightened and handed out whole rows at a time; 8192² takes about 75 MB rather than 260 MB
  (debug build, peak RSS above an idle process). Errors after the start reach the bands in order;
  dropping the bands stops the tiles at the next frame.
- The tiles draw with their own `ViewportRenderer` (`image_sibling`): the window's pipelines,
  layouts and uploaded mesh buffers shared, its own uniforms, batches and face styles, so the
  window's frames between tiles neither disturb the image nor upload anything again. When the
  surface is neither `Rgba8Unorm` nor `Bgra8Unorm` a throwaway one in `Rgba8Unorm` is built
  instead. Output is written unconverted, as on screen, so pixels are sRGB.
- Errors are `ImageError` (`Busy` while another image's tiles are still drawing, `OutOfMemory`,
  `Refused`, `DeviceLost`, `Readback`, `Abandoned` when the tiles stopped without saying
  why). Nothing on the UI thread waits on the GPU, and the bands turn the premultiplied colour of
  a transparent clear into straight alpha.
- `OffscreenRenderer` opens a device without a surface and draws the same `ImageRequest` into an
  `Rgba8Unorm` target, its `ImageBands` drawing the tiles inline as it reads them (`drawn_inline`):
  the headless `--export` of a PNG (`app.md`) streams them to the file.
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
  selection. Alt pressed once a primary drag has begun in the sketch (a grab, a box, a shape drawn
  by press and drag) leaves the drag to it, where Alt holds the snap (`app-sketching.md`). Fusion 360 (middle-drag pans, Shift+middle-drag orbits), FreeCAD (its CAD style:
  middle-drag pans, the middle button held with the left or right one orbits) and Blender
  (middle-drag orbits, Shift+middle-drag pans, Ctrl+middle-drag zooms) follow those programs;
  every mode keeps right-drag orbiting and Shift+right-drag panning, the wheel zooms, and a primary
  drag with the middle button held never starts a selection (`viewport::drag_motion`).
