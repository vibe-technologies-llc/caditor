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
- The 3D pass resolves into a surface-sized texture of the scene targets (`kept.rs`, `KeptView`)
  that one full-screen triangle (`ViewCopy`, `kept.wgsl`, an exact `textureLoad` of each pixel)
  copies onto the surface every frame, so an egui-only repaint (a tooltip, hover over a panel, a
  spinner tick) copies the last view instead of drawing it again. The renderer decides alone,
  trusting no caller's generation: a frame draws the view when the view uniform's bytes (view,
  anchor, scale, shading, reflection, section) or the grid's changed, a mesh or silhouette cache
  wrote a placement or styles or draws a different list (`DrawnOrder`, the mesh addresses in
  draw order), a batch slot uploaded or the batch count changed, the fills were sorted again, or
  the rect or the grid's presence differs from what the kept view holds (`kept::Shown`); anything
  new the 3D pass reads must join that check (`Prepared::changed`, worked out in `upload`). New
  scene targets (resize, multisampling, linear resolve, a new device) start empty. The kept view
  counts only once its frame is submitted (`ViewportRenderer::after_submit`; `begin_frame`'s
  `abandon_unsubmitted` forgets an unsubmitted one, as for picks), so a dropped encoder never
  leaves a stale view claimed. The pick pass is drawn whenever asked, copied frame or not
  (offscreen test). The surface is configured for `RENDER_ATTACHMENT` alone, so the copy is a
  draw rather than `copy_texture_to_texture`, which would need `COPY_DST` on the surface.
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
- A pick readback is answered without the app polling for it: after `submit`, `Picking` maps the
  readback buffer and hands the device, the submission index and a settled flag to its helper
  thread (`pick-readback`, started on the first pick), which blocks in `Device::poll` with
  `PollType::Wait` in `WAIT_SLICE` steps until the map callback ran, then calls the renderer's
  `Wake` (the renderer knows nothing of winit). The wait gives up after `WAIT_LIMIT`, on a poll
  error, or when the `Picking` is dropped (a replaced device, shutdown; its drop joins the thread, at most one slice later), and wakes either way; a
  lost device is also woken by `DeviceLoss`. `Picking::is_answered` and `poll` still poll the device
  themselves, which is the app's slow fallback timer (`app.md`).

## Precision

- Model positions are converted relative to a nearby point in f64 before the f32 cast and the view
  matrix is rotation only, so geometry far from the origin stays exact.
- Batches store positions relative to an anchor (the eye when uploaded), whose offset from the
  eye the view uniform carries; it stays while the eye is within `reanchor_reach`, the larger of
  `REANCHOR_DISTANCES` view distances and the distance at which f32 rounding of an offset stays
  under `ANCHOR_ERROR_PIXELS` of a pixel at the current zoom; beyond that the next frame
  re-anchors and uploads every batch again.
