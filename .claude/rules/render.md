---
paths:
  - "crates/caditor-render/**"
---

# Rendering (`caditor-render`)

- Owns the wgpu device and surface, the camera and the viewport. Depends on neither winit nor the
  document: takes any `Arc<dyn WindowTarget>` and draws a `Scene` the app builds (shaded meshes, a
  grid, `Arc<Batch>`es of lines, markers and triangle fills). It draws whatever scene it is given;
  leaving out highlights and the grid is the app's choice (`app-files.md`).
- The UI shares the frame's encoder, so nothing in the viewport may invalidate it: allocations run
  inside out-of-memory and validation error scopes (`gpu::scoped`) and a refusal degrades alone.

## Frames

- `begin_frame` reconfigures the surface when the size differs or it is outdated, draws the
  viewport and returns `FrameStart::Ready` with a `Frame` whose encoder the app draws the UI into;
  `submit` presents it.
- The 3D pass resolves into the scene targets' kept view (`kept.rs`, `KeptView`) that one
  full-screen triangle (`ViewCopy`, `kept.wgsl`, an exact `textureLoad` per pixel) copies onto the
  surface every frame, so an egui-only repaint copies the last view instead of redrawing it. The
  renderer decides alone, trusting no caller's generation: a frame draws the view when the view
  uniform's bytes or the grid's changed, a mesh or silhouette cache wrote a placement or styles or
  draws a different list (`DrawnOrder`), a batch uploaded or the batches drawn changed, the fills
  were sorted again, or the rect or grid presence differs from what the kept view holds
  (`kept::Shown`). Anything new the 3D pass reads must join that check (`Prepared::changed`, worked
  out in `upload`). New scene targets start empty. The kept view counts only once its frame is
  submitted (`ViewportRenderer::after_submit`; `abandon_unsubmitted` forgets an unsubmitted one, as
  for picks). The pick pass is drawn whenever asked. The surface is `RENDER_ATTACHMENT` only, so the
  copy is a draw, not `copy_texture_to_texture`.
- Scene targets (multisampled colour, depth, kept view) are sized with each side rounded up to
  `TARGET_SIDE_STEP` (256 px, within the largest texture side) and kept while the surface fits and
  they exceed its rounded size by at most one step (`TargetSize::holds`), so an interactive resize
  makes new ones every 256 px. The pass draws within the surface's rect and the copy reads from the
  top left.
- `Hidden` (occluded) makes the app stop drawing until shown or a probe timer fires, so a hidden
  Wayland window does not block the UI thread on the acquire timeout. `Skipped` (timeout, outdated
  or lost surface) is a retry, never a reason to stop.
- A `SurfaceValidation` acquire climbs a ladder, one rung per consecutive failure: reconfigure,
  recreate the surface, configure conservatively, open a replacement device.
- Refused scene targets step multisampling down a level (to Off, the preference untouched) and
  draw again; a refused mesh or batch is dropped alone and not retried until it leaves the scene
  or its anchor moves; refused pick targets disable picking. Each is one `RenderFault`
  (`Renderer::take_faults`), shown as a notice.

## Devices and settings

- `open_device` tries the adapter `AdapterPreference` asks for, then every other adapter that can
  present, ranked by `adapter_rank`: device type, then `BACKEND_ORDER` (Vulkan, OpenGL; Direct3D 12
  first on Windows). `WGPU_BACKEND` limits the backends tried, also for `OffscreenRenderer` and the
  offscreen tests (`new_without_display_handle_from_env`), so CI runs them on GL (`ci.md`); a test
  needing a Vulkan-only capability (the linear resolve) reads `linear_resolve()` and skips without
  it. Each adapter gets its own limits, then defaults, then WebGL2-level ones.
- `AdapterPreference` is Auto (default), Power saving or Performance; `WGPU_POWER_PREF`, when set,
  overrides all (`gpu::power_preference`). Auto resolves by `AdapterPreference::resolved` from
  `Machine::detect()` (`machine.rs`): a desktop is Performance, a portable or undetectable computer
  is Power saving. Detection runs once per process; `classify` is pure over what the platform item
  read. Unix (`machine/unix.rs`): portable when `/sys/class/power_supply/*/type` says `Battery` with
  a `scope` other than `Device`, or `/sys/class/dmi/id/chassis_type` is 8, 9, 10, 11, 14, 30, 31 or
  32; desktop when the chassis type (other than 0 to 2, which say nothing) is another one or the
  power supplies were readable without a battery; unknown when neither could be read. Windows
  (`machine/windows.rs`): `caditor_windows::battery_flag`: 128 desktop, 255 or a failed call
  unknown, anything else portable (a desktop on a UPS counts as portable). The app words the
  outcome (`Machine::reason`).
- Nothing uses storage buffers, so downlevel and GL devices draw everything. Every entry point of
  `viewport.wgsl` translates to GLSL ES 3.0
  (`every_entry_point_translates_to_the_glsl_the_opengl_backend_runs`), so a shader using what GLSL
  lacks (a `textureLoad` of a depth texture, ES 3.1 builtins) fails `cargo test`, not only the GL
  offscreen run. The surface is clamped
  to the largest texture side and is a plain 8-bit format, never a float or snorm one. Offered MSAA
  levels (`gpu::offered_msaa`) need surface and `Depth32Float` support with resolve.
- `GraphicsSettings` (vsync, `Msaa`, `Shading`, `AdapterPreference`) is applied live by
  `Renderer::set_graphics`, which changes only what differs; a changed adapter preference opens a
  new device like device loss does, but only when the power preference it resolves to differs from
  the one in use (frames are skipped until it answers; a failure keeps the old device);
  `graphics_info` reports what is in use. MSAA uses the offered level closest to the one asked for
  (`Msaa::closest`); a change rebuilds the scene targets and the colour pipelines only (the pick
  pass is always single-sampled), keeps mesh buffers and picking, and sets the old level's
  pipelines aside so going back builds nothing. Shading is a uniform flag.
- Device loss: `DeviceLoss::watch` wakes the app. The next `begin_frame` opens a new device on a
  worker thread (same surface, else a new one); until it answers frames are `Skipped`. The answer
  is installed on the UI thread, rebuilding the `ViewportRenderer` with the current settings and
  bumping `Renderer::generation`. A `Frame` from an older generation, or drawn while the device is
  lost, is dropped by `submit`; a pick in flight polls `Failed`.