- A `ShadedMesh` stores positions relative to its own centre; its placement uniform (and a
  silhouette's) holds the placed centre relative to the same anchor, worked out in f64, and the
  shader adds the view uniform's anchor offset to it before the mesh-local position, so a camera
  move writes no mesh or silhouette uniform until the anchor moves (offscreen test with a mesh
  4,000 km out).

## Meshes

- A `MeshInstance` is an `Arc<ShadedMesh>` plus a `FaceStyle` (colour, pick id) per face. Buffers
  upload once per `Arc` into the `MeshPool` the five mesh caches share (keyed by the `Arc`'s
  address, the entry holding the `Arc`) and drop when no list of the scene holds the mesh; each
  cache keeps only its styles, placement and bind group per mesh, so moving a mesh between lists
  (zebra, X-ray, hidden lines) or drawing it in two (a partly see-through body) draws it at once
  without uploading it again (offscreen test). The pool keeps a refused mesh refused while any
  list holds it. A mesh past `max_buffer_size` is split into parts that each fit.
- New meshes upload across frames under one byte budget a frame (`MESH_UPLOAD_BYTES_PER_FRAME`,
  shared by the five mesh caches and the silhouette cache): each frame packs the next whole
  vertices and indices into buffers made at the start, so the frame that first shows a large body
  never stalls (under 2 ms at worst instead of 11 ms for 31 MB of meshes and 47 MB of silhouettes
  in a release build). A mesh is drawn only once it
  is complete, never half; while any mesh of a cache is still uploading, the meshes the cache drew
  before that are no longer in the scene stay drawn (the old result of a recomputed body), at their
  last styles with their pick ids withdrawn, since the app's pick table no longer knows them, so
  they hide what is behind them in the pick pass but pick nothing. `Renderer::is_uploading` says
  whether frames must follow (`app.md`); image export uploads whatever is left at once.
- The ignored `frame_costs_of_drawing_a_large_scene` test times the UI thread's share of a frame
  for a scene of lines, markers, fills and four 245,000-triangle meshes with their silhouettes: idle, with the camera
  moving, hovering with a pick and a face restyled every frame, with the batch replaced every
  frame, with new meshes shown every 20 frames, uploaded whole and under the budget, and with the
  meshes switched to zebra or also drawn see-through every 20 frames; and 2,000 small placed
  meshes with silhouettes, idle and with the camera moving. Each reports the time spent waiting
  for the GPU apart from the UI thread's; `CADITOR_BENCH_CASE` runs only the cases whose name
  contains it.
- A `MeshInstance` may carry a `placement` (a `RigidTransform`) drawing the mesh moved and turned
  without a new upload: the placement uniform, rewritten only when the placement or the anchor
  moved, holds the turned axes and the placed centre relative to the anchor (worked out in f64), and
  `vs_mesh` turns positions and normals by them. An `Arc` appears at most once in each list of a
  scene, since styles and placement are kept per mesh in each cache.
- A mesh whose placed bounds lie wholly beyond one side of the clip volume is not drawn
  (`culling::ClipWindow`, the eight placed corners against the clip planes in f64), tested against
  the window in the main pass, the pick window in the pick pass and each tile in image export.
  A batch is culled the same way, its lines, hidden lines, markers and fills alike, by the bounds
  of all its points worked out on upload (`BatchBounds`), but only against the four side planes
  widened by its reach (half its widest line or marker plus `STROKE_FRINGE_POINTS`, turned into
  the window's clip units by `ClipWindow::sees_reaching`), since strokes and marks reach that far
  past their points on screen and front-layer depths are moved; so a mark whose centre lies
  outside the pick window but whose disc covers the cursor still picks (offscreen test).
- A `ShadedMesh` either owns its vertices (`ShadedMesh::new` from `MeshFace`s: 28 bytes a vertex
  on the CPU and 12 a triangle) or reads them from a `MeshSource` it shares (`ShadedMesh::shared`): the
  source's triangles are its indices, and each vertex is converted (position relative to the
  centre, normal, face) when uploaded or read, its face found among the per-face vertex ends, so a
  body's display mesh is held once on the CPU, by the kernel (`app.md`). `shared` takes each face's
  end in the triangles and refuses (`None`) a source whose faces do not each use their own
  contiguous run of vertices in face order, covering every vertex; the caller then copies.
- `ShadedMesh::divide` makes another mesh in which every face is split into one face per class a
  caller's classifier gives each triangle (from its corners' positions and normals), with the
  source face and the area of every piece; analysis colouring uses it, since styles are per face.
  `ShadedMesh::origin` and `face_triangles` (world corners with their face) let the app's reach
  analysis and `through.rs` work on the triangles without a copy of the mesh.
- Per-face styles live in an `Rg32Uint` texture (`StyleLayout`) read by face index in the vertex
  shader, so hover and selection cost nothing in geometry. A restyle writes only the texture rows
  whose styles changed, each as one span from its first to its last changed face, and updates the
  kept copy in place (`changed_spans`); past `MAX_STYLE_SPANS` such rows, or when the face count
  changes, the whole texture is written.
- Faces are lit two-sided and write depth, hiding edges and sketches behind them in view and
  picking alike (everything but `Layer::Front`). A face without a pick id writes id 0 with its
  depth in the pick pass, not discarded. Enhanced shading scales highlight and rim with the face
  colour's lightness so dimmed and tinted bodies stay dark and keep their hue (offscreen test).

## Colour and light

- Colours are sRGB-encoded everywhere (palettes, styles, `BACKGROUND`) and every pass draws on
  the plain 8-bit view, so blending stays in the space the palettes' contrast tests model
  (`scene_palette.rs`). Only `fs_mesh` works in linear light: it decodes the face colour
  (`to_linear`), lights it and encodes the result (`to_srgb`), the light constants tuned so a
  face lit by ambient alone or fully lit reads about as it did when lighting was in gamma space, the
  tones between a little lighter. Zebra and chrome keep their sRGB-space look.