- A pick readback is answered without the app polling: after `submit`, `Picking` maps the readback
  buffer and hands the device, submission index and a settled flag to its helper thread
  (`pick-readback`, started on the first pick), which blocks in `Device::poll` (`PollType::Wait`,
  `WAIT_SLICE` steps) until the map callback ran, then calls the renderer's `Wake` (the renderer
  knows nothing of winit). It gives up after `WAIT_LIMIT`, on a poll error, or when `Picking` is
  dropped (its drop joins the thread), and wakes either way; `DeviceLoss` also wakes a lost device.
  `Picking::is_answered` and `poll` still poll the device themselves, the app's slow fallback timer
  (`app.md`).

## Precision

- Model positions are converted relative to a nearby point in f64 before the f32 cast and the view
  matrix is rotation only, so geometry far from the origin stays exact.
- Batches store positions relative to an anchor (the eye when uploaded), whose offset from the eye
  the view uniform carries; it stays while the eye is within `reanchor_reach`, the larger of
  `REANCHOR_DISTANCES` view distances and the distance at which f32 rounding of an offset stays
  under `ANCHOR_ERROR_PIXELS` of a pixel at the current zoom; beyond that the next frame re-anchors
  and uploads every batch again.
- A `ShadedMesh` stores positions relative to its own centre; its placement uniform (and a
  silhouette's) holds the placed centre relative to the same anchor, worked out in f64, and the
  shader adds the view uniform's anchor offset, so a camera move writes no mesh or silhouette
  uniform until the anchor moves.

## Meshes

- A `MeshInstance` is an `Arc<ShadedMesh>` plus a shared `Arc<[FaceStyle]>` (colour, pick id,
  finish per face), so instances kept between scenes are handed over without copying styles.
  Buffers upload once per `Arc` into the `MeshPool` the five mesh caches share (keyed by the
  `Arc`'s address) and drop when no list of the scene holds the mesh; each cache keeps only its
  styles, placement and bind group per mesh, so moving a mesh between lists (zebra, X-ray, hidden
  lines) or drawing it in two uploads nothing. The pool keeps a refused mesh refused while any list
  holds it. A mesh past `max_buffer_size` is split into parts that each fit. An `Arc` appears at
  most once in each list of a scene.
- New meshes upload across frames under one byte budget a frame (`MESH_UPLOAD_BYTES_PER_FRAME`,
  shared by the five mesh caches and the silhouette cache), packing whole vertices and indices into
  buffers made at the start of the frame, so showing a large body never stalls. A mesh is drawn only
  once complete; while any mesh of a cache is still uploading, meshes the cache drew before that are
  no longer in the scene stay drawn (the old result of a recomputed body), at their last styles with
  pick ids withdrawn, so they hide what is behind them in the pick pass but pick nothing.
  `Renderer::is_uploading` says whether frames must follow (`app.md`); image export uploads whatever
  is left at once.
- The spread-over-frames upload can be stopped and resumed (`Renderer::stop_uploading`,
  `resume_uploading`; `UploadBudget::STOPPED`, which grants nothing and starts nothing). While
  stopped, a mesh, silhouette or new batch not on the graphics card is left out of the scene and
  is not started (a partial upload is kept for the resume), meshes already uploaded keep drawing,
  replaced meshes stay drawn as during an upload, `is_uploading` is false and
  `Renderer::is_upload_stopped` is true while anything is left out (`MeshPool::left_out`,
  `SilhouetteCache::left_out`, `batches_left_out`). The stop clears itself in the first frame that
  leaves nothing out, so a Cancel pressed as the last body goes up stops nothing, and a stopped
  scene that does not change draws no new view pass. Image export's unlimited budget uploads what
  was left out regardless, and an anchor move uploads every batch at once even when stopped.
- The ignored `frame_costs_of_drawing_a_large_scene` test times the UI thread's share of a frame
  (separately from GPU wait) for large and many-small-mesh scenes under idle, camera, restyle,
  batch-replace, new-mesh and zebra/see-through cases; `CADITOR_BENCH_CASE` runs only cases whose
  name contains it.
- A `MeshInstance` may carry a `placement` (`RigidTransform`) drawing the mesh moved and turned
  without a new upload: the placement uniform, rewritten only when placement or anchor moved, holds
  the turned axes and the placed centre relative to the anchor (f64), and `vs_mesh` applies them to
  positions and normals.
- A mesh whose placed bounds lie wholly beyond one side of the clip volume is not drawn
  (`culling::ClipWindow`, the eight placed corners against the clip planes in f64), against the
  window in the main pass, the pick window in the pick pass and each tile in image export. A batch
  is culled the same way (lines, hidden lines, markers, fills) by the bounds of its points worked
  out on upload (`BatchBounds`), but only against the four side planes widened by its reach (half
  its widest line or marker plus `STROKE_FRINGE_POINTS`, via `ClipWindow::sees_reaching`), since
  strokes reach past their points and front-layer depths are moved; a mark whose centre lies
  outside the pick window but whose disc covers the cursor still picks.
- A `ShadedMesh` either owns its vertices (`ShadedMesh::new` from `MeshFace`s) or reads them from a
  `MeshSource` it shares (`ShadedMesh::shared`): the source's triangles are its indices and each
  vertex is converted (position relative to the centre, normal, face) when uploaded or read, so a
  body's display mesh is held once on the CPU, by the kernel (`app.md`). `shared` refuses (`None`) a
  source whose faces do not each use their own contiguous run of vertices in face order covering
  every vertex; the caller then copies.
- `ShadedMesh::divide` makes another mesh splitting every face into one face per class a caller's
  classifier gives each triangle, with the source face and area of every piece (analysis
  colouring, since styles are per face); the classifier may fail, which ends the division with
  its error, so the app's analysis thread stops at its interrupt. `ShadedMesh::origin` and `face_triangles` let the app's
  reach analysis and `through.rs` work on triangles without a copy.
- Per-face styles live in an `Rgba32Uint` texture (`StyleLayout`: colour, pick id, packed finish, a
  spare word) read by face index in the vertex shader, so hover and selection cost nothing in
  geometry. A restyle writes only the texture rows whose styles changed, each as one span from its
  first to its last changed face (`changed_spans`), and keeps the slice it wrote, so handing the
  same slice again costs a pointer comparison; past `MAX_STYLE_SPANS` rows, or when the face count
  changes, the whole texture is written.
- Faces are lit two-sided and write depth, hiding edges and sketches behind them in view and
  picking alike (everything but `Layer::Front`). A face without a pick id writes id 0 with its
  depth in the pick pass, not discarded. The opaque mesh pass
  (`fs_mesh_solid`) discards the back faces of a closed mesh (`CAPPABLE`, `is_back_face`): the
  slope-scaled `FACE_DEPTH_BIAS` pushes a grazing front face back further than the steeper back
  face under it, so along a silhouette that back face won a pixel-wide band and showed its shadowed
  inside (a dark rim line in the rendered view). Open meshes, translucent and sectioned meshes keep
  both sides. Enhanced shading scales highlight and rim with the face
  colour's lightness so dimmed and tinted bodies stay dark and keep their hue.

## Colour and light

- Colours are sRGB-encoded everywhere (palettes, styles, `BACKGROUND`) and every pass draws on the
  plain 8-bit view, so blending stays in the space the palettes' contrast tests model
  (`scene_palette.rs`). Only `fs_mesh` works in linear light: it decodes the face colour
  (`to_linear`), lights it and encodes the result (`to_srgb`). Zebra and chrome keep their
  sRGB-space look.
- A face whose style carries a `SurfaceFinish` (metalness and roughness, 0 to 1, packed into the
  style texel's third word as two bytes and a present bit, `SurfaceFinish::packed`) is shaded by
  `finished` instead of `lit`: `vs_mesh` puts the word in the flat `sectioned` varying above its
  flag bits (`FINISH_SHIFT`), so it costs no varying, pass or pipeline, and a face without one
  takes exactly the old branch (an offscreen test pins its pixels). `finished` keeps the shading
  mode's diffuse lights (`standard_diffuse`, `enhanced_diffuse`) on the colour (a metal keeps
  `METAL_DIFFUSE` of it), adds a GGX highlight (Smith visibility, Schlick Fresnel) per light (key,
  and fill in Enhanced) capped by `HIGHLIGHT_LIMIT`, and reflects `studio` (the chrome reflection's
  world-Z-up sky, ground and panel in linear light, horizon and panel blurred by roughness,
  weighted by a roughness-aware Fresnel). Reflectance is 4% for a dielectric and the colour times
  `METAL_REFLECTANCE_GAIN` for a metal. Section caps, flat, reflective and pick shaders ignore the
  finish; translucent meshes, drawn with `fs_mesh`, take it.
- `Scene::lighting` (`lighting.rs`, `Lighting`: a `LightDirection` in the view's or the model's
  frame, colour, strength, environment, exposure in stops, shadows, bounce) switches faces carrying
  a finish to `rendered` (uniform fields `sun`, `sun_colour`, `exposure_shadow`, `bounce`,
  `shadow_matrix`, `relative_from_light`, after the section planes; `uses_rendered_shading` reads
  the exposure scale, zero without lighting, so every pixel of a scene without lighting is as
  before). `rendered` is Cook-Torrance: GGX, height-correlated Smith visibility, Schlick Fresnel
  with F0 mixed from 4% to the colour by metalness, Lambert diffuse weighted by one less Fresnel
  and metalness, times the sun and its shadow; the procedural `sky` (blue above, a sunlit ground's
  neutral below) as split-sum ambient (`sky_irradiance(normal)`, `sky_reflection(reflected,
  roughness)` with Karis' analytic environment BRDF) times `environment`, or the environment
  picture's in its place (`ambient_irradiance`, `ambient_reflection`, below); one bounce of
  sunlight; then exposure, Khronos PBR Neutral tone mapping and `to_srgb`. Faces without a finish
  (the app's hovered and selected faces in high contrast) keep `lit`. Section caps of a finished
  face are rendered without shadow or bounce.
- `LightDirection::View` turns with the camera's heading about Z (`behind_the_viewer`: the eye's
  side levelled, else the far side of the view's up when looking straight down), its height taken
  from the world horizon, never the camera's, so a view from above keeps the faces toward the
  viewer lit. `Model` is fixed with Z up.
- With lighting, every frame of the 3D view (and image tiles on the viewport background, not
  transparent ones) starts with the sky pass (`vs_sky`, `fs_sky`, layout `[view, -, light]`, no
  depth): a backdrop graded in screen space from `BACKDROP_TOP` to `BACKDROP_BOTTOM` (the
  tile's place in the whole image from the inverse tile transform), tone mapped, darkened where an
  unseen ground at the opaque meshes' lowest point (`Ground::under`, uniform `ground`: height and
  centre relative to the eye, radius) lies in the sun's shadow (`GROUND_SHADOW`, fading from
  `GROUND_FADE_FROM` to `GROUND_FADE_TO` radii). The light frame is fitted over the casters and
  their shadows on that ground (`Ground::shadows_of`, at most `LONGEST_GROUND_SHADOW` radii long,
  none with the sun under `LOWEST_SUN_FOR_GROUND_SHADOWS`). Per-pixel sky rays use `sky_view`
  (tangents of the half field of view) and the camera vectors, which the occlusion uniform carries
  whenever lighting is on.
- `Scene::backdrop` (`Backdrop`: an `Arc<Environment>`, a turn about Z in radians and a brightness
  in stops) replaces the procedural sky with a picture, only while `Scene::lighting` is set.
  `Environment::new` (`environment.rs`, off the render thread, CPU only) takes linear RGB of an
  equirectangular picture (width twice the height within 2%, at least
  `LEAST_ENVIRONMENT_WIDTH` across, else an `EnvironmentError`), makes every value finite and at
  most half-float range, shrinks it by area averaging to at most `MAX_ENVIRONMENT_WIDTH` and an
  even width, halves it down to one row for the mip chain and convolves a 64x32 copy with the
  cosine lobe into a 32x16 irradiance map (divided by the lobe's own total so a uniform picture
  lights every normal exactly with its radiance, and read as E/pi like `sky_irradiance`), both
  packed as `Rgba16Float` bytes with `half`. Identity is a counter, so scene comparison never
  walks the texels. The orientation is `direction_of`: the picture's middle lies along +X, u grows
  toward -Y (as seen from inside), v runs from +Z at the top to -Z; the turn adds to the heading.
  `LightPass::bind_environment` uploads it inside `gpu::scoped` into group 2 (bindings 5 and 6, a
  filtering sampler at 7 that repeats in u; 1x1 stand-ins until bound, `environment_textures.rs`),
  rebuilding the group, and a refusal or a picture wider than `max_texture_dimension_2d` reports
  `RenderFault::EnvironmentRefused` once and draws the procedural sky. The uniform's `environment`
  carries turn, gain (`2^stops`) and the last mip; a zero gain means no picture. The sky pass
  then draws the picture along each pixel's ray (the nominal field of view's perspective ray in an
  orthographic view too), at the mip the pixel's angular size picks, times the exposure and tone
  mapped, with the ground's shadow over it as before. `rendered` takes its diffuse from the
  irradiance map and its specular from the mip a surface's roughness picks (`sqrt(roughness)`
  across the chain, easing to the irradiance map from 0.5 to 1), times `environment`. Group 2
  therefore belongs to every pipeline that can reach `rendered`, section caps of flat and
  reflective meshes included (`lit_mesh_pipeline_layout`). A new picture, like any change the
  uniform bytes do not show, redraws the kept view (`EnvironmentBound::rebound`).
- The light pass (`LightPass`, `LightFrame`): when lighting has shadows or bounce, one depth pass
  from the sun (`vs_shadow`, `fs_shadow`, `shadow_pipeline`, layout `[view, mesh]`) draws every mesh
  of `Scene::meshes` (translucent ones receive but do not cast; faces of no alpha and geometry cut
  away by a section cast nothing) into a `SHADOW_MAP_SIDE` depth map plus two `Rgba8Unorm` targets,
  the lit diffuse albedo with the sun's cosine and the normal, and an `R32Uint` target holding the
  fragment's depth as f32 bits (`lit_depth`, without the pipeline's depth bias): a reflective shadow
  map. The depth map is read only through the comparison sampler; `bounced` reads `lit_depth`,
  since GLSL has no `textureLoad` of a depth texture. It is
  encoded before the scene pass whenever the kept view is drawn, and before each image tile.
  `LightFrame::fitted` fits an orthographic light frame to the placed corners of the meshes
  (`MeshCache::placed_corners`, f64, eye-relative), so it rides in the view uniform with its
  inverse. The colour mesh pipelines running `fs_mesh` (meshes, sectioned, translucent, overlay)
  bind the maps as group 2 (`LightPass::bind_group`, 1×1 stand-ins until made). Shadows are 3×3
  PCF on a comparison sampler with a normal offset; bounced light (`bounced`) gathers
  `BOUNCE_SAMPLES` texels on a spun golden-angle disc of `BOUNCE_REACH` of the frame around the
  receiver, each a lit patch's radiance over distance squared with both cosines. A sample facing
  the receiver is tested for visibility (`bounce_blocked`): the segment between the two, each end
  lifted along its normal by the shadow normal offset, is marched in light space at
  `BOUNCE_BLOCK_STEPS` points against `lit_depth` (texels holding 0 are empty), and a point behind
  the stored surface by more than `BOUNCE_BLOCK_LIFTS` lifts and less than the segment's length
  drops the sample. The upper bound tells a wall crossed from a shadow travelled through: a
  caster farther above than the segment is long shades the path without standing in it. Only
  surfaces the sun lights block, so an unlit wall still leaks. The targets are made lazily inside `gpu::scoped`; a
  refusal draws without shadows and bounce and reports `RenderFault::LightRefused` once. Image
  siblings make their own.
- Ambient occlusion (`occlusion.rs`, `Lighting::occlusion` above zero) is GTAO over what the view
  shows. Before the scene pass, three single-sampled passes over targets the size of the scene
  targets (`Occlusion`, made lazily in `gpu::scoped`, a refusal reporting
  `RenderFault::OcclusionRefused` once): `fs_surface` writes each opaque mesh's eye-relative
  position (three f32 bits) and normal (`packed_snorm`, WGSL's `pack4x8snorm` spelled out, which GLSL
  ES 3.0 lacks) into an `Rgba32Uint` target with its own depth (no float target, for GL); `fs_occlusion` (a full-screen triangle, `vs_screen`) integrates
  the cosine-weighted visible arc of `OCCLUSION_SLICES` slices of `OCCLUSION_STEPS` steps each way,
  horizons taken from those positions within a world reach of `REACH_OF_SCENE` of the opaque
  meshes' box (falloff over its last `OCCLUSION_FALLOFF`), at most `MOST_PIXELS` away on screen,
  spun per pixel by interleaved gradient noise; `fs_occlusion_blur` averages 4×4 neighbours of
  like depth and normal into an `R8Unorm` map. The uniform's `occlusion`, `occlusion_frame`
  (texel offset into the map, the pixel cap), `camera_right` and `camera_up` carry it. `rendered`
  reads the map at the fragment's pixel (group 3) only where the stored surface lies at the
  fragment's depth (`OCCLUSION_DEPTH_MATCH`; translucent faces and caps get none) and multiplies
  the sky diffuse and bounce by it and the sky reflection by Lagarde's specular occlusion;
  the sun is shadowed, not occluded. An image tile draws its occlusion over the tile widened by
  `MOST_PIXELS` each side (`margined_tile_transform`, a view uniform of its own) and reads it
  offset by that margin, so tiles meet without seams.