- The multisample resolve averages in linear light where views allow it: a renderer told so
  (`set_linear_resolve`, from `gpu::resolves_linearly`, the adapter's `VIEW_FORMATS`) gives its
  multisampled colour target and the kept view an sRGB view each, stores the scene pass instead
  of resolving it and resolves in an empty pass through the two sRGB views (`ColorAttachment`),
  so a half-covered edge pixel is the linear mean of its samples (offscreen test). The resolve
  lands in the kept view, never the surface, so the surface needs no sRGB view format and a
  conservatively configured one resolves linearly too. Image tiles on the viewport background do
  the same; a transparent image resolves in gamma, since its bands straighten premultiplied
  colour there. GL and other devices without view formats resolve in gamma as before. The extra
  pass costs a few microseconds a frame in the frame-cost benchmark (release, 1600 by 1000 at
  4x).

## Depth, buffers and layers

- Reverse-Z, infinite far plane, `Depth32Float`, multisampled at the level in use (never stored;
  colour is resolved into the kept view, through the linear resolve pass where views allow it,
  and copied onto the surface). The UI is drawn on the surface after the copy.
- Each batch has a `GpuBatch` slot of `GrowableBuffer`s. A slot uploads only when its `Arc`
  differs or the anchor moved, so an idle frame or a camera move writes no vertices. A batch past
  `max_buffer_size` draws only its first whole primitives (logged once). A buffer an upload
  outgrows is replaced by one a quarter larger than the upload (aligned), and one an upload fills
  to under a quarter by one fitting that upload the same way, so the band between keeps it.
- Vertex, index and silhouette records are packed one fixed-size `gpu::record` at a time straight
  into wgpu's staging memory (`gpu::write_records` over `Queue::write_buffer_with`), so no CPU copy
  of a batch or mesh outlives its upload; the `Bytes` staging left for uniforms and face styles
  drops any capacity past 64 KiB when cleared. Replacing the frame-cost benchmark's batch every
  frame costs about 2.8 ms instead of 7.9.
- Records are as narrow as the shaders allow: a mesh vertex is 20 bytes (`MESH_VERTEX_STRIDE`:
  position, normal, face), a silhouette 48, a line 48, a marker 32 and a fill vertex 28. Normals are
  octahedral `Snorm16x2` (`Pack::octahedral`, `unfolded` in the shader), back within 0.01°; a zero
  or non-finite normal, which tessellation gives at a degenerate point and `facing_normal` replaces
  by the eye direction, is kept as the corner code (-1, -1) the shader reads as zero, and normals
  within about 0.02° of -Z that would land beside that code take the opposite corner instead.
  Colours are `Unorm8x4` (`Pack::unorm8x4`, rounded): palettes are 8-bit sRGB and every pass draws
  them unconverted on an 8-bit view, so an 8-bit colour reaches the target exactly as before.
- A batch uploads its fills once, ordered pickable reference fills, then the other pickable fills,
  then the rest, each group in batch order (`FillGroups`): the pick pass draws the first two
  groups as one range each of the colour pass's buffer, and the colour pass's spans keep batch
  order, so fills of equal depth still draw in the order given.
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
- A line stroked `Stroke::DashedWhereHidden` (the hidden-edges style, threads) is drawn as usual,
  solid or dashed by `seen_dashed` (a solid one carries `SOLID_WHERE_SEEN` in its flags, since its
  record holds the distance along for the hidden dashes), and its own record is drawn again after
  every batch's lines with a depth test of `Less` and no depth write (`hidden_lines` pipeline,
  `vs_hidden_line`, always dashed), so the dashes show only where a nearer face covers it. The
  batch keeps the runs of its shown lines so stroked (`hidden_runs`), one draw each, so the scene
  holds no second copy of a line, and the pick pass never draws the hidden part (offscreen test,
  and pixel for pixel the look of the copies it replaced).
- `Layer::Front` draws over everything whatever its depth, in view and picking alike (the app
  puts the edited sketch there). `layered_depth` halves every depth into the far half of the range
  (an exact scaling) and moves front geometry into the near half, where biases stay wide enough
  for the coarser floats (fills under lines under markers).

- `Scene::flat_meshes` draw right after the opaque meshes with the same depth writes and pick pass
  but `fs_color`, so each face shows its style's colour exactly, unlit (the hidden-line style).
- `Scene::reflective_meshes` draw right after the flat ones with the opaque depth writes and pick
  pass but `fs_reflective`, which reflects the eye ray about the normal in world space and shades
  by `Scene::reflection`: `Zebra` (a ring of `stripes` light and dark bands per turn around an axis,
  antialiased with the narrower `fwidth` of two angles whose seams differ, the light band the face
  colour brightened and the dark one nearly black) or `Chrome` (a procedural world-Z-up sky,
  horizon, ground and one light panel, tinted halfway to the face colour's hue, so hover and
  selection still show). The reflection rides in the view uniform's last two vectors (the stripe
  axes' orthonormal pair, the stripe count and a zebra flag).