- The multisample resolve averages in linear light where views allow it: a renderer told so
  (`set_linear_resolve`, from `gpu::resolves_linearly`, the adapter's `VIEW_FORMATS`) gives its
  multisampled colour target and the kept view an sRGB view each, stores the scene pass instead of
  resolving it and resolves in an empty pass through the two sRGB views (`ColorAttachment`). The
  resolve lands in the kept view, never the surface. Image tiles on the viewport background do the
  same; a transparent image resolves in gamma, since its bands straighten premultiplied colour
  there. GL and other devices without view formats resolve in gamma, through the same empty pass
  over the plain views and never as the scene pass's own resolve target: wgpu's GL backend runs
  that resolve as a blit under the pass's scissor, which would resolve only the viewport's rect and
  leave the kept view transparent black outside it (offscreen tests run on GL in CI).

## Depth, buffers and layers

- Reverse-Z, infinite far plane, `Depth32Float`, multisampled at the level in use (never stored;
  colour resolves into the kept view and is copied onto the surface). The UI is drawn on the
  surface after the copy.
- Each batch has a `GpuBatch` of `GrowableBuffer`s, made only for what the batch holds (lines,
  markers, fills, style tables), found again each frame by its `Arc`'s address rather than position
  (`upload_batches`), so a batch kept while others come, go or change places uploads nothing and an
  app may hand over thousands (one per body and per sketch, `app.md`). A batch uploads only when
  new or when the anchor moved, so an idle frame or a camera move writes no vertices; a new one
  reuses the buffers of one that left. New batches upload under a byte budget a frame of their own
  (`MESH_UPLOAD_BYTES_PER_FRAME` again), at least one a frame however large; one left waiting is
  not drawn yet and keeps `Renderer::is_uploading` true. After the anchor moves every batch uploads
  at once. A batch past `max_buffer_size` draws only its first whole primitives (logged once). A
  buffer an upload outgrows is replaced by one a quarter larger than the upload (aligned), and one
  an upload fills to under a quarter by one fitting that upload the same way. Each pass sets a batch
  pipeline once for all batches it draws (`Once`).
- Vertex, index and silhouette records are packed one fixed-size `gpu::record` at a time straight
  into wgpu's staging memory (`gpu::write_records` over `Queue::write_buffer_with`), so no CPU copy
  of a batch or mesh outlives its upload; the `Bytes` staging left for uniforms and face styles
  drops any capacity past 64 KiB when cleared.
- Records are as narrow as the shaders allow: a mesh vertex is 20 bytes (`MESH_VERTEX_STRIDE`:
  position, normal, face), a silhouette 48, a line point 24, a marker 28, a fill vertex 16 (a
  position and the index of its fill's style). Every layer's depth biases are WGSL constants chosen
  from the layer bits of the flags (`line_depth_bias`, `marker_depth_bias`, `fill_depth_bias`).
  Normals are octahedral `Snorm16x2` (`Pack::octahedral`, `unfolded` in the shader); a zero or
  non-finite normal (which `facing_normal` replaces by the eye direction) is kept as the corner code
  (-1, -1) the shader reads as zero, and normals within about 0.02° of -Z take the opposite corner
  instead. Colours are `Unorm8x4` (`Pack::unorm8x4`, rounded), reaching the 8-bit target exactly.
- A batch's lines upload as points, not segments (`lines.rs`, `LineStrips`): consecutive lines of
  one style (colour, width, layer, stroke flags and kind) where one ends at the next one's start
  form a strip sharing its inner points, and a strip of three or more whose last end meets its first
  start is closed. A point record holds position, distance along, pick id and a style-index word
  with two bits: `DRAWN` (a segment starts here) and `LINKED` (a segment of the strip, drawn or not,
  starts here). Each instance reads four consecutive points through the one buffer bound at four
  offsets (`LINE_POINT_SLOTS`: previous, start, end, next); an instance whose start is not `DRAWN`
  collapses. Between a padding point at each end of the buffer, an open strip of n segments costs
  n + 1 points, a closed one n + 3 (its last segment's start before it and its second point after
  it, both undrawn), a lone segment two. Styles (colour, width, flags) sit in a small `Rgba32Uint`
  texture per batch (`StyleTable`, `styles.rs`, laid out by `StyleLayout`, bound as group 1 for the
  line pipelines); a batch with more styles than fit draws only the lines before the first that
  does not.
- A batch holds `Line`s (88 bytes a segment) and `PolylineRun`s (`runs.rs`): polylines read
  through a shared `Arc<dyn PolylineSource>` (index lists over positions, as `MeshSource` is for
  meshes), each with one `RunPolyline` style (polyline index, colour, width, pick, `StrokeKind`; 32
  bytes, `a_styled_polyline_is_far_smaller_than_the_lines_it_draws`), a layer and an optional
  placement. `Batch::all_lines` yields the lines, then each run's segments placed and with their
  distance along their own polyline; the upload, `BatchBounds`, `Scene::lines` and `hits_through`
  read that, so a run draws and picks exactly as its lines would
  (`a_run_over_shared_polylines_draws_and_picks_as_the_lines_it_stands_for`) and its segments exist
  only while a batch uploads. Runs compare by their source's address, so a kept part keeps its batch.
- A batch uploads its fills once, ordered pickable reference fills, then other pickable fills, then
  the rest, each group in batch order (`FillGroups`): the pick pass draws the first two groups as
  one range each of the colour pass's buffer, and the colour pass's spans keep batch order. Each
  fill has one texel (colour, pick id, flags; `fill_texel`) in a second `StyleTable` of the batch,
  in that order; its vertices carry only position and the fill's index, which `vs_fill` reads
  through `style_texel`; the colour and pick fill pipelines bind the table as group 1.
- A batch of at least `FEWEST_SEGMENTS_MADE_COARSER` shown segments also uploads up to four coarser
  levels of its shown lines after them in the same buffer (`coarser_lines`), each for a tolerance of
  one of `COARSER_LINE_SHARES` of the batch's size (`LineStrips::simplified`): a strip wholly within
  the tolerance of its first point is left out, every other one is thinned by Douglas–Peucker over
  its corners, keeping both ends and every corner where the pick id changes, so a coarser segment
  never spans two picks. A level is kept only while it keeps at most `COARSER_LINES_MOST_KEPT` of
  the previous level's segments. Each frame the colour pass draws, for shown lines and their hidden
  dashes, the coarsest level whose tolerance stays within `COARSER_LINES_ERROR_PIXELS` at the
  batch's nearest corner (`BatchBounds::pixels_across`, every segment when a corner is behind the
  near plane), worked out from the whole view so image tiles draw the level of the whole image; the
  pick pass always draws every segment. Most of a large assembly's segments belong to bodies a few
  dozen pixels across.
- Draw order: every batch's lines, then markers, then fills. Translucent fills sort back to front
  by centroid depth across all batches, front-layer fills last (`FillOrder`).
- Model geometry draws over reference geometry (datum planes, axes) through a per-`Layer` depth
  bias, model fills (sketch regions) over the faces they lie on.
- Faces carry a slope-scaled pipeline bias (`FACE_DEPTH_BIAS`, two slopes back in reverse-Z) so an
  edge beyond a face at a grazing angle is not eaten; reference fills (own `reference_fills`
  pipeline in the drawing and pick pipelines alike, chosen per draw by `FillDraw::behind_faces`) sit
  behind faces on their plane by a larger bias (`BEHIND_FACES_DEPTH_BIAS`) and the grid by
  `GRID_DEPTH_BIAS`, so a face on the XY plane never speckles with the grid or a principal plane.
- A line stroked `Stroke::DashedWhereHidden` (the hidden-edges style, threads) is drawn as usual,
  solid or dashed by `seen_dashed` (a solid one carries `SOLID_WHERE_SEEN` in its flags), and its own
  record is drawn again after every batch's lines with depth test `Less` and no depth write
  (`hidden_lines` pipeline, `vs_hidden_line`, always dashed), so the dashes show only where a nearer
  face covers it. The batch keeps the instance runs of its shown strips so stroked
  (`LineStrips::hidden_runs`), one draw each, so the scene holds no second copy of a line; the pick
  pass never draws the hidden part.
- `Layer::Front` draws over everything whatever its depth, in view and picking alike (the app puts
  the edited sketch there). `layered_depth` halves every depth into the far half of the range (an
  exact scaling) and moves front geometry into the near half, where biases stay wide enough for the
  coarser floats (fills under lines under markers).
- `Scene::flat_meshes` draw right after the opaque meshes with the same depth writes and pick pass
  but `fs_color`: each face shows its style's colour exactly, unlit (the hidden-line style).
- `Scene::reflective_meshes` draw right after the flat ones with the opaque depth writes and pick
  pass but `fs_reflective`, which reflects the eye ray about the normal in world space and shades
  by `Scene::reflection`: `Zebra` (a ring of `stripes` light and dark bands per turn around an axis,
  antialiased with the narrower `fwidth` of two angles whose seams differ, light band the face
  colour brightened, dark one nearly black) or `Chrome` (a procedural world-Z-up sky, horizon,
  ground and one light panel, tinted halfway to the face colour's hue so hover and selection show).
  The reflection rides in the view uniform's last two vectors (the stripe axes' orthonormal pair,
  the stripe count and a zebra flag).
- `Scene::silhouettes` (`silhouette.rs`) draw the outline of curved faces as seen from the current
  view, after every mesh and before the batches' lines, with the line pipeline's depth test and
  `fs_line`. A `Silhouette` names a mesh, a colour, a width, a dash flag, a `dashed_where_hidden`
  flag and a placement, with no faces drawn needed, so wireframe shows it too. One
  `dashed_where_hidden` is drawn again after the hidden lines (`hidden_silhouettes` pipeline:
  `vs_hidden_silhouette`, always dashed, depth `Less`, no depth write).
- Silhouettes never draw in the pick pass: an outline is not topology, has no stable name and
  slides over its face as the view turns, and the pixels inside it already pick its face (in
  wireframe, where no face picks, picking it would make curved faces selectable and flat ones not).
  On upload (under the mesh byte budget, in chunks that fit a buffer) every triangle whose corner
  normals differ (`ShadedMesh::curved_triangles`, counted when built, so flat faces cost nothing)
  becomes a `SILHOUETTE_STRIDE` (48-byte) instance of three positions and three octahedral normals;
  `vs_silhouette` finds where the facing of the interpolated normals toward the eye changes sign
  across the triangle and strokes that segment like a line, so the outline follows every orbit with
  no CPU work per frame. A dashed silhouette dashes along its dominant screen axis, since contour
  segments carry no distance along a curve.
- `Scene::overlay_meshes` draw right after the translucent ones, blended, with no depth test or
  write and never in the pick pass, so they show through whatever covers them (the cut preview).
- `Scene::translucent_meshes` draw after the opaque meshes and before lines with alpha blending and
  no depth write (`translucent_meshes` pipeline). The pick pass draws them with `fs_pick`, which
  discards faces without a pick id, so an unpickable see-through face (X-ray) never occludes or
  takes a pick while one carrying an id picks as usual.
- `fs_mesh` discards a face of no alpha, so a body with only some faces see-through is drawn as two
  instances of one mesh: in `meshes` with those faces at alpha 0 and in `translucent_meshes` with
  only those faces visible, both carrying every face's pick id.

## Reference pictures (`pictures.rs`)

- `Scene::pictures` are `ScenePicture`s: an `Arc<PictureTexture>` (RGBA8 in sRGB, as every colour
  here, shrunk by box averaging until no side passes `MAX_PICTURE_TEXTURE_SIDE` and carrying its
  whole mip chain, built off the UI thread by the app), the four corners (the picture's bottom
  left, bottom right, top right, top left, so a flip is the app's corner order), an alpha, whether
  it shows in front of bodies and an optional pick id.
- `PictureCache` uploads each texture once per `Arc` (from the first mip level within the device's
  largest texture side, inside `gpu::scoped`; a refusal counts as a refused mesh and the picture is
  left out alone), keeps a uniform and bind group per picture (corners relative to the anchor in
  f64, alpha, pick id; written only when they change, which joins `Prepared::changed`) and drops
  what leaves the scene. Image siblings make their own cache sharing the bind group layout.
- `vs_picture` draws a quad of six vertices from the uniform with the model fill depth bias
  (`MODEL_FILL_DEPTH_BIAS`), so a picture on a face shows over it, and `fs_picture` samples with
  linear mipmapped filtering, multiplying the texture's alpha. A picture behind bodies draws right
  after the opaque, flat and reflective meshes, depth-tested and writing no depth; one in front
  draws after the translucent and overlay meshes with no depth test. Both draw before silhouettes,
  lines and markers, so sketch curves always show over a picture, and neither is sectioned. A
  picture with a pick id is drawn in the reference pick pass (`fs_picture_pick`, writing its depth),
  so it owns a pixel only where nothing else is picked; one without draws nothing there.

## Sections

- `Scene::section` holds up to `MAX_SECTION_PLANES` `SectionPlane`s (a `Plane` and a `CutFace`,
  hatched or filled); each cuts away the side its normal points to, so together they keep what
  every plane keeps. It only changes what is drawn and picked, never a mesh or batch, so moving a
  plane costs a uniform write: the view uniform carries the count, a slack (`section_slack`, a
  fraction of the view distance, so a face lying on a plane is kept), each plane's normal and offset
  relative to the eye (f64) and each plane's hatch direction over its spacing with a phase anchored
  to the plane, the spacing `HATCH_SPACING_POINTS` at the target rounded up to a power of two so the
  hatch stays put while zooming a little.
- Every mesh and silhouette, and lines, markers and fills of `Layer::Model`
  (`Layer::is_sectioned`, a bit in the instance flags), are discarded beyond a plane in view and
  picking alike; reference and front geometry never are.
- While a section is shown the opaque, flat and reflective meshes draw with `fs_*_sectioned`
  pipelines (and `fs_mesh_pick_sectioned`), which cap the cut: a back face (flat normal from
  derivatives, oriented by the interpolated normal, turned from the eye) seen where the eye's ray
  crossed a plane is the inside of a cut solid. It is drawn as the plane there: shaded with the
  plane's normal in the face's colour darkened by `CAP_SHADE`, hatched (`HATCH_SHADE`) or filled from
  the point where the ray entered the kept region. Its depth stays the back face's, pulled forward by
  `CAP_DEPTH_BIAS` so the back faces' edges stay hidden behind it; moving it onto the plane would let
  it win over front faces between the plane and the back face. The pick pass writes id 0 there with
  the plane's depth, so a cap hides what lies behind it, picks nothing and gives the orbit pivot on
  the cut. A face of no alpha is discarded before any cap in both passes, so a see-through face
  leaves no hole in a cap that picks nothing; its own pick comes from the translucent instance.
  These shaders write `frag_depth`, so they take no pipeline depth bias and add the face slope bias
  themselves; plain pipelines draw when there is no section, keeping early depth testing. Only a
  closed mesh is capped (`ShadedMesh::is_closed`, `with_closed` clears it, set by the app from the
  solid's edges all having two uses; `vs_mesh` raises `CAPPABLE` in the instance flags), so the far
  side of an open sheet is drawn and picked as an ordinary face; translucent and overlay meshes are
  only cut, never capped.
- `Scene::hits_through` skips hits on sectioned geometry beyond a plane (`is_cut_away`), so the pick
  list, paint selection and Measure only reach what is shown. It also reproduces the cap: for the
  closed opaque, flat and reflective meshes (and faces with alpha), a back-facing triangle
  (`ShadedMesh::oriented_triangles`) beyond the ray's entry into the kept region (`Probe::entry`,
  measured from the eye plane in an orthographic view) is the cap, the nearest one setting
  `Probe::cap_depth`: no hit is recorded at or behind it, reference fills not at all, front-layer
  hits stay, as in the pick pass; an open mesh keeps its far side.
- `kept_span` gives the part of a segment the planes keep as a parameter interval, for callers that
  clip curves to what is shown (box selection of sketch curves, `sectioned_screen.rs`).

## Lines, markers and sizes

- Sizes are logical points: `ViewportFrame::pixels_per_point` goes into the view uniform and
  shaders scale line widths, marker diameters and the grid by it.
- Lines, markers, silhouettes and the grid are quads, one instance each, drawn indexed through one
  static index buffer (`gpu::QuadIndices`, bound once after the meshes in each pass).
- Lines and silhouettes are finished in the colour pass (`Strokes::Finished` in the view uniform's
  `fill_light.w`): each quad reaches `STROKE_FRINGE_PIXELS` beyond its edges and `fs_line` turns the
  distance from the segment (carried in screen space) into coverage, so lines are smooth at every
  multisampling level, Off included. An opaque line also reaches half its width past an end and
  rounds it; a translucent one keeps square ends where its strip ends. Where a segment joins its
  strip's neighbour (joint and both far ends ahead of the near plane, neither segment of no length
  on screen; colour pass only, `line_joins`), its quad reaches its full half-width past the joint
  and carries the neighbour's far end (the `joints` varying), and `fs_line` gives each pixel both
  strokes cover to one of them alone, by its side of the joint's bisector (`neighbour_owns`, the
  outgoing segment taking ties). The two meet in a round join with no notch and no pixel blended
  twice, translucent or not. Round rather than mitred joins keep translucent and opaque polylines
  alike, so a hover that changes alpha never reshapes a corner, and need no miter limit. The pick
  pass (`Strokes::Bare`) draws bare quads, so pick reach is unchanged.