- `Scene::silhouettes` (`silhouette.rs`) draw the outline of curved faces as seen from the
  current view, after every mesh and before the batches' lines, with the line pipeline's depth
  test and `fs_line`. A `Silhouette` names a mesh, a colour, a width, a dash flag, a
  `dashed_where_hidden` flag and a placement, with no faces drawn needed, so wireframe shows it
  too. One `dashed_where_hidden` is drawn again after the hidden lines like them
  (`hidden_silhouettes` pipeline: `vs_hidden_silhouette`, always dashed, depth `Less`, no depth
  write), so its hidden part shows dashed where a nearer face covers it (offscreen test).
- Silhouettes never draw in the pick pass, visible or hidden: an outline is not topology, has no
  stable name and slides over its face as the view turns, so the selection model (named faces,
  edges and vertices) has nothing to give it but its face, which the pixels inside the outline
  already pick; in wireframe, where no face picks, it picking its face would make curved faces
  selectable there and flat ones not. On upload (under the same per-frame byte budget as meshes, in chunks that fit a
  buffer) every triangle whose corner normals differ (`ShadedMesh::curved_triangles`, so flat
  faces cost nothing) becomes a `SILHOUETTE_STRIDE` (48-byte) instance of three positions and
  three octahedral normals; `vs_silhouette` finds where the facing of the interpolated normals toward
  the eye changes sign across the triangle and strokes that segment like a line, so the outline
  follows every orbit without re-meshing and with no CPU work per frame. A dashed silhouette
  dashes along its dominant screen axis, since contour segments carry no distance along a curve.
  `ShadedMesh` counts its curved triangles when built, so starting an upload costs nothing. The
  frame-cost benchmark gives its four meshes waving normals and silhouettes, so every triangle is
  a candidate (about 11.8 MB each against 7.8 MB of mesh): steady frames stay within noise (about
  55 µs idle or orbiting, release build), an upload frame under the budget stays under 2 ms
  at worst but new meshes take about 2.5 times as many frames.
- `Scene::overlay_meshes` draw right after the translucent ones, blended, with no depth test or
  write and never in the pick pass, so they show through whatever covers them (the cut preview).
- `Scene::translucent_meshes` draw after the opaque meshes and before lines with alpha blending and
  no depth write (`translucent_meshes` pipeline), so edges and what lies behind show through. The
  pick pass draws them with `fs_pick`, which discards faces without a pick id, so an unpickable
  see-through face (X-ray) never occludes or takes a pick while one carrying an id picks as usual.
- `fs_mesh` discards a face of no alpha, so a body with only some faces see-through is drawn as
  two instances of one mesh: in `meshes` with those faces at alpha 0 and in `translucent_meshes`
  with only those faces visible, both carrying every face's pick id.

## Sections

- `Scene::section` holds up to `MAX_SECTION_PLANES` `SectionPlane`s (a `Plane` and a `CutFace`,
  hatched or filled); each cuts away the side its normal points to, so together they keep what
  every plane keeps. It only changes what is drawn and picked, never a mesh or a batch, so moving
  a plane costs a uniform write: the view uniform carries the count, a slack
  (`section_slack`, a fraction of the view distance, so a face lying on a plane is kept), each
  plane as its normal and offset relative to the eye (worked out in f64) and each plane's hatch
  direction over its spacing with a phase anchored to the plane, the spacing `HATCH_SPACING_POINTS`
  at the target rounded up to a power of two so the hatch stays put while zooming a little.
- Every mesh and silhouette, and lines, markers and fills of `Layer::Model`
  (`Layer::is_sectioned`, a bit in the instance flags beside the front-layer bit), are discarded
  beyond a plane in view and picking alike; reference and front geometry never are, so datums,
  the edited sketch and the front-layer marks stay whole.