- `Stroke::Dashed` carries the distance along the curve at its start, so dashes
  (`DASH_PERIOD_POINTS`) run on across a polyline's segments and joints at any zoom and interface
  size. Each corner's distance in points is led by `DASH_LEAD_POINTS`, a whole number of periods, so
  the clamp keeping it from going negative (a negative one reads as solid) never bends the pattern
  at a segment starting at distance 0. The pick pass draws dashed lines whole, so a gap still picks
  its curve.
- A marker or line whose colour has no alpha draws nothing but is still picked, so pickable points
  and edges can stay invisible until hovered or selected. Such ones upload after the drawn ones of
  their batch (`shown_first`, and for lines `LineStrips`, whose strips never join the two) and the
  colour pass draws only the drawn ones.
- Markers at one place in one layer have equal depths, which `GreaterEqual` passes, so they draw in
  batch order: a smaller unpicked marker after a larger one makes a ring that still picks whole (the
  app's hollow sketch points). `BACKGROUND`, the default canvas clear colour, is public so the app
  tests its scene colours against it; the app sets the clear colour of its light or dark view with
  `ViewportRenderer::set_background` (`Renderer::set_background`, kept across device recovery,
  copied to image siblings and used for `Background::Viewport` images), and the kept frame
  (`kept::Shown`) holds the background it was drawn on, so a change redraws it.

## Projection

- `camera::ProjectionMode` is what the user chooses and the `Camera` keeps: perspective,
  orthographic or automatic, which is perspective and turns orthographic while the viewpoint looks
  square at a coordinate axis (within `SQUARE_TOLERANCE_DEGREES`: the six axis views, not the
  isometric one). `Camera::view` resolves it against the current viewpoint, so a `View` only holds
  the effective `camera::Projection`. Orthographic shows at every depth the scale perspective shows
  at its target, so switching keeps the on-screen size. The eye stays `distance` in front of the
  target and the depth range is finite, centred on the target; `View::reaching` widens it to the
  scene's bounds so geometry behind the eye draws and the grid fades before the range ends.
- Orthographic depth is linear, so shaders turn a layer's depth bias into a fixed offset
  (`ORTHOGRAPHIC_DEPTH_BIAS`); a factor would push edges far through faces.
- Fitting puts the eight corners of the bounds inside the field of view with a margin, exactly in
  both projections (no bounding sphere); bounds of no size keep the distance and recentre. Grid
  spacing follows the view distance, not the eye's height (`grid_snap_spacing` gives the app the
  spacing of the finest level the shader draws in full, `app-sketching.md`).
- `View::unproject` gives nothing for a non-finite depth or result, so a NaN read back from a pick
  never becomes a hit; `Camera::orbit`, `pan` and `zoom` ignore non-finite pivots, anchors and
  drags.
- `View::plane_projection` folds the view's rotation, eye, field of view, size and a scale into an
  affine map of one plane's coordinates to the camera's frame (`PlaneProjection`), so projecting
  many points of a plane is cheap; a test holds it to `View::project` in both projections.

## Picking

- Renders a window of `PICK_RADIUS_POINTS` around the cursor (`PickWindow`, physical pixels from the
  scale). ID and depth targets are `R32Uint` (depth as f32 bits, since GL does not always render to
  float targets), read back asynchronously so hover never blocks the UI thread.
- Hits report their distance from the cursor in points (`offset_points`, for the app's pick
  tolerances) and their world position (orbit pivot, pan grab point, zoom anchor).
- `poll_pick` says `Pending`, `Ready` or `Failed` (failed readback, or a pick whose frame was
  dropped before `submit`); the app asks again after a failure. `is_pick_answered` polls the device
  without drawing and says whether `poll_pick` would answer, so a pick in flight needs no frames.
- Reference-layer pick fills (principal and datum planes) are drawn first in a pass of their own
  and everything else over them: a translucent plane owns a pixel only where no face, line, marker
  or model or front fill covers it, and a face seen through a plane is picked. Front-layer geometry
  is picked through any face, as it is drawn.