- While a section is shown the opaque, flat and reflective meshes draw with `fs_*_sectioned`
  pipelines (and `fs_mesh_pick_sectioned`), which cap the cut: a back face (the flat normal from
  derivatives, oriented by the interpolated normal, turned from the eye) seen where the eye's ray
  crossed a plane is the inside of a cut solid, since a closed solid's back face is the nearest
  kept surface only from inside it. It is drawn as the plane there: shaded with the plane's normal
  in the face's colour darkened by `CAP_SHADE`, hatched (`HATCH_SHADE`) or filled from the point
  where the ray entered the kept region. Its depth stays the back face's, pulled forward by
  `CAP_DEPTH_BIAS` so the back faces' edges (lines at their smaller bias) stay hidden behind it;
  moving it onto the plane instead would let it win over front faces between the plane and the
  back face. The pick pass writes id 0 there with the plane's depth value, so a cap hides what
  lies behind it, picks nothing and gives the orbit pivot on the cut. These shaders write
  `frag_depth`, so they take no pipeline depth bias and add the face slope bias themselves; plain
  pipelines draw when there is no section, keeping early depth testing then. Only a closed mesh
  is capped (`ShadedMesh::is_closed`, `with_closed` clears it, set by the app from the solid's
  edges all having two uses; the placement uniform carries it and `vs_mesh` raises `CAPPABLE` in
  the instance flags), so the far side of a sheet that is no closed solid is drawn and picked as
  an ordinary face; translucent and overlay meshes are only cut, never capped.
- `Scene::hits_through` skips hits on sectioned geometry beyond a plane (`is_cut_away`), so the
  pick list, paint selection and Measure only reach what is shown (offscreen and `through.rs`
  tests).
- `kept_span` gives the part of a segment the planes keep as a parameter interval, for callers that
  clip curves to what is shown (box selection of sketch curves, `sectioned_screen.rs`).

## Lines, markers and sizes

- Sizes are logical points: `ViewportFrame::pixels_per_point` goes into the view uniform and
  shaders scale line widths, marker diameters and the grid by it.
- Lines, markers, silhouettes and the grid are quads, one instance each, drawn indexed through one
  static index buffer of two triangles over four corners (`gpu::QuadIndices`, bound once after the
  meshes in each pass), so the vertex shader runs four times a quad rather than six.
- Lines and silhouettes are finished in the colour pass (`Strokes::Finished` in the view uniform's
  `fill_light.w`): each quad reaches `STROKE_FRINGE_PIXELS` beyond its edges and `fs_line` turns the
  distance from the segment, carried in screen space (the `stroke` varying times `w`, divided
  back per fragment), into coverage, so lines are smooth at every multisampling level, Off
  included, at the same width. An opaque line also reaches half its width past each end and
  rounds it, which closes the notches where a polyline's segments meet; a translucent one keeps
  square ends, since overlapping caps would blend twice at every joint. The pick pass
  (`Strokes::Bare`) draws the bare quads as before, so pick reach is unchanged.
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
  so the order follows what the pick pass would let win; overlay meshes are never listed, as they
  are never picked.

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
- The moves are `Viewpoint::orbited`, `panned` and `zoomed` (nothing for non-finite input), so the
  app can move the viewpoint the camera is heading to rather than the one shown. `Camera::orbit`,
  `pan` and `zoom` apply them to the shown viewpoint and end any transition (the pointer moves what
  is under it); `animate_to` eases in and out over `TRANSITION_DURATION`, and `glide_to` eases out
  only over the shorter `GLIDE_DURATION`, so it moves at once and a key held down, each repeat
  gliding on from the last destination, follows smoothly instead of restarting from rest.
- `Camera::destination_view` and `view_from` build a `View` for the destination or any viewpoint
  with the projection resolved against that viewpoint, so a fit or a look at a face started during
  a turn into a standard view is fitted in the projection it will end in.
- The input mode (`preferences::InputMode`, chosen in Preferences › Navigation) is caditor, the
  mouse scheme above, or Laptop, for a touchpad: two-finger scroll orbits, Alt and scroll pans (egui
  turns Shift and scroll into horizontal-only scrolling, so Alt keeps both axes), pinch and
  Ctrl+scroll zoom, and Alt-drag orbits and Shift+Alt-drag pans, the Alt press never starting a
  selection. Alt pressed once a primary drag has begun in the sketch (a grab, a box, a shape drawn
  by press and drag) leaves the drag to it, where Alt holds the snap (`app-sketching.md`). Fusion 360 (middle-drag pans, Shift+middle-drag orbits), FreeCAD (its CAD style:
  middle-drag pans, the middle button held with the left or right one orbits) and Blender
  (middle-drag orbits, Shift+middle-drag pans, Ctrl+middle-drag zooms) follow those programs;
  every mode keeps right-drag orbiting and Shift+right-drag panning, the wheel zooms, and a primary
  drag with the middle button held never starts a selection (`viewport::drag_motion`). Invert
  zoom turns the wheel and Blender's Ctrl+middle-drag round alike (`viewport::zoom_factor`). A
  middle-button double-click fits the view in every mode, since none gives it another use. The
  canvas hint at the bottom right is the mode's `InputMode::navigation_hint`.