- `Scene::hits_through` (`through.rs`) answers on the CPU what lies under a pixel at every depth,
  for the app's pick list: faces of the opaque, flat and translucent meshes (a ray test per
  triangle after one against the placed bounds), pickable fills the ray meets, and pickable lines
  and markers within `PICK_RADIUS_POINTS`, each id once with its nearest offset. It lists
  front-layer geometry first, then the rest nearest first, then reference fills, following what the
  pick pass would let win; overlay meshes are never listed.

## Image export (`image.rs`)

- `Renderer::render_image` draws an `ImageRequest` offscreen, independent of the window, at most
  `MAX_IMAGE_SIDE` a side in tiles of `TILE_SIDE` (or the largest texture side). Each tile writes
  the view uniform with `image::tile_transform`, the same clip-space transform the pick window uses,
  so line widths and grid fades stay those of the whole image; one submit per tile, since uniform
  writes land at the next submit.
- Memory is a small multiple of one band (a row of tiles), never the image: `ImageTiles` (kept by
  the `Renderer`) draws tiles in rows from the top into at most `READBACK_BUFFERS` readback buffers,
  and `ImageBands` (`Send`, for the writer's thread) waits for each tile by polling the device every
  millisecond (never wgpu's blocking wait, which on GL holds the context the UI thread draws with),
  copies it into the band, returns the buffer and wakes the app, whose next frame's `advance_image`
  draws the next tiles. A finished band is straightened and handed out whole rows at a time. Errors
  after the start reach the bands in order; dropping the bands stops the tiles at the next frame.
- The tiles draw with their own `ViewportRenderer` (`image_sibling`): the window's pipelines,
  layouts and uploaded mesh buffers shared, its own uniforms, batches and face styles, so the
  window's frames between tiles neither disturb the image nor upload anything again. When the
  surface is neither `Rgba8Unorm` nor `Bgra8Unorm` a throwaway `Rgba8Unorm` one is built. Output is
  written unconverted, as on screen, so pixels are sRGB.
- Errors are `ImageError` (`Busy` while another image's tiles are still drawing, `OutOfMemory`,
  `Refused`, `DeviceLost`, `Readback`, `Abandoned`). Nothing on the UI thread waits on the GPU, and
  the bands turn the premultiplied colour of a transparent clear into straight alpha.
- `OffscreenRenderer` opens a device without a surface and draws the same `ImageRequest` into an
  `Rgba8Unorm` target, its `ImageBands` drawing the tiles inline as it reads them (`drawn_inline`):
  the headless `--export` of a PNG (`app.md`) streams them to the file. Both draw a scene's
  pictures behind or in front of its bodies as the window does
  (`a_window_less_renderer_draws_a_scene_s_pictures_behind_bodies_or_in_front_of_them`).

## Navigation

- Right-drag orbits (turntable around world Z, stopping at the poles; a rolled view turns level),
  middle-drag or Shift+right-drag pans, wheel and pinch zoom toward the point under the cursor.
  Preferences › Navigation can zoom toward the view's centre instead (`ZoomToward`), orbit about
  the view's centre rather than the point under the pointer (`OrbitAround`; the app passes no
  pivot) and turn either orbit axis round (`invert_orbit_x`, `invert_orbit_y`, on pointer and cube
  drags, not the orbit keys) (`viewport::orbit_by`).
  View cube and fit changes animate. While an orbit runs, and for `ORBIT_MARK_LINGER` after, the
  app rings its pivot (`ViewportState::orbit_mark`, `canvas_guidance::paint_pivot` in the palette's
  `snap` over an `outline` halo); the renderer draws nothing for it.
- The moves are `Viewpoint::orbited`, `panned` and `zoomed` (nothing for non-finite input), so the
  app can move the viewpoint the camera is heading to rather than the one shown. `Camera::orbit`,
  `pan` and `zoom` apply them to the shown viewpoint and end any transition; `animate_to` eases in
  and out over `TRANSITION_DURATION`, and `glide_to` eases out only over the shorter
  `GLIDE_DURATION`, so a key held down, each repeat gliding on from the last destination, follows
  smoothly instead of restarting from rest. Both durations are multiplied by
  `Camera::set_transition_scale` (the View changes preference); a scale of zero lands every
  transition at once, one under way included.
- The vertical field of view is `DEFAULT_FIELD_OF_VIEW_DEGREES` (30°) unless `Camera::set_field_of_view`
  sets another, clamped to 10° to 90°; `Camera::view_from` carries it into each `View`
  (`View::with_field_of_view`), whose projection, rays, pixel sizes, plane projections, fitting and
  the orthographic scale all read it.
- `Camera::destination_view` and `view_from` build a `View` for the destination or any viewpoint
  with the projection resolved against that viewpoint, so a fit started during a turn into a
  standard view is fitted in the projection it will end in.
- The input mode (`preferences::InputMode`, Preferences › Navigation) is caditor (the mouse scheme
  above) or Laptop (touchpad): two-finger scroll orbits, Alt and scroll pans (egui turns Shift and
  scroll into horizontal-only scrolling, so Alt keeps both axes), pinch and Ctrl+scroll zoom,
  Alt-drag orbits and Shift+Alt-drag pans, the Alt press never starting a selection. Alt pressed
  once a primary drag has begun leaves the drag to it: in the sketch Alt holds the snap
  (`app-sketching.md`), in a model box, lasso or paint drag it reaches through (`app-input.md`). Fusion 360 (middle-drag pans, Shift+middle-drag orbits), FreeCAD (CAD style:
  middle-drag pans, middle held with left or right orbits), Blender (middle-drag orbits,
  Shift+middle-drag pans, Ctrl+middle-drag zooms) and SolidWorks (middle-drag orbits,
  Ctrl+middle-drag pans, Shift+middle-drag zooms) follow those programs; every mode keeps
  right-drag orbiting and Shift+right-drag panning, the wheel zooms, and a primary drag with the
  middle button held never starts a selection (`viewport::drag_motion`). Invert zoom turns the wheel
  and the Blender and SolidWorks zoom drags round alike (`viewport::zoom_factor`). A middle-button double-click
  fits the view in every mode. The canvas hint at the bottom right is
  `InputMode::navigation_hint`.
