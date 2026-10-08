use std::{ops::Range, sync::Arc};

use caditor_geometry::{Plane, Point2, Point3, Vector3};
use glam::{DVec2, Vec3};

use crate::{
    SurfaceSize,
    camera::{Projection, View},
    culling::ClipWindow,
    gpu::{self, Bytes, GrowableBuffer},
    image::{self, Background, ChannelOrder, ImageRequest, Tile},
    mesh::{MESH_VERTEX_STRIDE, MeshCache},
    picking::{self, PickPrepared, PickTargets, PickWindow, Picking},
    scene::{Batch, Fill, Grid, Layer, PickId, Primitive, Scene, ViewportRect},
    settings::Shading,
};

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub const BACKGROUND: wgpu::Color = wgpu::Color {
    r: 0.105,
    g: 0.11,
    b: 0.12,
    a: 1.0,
};
const FAR_DEPTH: f32 = 0.0;
const QUAD_VERTICES: u32 = 6;
const FACE_DEPTH_BIAS: wgpu::DepthBiasState = wgpu::DepthBiasState {
    constant: 0,
    slope_scale: -2.0,
    clamp: 0.0,
};
const BEHIND_FACES_DEPTH_BIAS: wgpu::DepthBiasState = wgpu::DepthBiasState {
    constant: 0,
    slope_scale: -4.0,
    clamp: 0.0,
};
const LINE_STRIDE: u64 = 60;
const MARKER_STRIDE: u64 = 44;
const FILL_VERTEX_STRIDE: u64 = 40;
const FILL_TRIANGLE_STRIDE: u64 = FILL_VERTEX_STRIDE * 3;
const VIEW_UNIFORM_SIZE: u64 = 160;
const REANCHOR_DISTANCES: f64 = 4.0;
const ANCHOR_ERROR_PIXELS: f64 = 0.02;
const F32_ROUNDING: f64 = f32::EPSILON as f64 / 2.0;
const KEY_LIGHT_UP: f64 = 0.8;
const KEY_LIGHT_LEFT: f64 = 0.5;
const FILL_LIGHT_DOWN: f64 = 0.35;
const FILL_LIGHT_RIGHT: f64 = 0.8;
const GRID_UNIFORM_SIZE: u64 = 64;
const GRID_CELLS_ACROSS_SCALE: f64 = 100.0;
const GRID_EXTENT_PER_SCALE: f64 = 40.0;
const GRID_MIN_SCALE_PER_DISTANCE: f64 = 0.25;
const WHOLE_VIEW: [f32; 4] = [1.0, 1.0, 0.0, 0.0];

pub struct ViewportFrame<'a> {
    pub rect: ViewportRect,
    pub view: &'a View,
    pub scene: &'a Scene,
    pub pick_at: Option<DVec2>,
    pub pixels_per_point: f32,
}

pub struct SurfaceTarget<'a> {
    pub view: &'a wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

struct Uniform {
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl Uniform {
    fn new(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        label: &'static str,
        size: u64,
    ) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        Self { buffer, bind_group }
    }
}

#[derive(Clone)]
struct Pipelines {
    meshes: wgpu::RenderPipeline,
    translucent_meshes: wgpu::RenderPipeline,
    overlay_meshes: wgpu::RenderPipeline,
    flat_meshes: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    markers: wgpu::RenderPipeline,
    fills: wgpu::RenderPipeline,
    reference_fills: wgpu::RenderPipeline,
    grid: wgpu::RenderPipeline,
    pick_lines: wgpu::RenderPipeline,
    pick_markers: wgpu::RenderPipeline,
    pick_fills: wgpu::RenderPipeline,
    pick_reference_fills: wgpu::RenderPipeline,
    pick_meshes: wgpu::RenderPipeline,
    pick_translucent_meshes: wgpu::RenderPipeline,
}

pub struct ImagePlan {
    size: SurfaceSize,
    order: ChannelOrder,
    view: View,
    anchor: Point3,
    pixels_per_point: f32,
    clear: wgpu::Color,
    grid: bool,
    targets: ImageTargets,
}

impl ImagePlan {
    pub fn order(&self) -> ChannelOrder {
        self.order
    }
}

struct ImageTargets {
    multisampled: Option<wgpu::TextureView>,
    resolved: wgpu::Texture,
    resolved_view: wgpu::TextureView,
    depth: wgpu::TextureView,
}

impl ImageTargets {
    fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        sample_count: u32,
        extent: wgpu::Extent3d,
    ) -> Self {
        let texture = |label, format, sample_count, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent,
                mip_level_count: 1,
                sample_count,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let view = |texture: &wgpu::Texture| texture.create_view(&Default::default());
        let resolved = texture(
            "image tile",
            format,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        Self {
            multisampled: (sample_count > 1).then(|| {
                view(&texture(
                    "multisampled image tile",
                    format,
                    sample_count,
                    wgpu::TextureUsages::RENDER_ATTACHMENT,
                ))
            }),
            resolved_view: view(&resolved),
            resolved,
            depth: view(&texture(
                "image tile depth",
                DEPTH_FORMAT,
                sample_count,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            )),
        }
    }

    fn begin_pass<'a>(
        &self,
        encoder: &'a mut wgpu::CommandEncoder,
        clear: wgpu::Color,
    ) -> wgpu::RenderPass<'a> {
        let (view, resolve_target, store) = match &self.multisampled {
            Some(multisampled) => (
                multisampled,
                Some(&self.resolved_view),
                wgpu::StoreOp::Discard,
            ),
            None => (&self.resolved_view, None, wgpu::StoreOp::Store),
        };
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("image tile"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear),
                    store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(FAR_DEPTH),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        })
    }
}

struct SceneTargets {
    width: u32,
    height: u32,
    multisampled_color: Option<wgpu::TextureView>,
    depth: wgpu::TextureView,
}

impl SceneTargets {
    fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        sample_count: u32,
        width: u32,
        height: u32,
    ) -> Self {
        let texture = |label, format| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        Self {
            width,
            height,
            multisampled_color: (sample_count > 1)
                .then(|| texture("multisampled viewport color", format)),
            depth: texture("viewport depth", DEPTH_FORMAT),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Faults {
    pub targets: bool,
    pub meshes: u32,
    pub batches: u32,
    pub picking: bool,
}

impl Faults {
    pub fn any(self) -> bool {
        self.targets || self.meshes > 0 || self.batches > 0 || self.picking
    }
}

#[derive(Debug, Clone, PartialEq)]
struct FillSpan {
    slot: usize,
    vertices: Range<u32>,
    centroid: Option<Point3>,
    in_front: bool,
    behind_faces: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Facing {
    eye: Point3,
    forward: Vector3,
}

impl Facing {
    fn of(view: &View) -> Self {
        Self {
            eye: view.eye(),
            forward: view.forward(),
        }
    }

    fn depth(self, point: Point3) -> f64 {
        (point - self.eye).dot(self.forward)
    }
}

#[derive(Default)]
struct FillOrder {
    sorted_for: Option<Facing>,
    draws: Vec<FillDraw>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FillDraw {
    slot: usize,
    vertices: Range<u32>,
    behind_faces: bool,
}

struct GpuBatch {
    shown: Option<Arc<Batch>>,
    anchor: Point3,
    lines: GrowableBuffer,
    markers: GrowableBuffer,
    fills: GrowableBuffer,
    pick_fills: GrowableBuffer,
    line_count: u32,
    marker_count: u32,
    fill_vertices: u32,
    fill_spans: Vec<FillSpan>,
    reference_pick_vertices: u32,
    nearer_pick_vertices: u32,
}

impl GpuBatch {
    fn new(device: &wgpu::Device) -> Self {
        Self {
            shown: None,
            anchor: Point3::ZERO,
            lines: GrowableBuffer::new(device, "lines", wgpu::BufferUsages::VERTEX),
            markers: GrowableBuffer::new(device, "markers", wgpu::BufferUsages::VERTEX),
            fills: GrowableBuffer::new(device, "fills", wgpu::BufferUsages::VERTEX),
            pick_fills: GrowableBuffer::new(device, "pick fills", wgpu::BufferUsages::VERTEX),
            line_count: 0,
            marker_count: 0,
            fill_vertices: 0,
            fill_spans: Vec::new(),
            reference_pick_vertices: 0,
            nearer_pick_vertices: 0,
        }
    }

    fn refused(device: &wgpu::Device, batch: &Arc<Batch>, anchor: Point3) -> Self {
        Self {
            shown: Some(Arc::clone(batch)),
            anchor,
            ..Self::new(device)
        }
    }

    fn holds(&self, batch: &Arc<Batch>, anchor: Point3) -> bool {
        self.anchor == anchor
            && self
                .shown
                .as_ref()
                .is_some_and(|shown| Arc::ptr_eq(shown, batch))
    }

    fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        staging: &mut Bytes,
        uploaded: Uploaded<'_>,
    ) {
        let Uploaded {
            batch,
            anchor,
            slot,
        } = uploaded;
        staging.clear();
        for line in &batch.lines {
            staging
                .vec3(relative_to_eye(line.start, anchor))
                .vec3(relative_to_eye(line.end, anchor))
                .floats(&line.color.to_array())
                .f32(line.width)
                .u32(PickId::raw(line.pick))
                .f32(line.layer.depth_bias(Primitive::Line))
                .f32(line.stroke.along())
                .u32(u32::from(line.layer.draws_in_front()));
        }
        self.line_count = count(self.lines.upload(device, queue, staging, LINE_STRIDE));

        staging.clear();
        for marker in &batch.markers {
            staging
                .vec3(relative_to_eye(marker.position, anchor))
                .floats(&marker.color.to_array())
                .f32(marker.diameter)
                .u32(PickId::raw(marker.pick))
                .f32(marker.layer.depth_bias(Primitive::Marker))
                .u32(u32::from(marker.layer.draws_in_front()));
        }
        self.marker_count = count(self.markers.upload(device, queue, staging, MARKER_STRIDE));

        staging.clear();
        let mut written = 0u32;
        let mut spans = Vec::with_capacity(batch.fills.len());
        for fill in &batch.fills {
            let start = written;
            written = written.saturating_add(stage_fill(staging, fill, anchor));
            spans.push(FillSpan {
                slot,
                vertices: start..written,
                centroid: fill.centroid(),
                in_front: fill.layer.draws_in_front(),
                behind_faces: fill.layer == Layer::Reference,
            });
        }
        self.fill_vertices = count(
            self.fills
                .upload(device, queue, staging, FILL_TRIANGLE_STRIDE)
                .saturating_mul(3),
        );
        let uploaded = self.fill_vertices;
        self.fill_spans = spans
            .into_iter()
            .map(|span| FillSpan {
                vertices: span.vertices.start.min(uploaded)..span.vertices.end.min(uploaded),
                ..span
            })
            .filter(|span| !span.vertices.is_empty())
            .collect();

        self.upload_pick_fills(device, queue, staging, &batch.fills, anchor);
        self.shown = Some(Arc::clone(batch));
        self.anchor = anchor;
    }

    fn upload_pick_fills(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        staging: &mut Bytes,
        fills: &[Fill],
        anchor: Point3,
    ) {
        staging.clear();
        let pickable = fills.iter().filter(|fill| fill.pick.is_some());
        let (reference, nearer): (Vec<&Fill>, Vec<&Fill>) =
            pickable.partition(|fill| fill.layer == Layer::Reference);
        let mut stage = |fills: Vec<&Fill>| {
            fills.into_iter().fold(0u32, |written, fill| {
                written.saturating_add(stage_fill(staging, fill, anchor))
            })
        };
        let reference_written = stage(reference);
        let nearer_written = stage(nearer);

        let uploaded = count(
            self.pick_fills
                .upload(device, queue, staging, FILL_TRIANGLE_STRIDE)
                .saturating_mul(3),
        );
        self.reference_pick_vertices = reference_written.min(uploaded);
        self.nearer_pick_vertices =
            nearer_written.min(uploaded.saturating_sub(self.reference_pick_vertices));
    }

    fn draw_lines(&self, pass: &mut wgpu::RenderPass<'_>, pipeline: &wgpu::RenderPipeline) {
        if self.line_count == 0 {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_vertex_buffer(
            0,
            self.lines.slice(u64::from(self.line_count) * LINE_STRIDE),
        );
        pass.draw(0..QUAD_VERTICES, 0..self.line_count);
    }

    fn draw_markers(&self, pass: &mut wgpu::RenderPass<'_>, pipeline: &wgpu::RenderPipeline) {
        if self.marker_count == 0 {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_vertex_buffer(
            0,
            self.markers
                .slice(u64::from(self.marker_count) * MARKER_STRIDE),
        );
        pass.draw(0..QUAD_VERTICES, 0..self.marker_count);
    }

    fn bind_fills(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_vertex_buffer(
            0,
            self.fills
                .slice(u64::from(self.fill_vertices) * FILL_VERTEX_STRIDE),
        );
    }

    fn draw_pick_fills(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipeline: &wgpu::RenderPipeline,
        vertices: Range<u32>,
    ) {
        if vertices.is_empty() {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_vertex_buffer(
            0,
            self.pick_fills
                .slice(u64::from(vertices.end) * FILL_VERTEX_STRIDE),
        );
        pass.draw(vertices, 0..1);
    }

    fn reference_pick_fills(&self) -> Range<u32> {
        0..self.reference_pick_vertices
    }

    fn nearer_pick_fills(&self) -> Range<u32> {
        self.reference_pick_vertices
            ..self
                .reference_pick_vertices
                .saturating_add(self.nearer_pick_vertices)
    }
}

struct Uploaded<'a> {
    batch: &'a Arc<Batch>,
    anchor: Point3,
    slot: usize,
}

pub struct ViewportRenderer {
    format: wgpu::TextureFormat,
    sample_count: u32,
    shading: Shading,
    view_layout: wgpu::BindGroupLayout,
    grid_layout: wgpu::BindGroupLayout,
    pipelines: Pipelines,
    view_uniform: Uniform,
    pick_view_uniform: Uniform,
    grid_uniform: Uniform,
    batches: Vec<GpuBatch>,
    anchor: Option<Point3>,
    fill_order: FillOrder,
    #[cfg(test)]
    work: Work,
    meshes: MeshCache,
    translucent: MeshCache,
    overlay: MeshCache,
    flat: MeshCache,
    staging: Bytes,
    targets: Option<SceneTargets>,
    targets_refused: Option<(u32, u32)>,
    picking: Picking,
    pick_refused: bool,
    pick_window: Option<ClipWindow>,
}

impl ViewportRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, sample_count: u32) -> Self {
        let uniform_layout = |label| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            })
        };
        let view_layout = uniform_layout("view uniform");
        let grid_layout = uniform_layout("grid uniform");
        let meshes = MeshCache::new(device);
        let layouts = Layouts {
            view: &view_layout,
            grid: &grid_layout,
            mesh: meshes.layout(),
        };

        Self {
            format,
            sample_count,
            shading: Shading::default(),
            pipelines: Pipelines::new(device, format, sample_count, &layouts),
            view_uniform: Uniform::new(device, &view_layout, "view", VIEW_UNIFORM_SIZE),
            pick_view_uniform: Uniform::new(device, &view_layout, "pick view", VIEW_UNIFORM_SIZE),
            grid_uniform: Uniform::new(device, &grid_layout, "grid", GRID_UNIFORM_SIZE),
            view_layout,
            grid_layout,
            batches: Vec::new(),
            anchor: None,
            fill_order: FillOrder::default(),
            #[cfg(test)]
            work: Work::default(),
            meshes,
            translucent: MeshCache::new(device),
            overlay: MeshCache::new(device),
            flat: MeshCache::new(device),
            staging: Bytes::default(),
            targets: None,
            targets_refused: None,
            picking: Picking::new(device, DEPTH_FORMAT),
            pick_refused: false,
            pick_window: None,
        }
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    #[cfg(test)]
    pub fn sample_count(&self) -> u32 {
        self.sample_count
    }

    pub fn set_sample_count(&mut self, device: &wgpu::Device, sample_count: u32) {
        if sample_count == self.sample_count {
            return;
        }
        let layouts = Layouts {
            view: &self.view_layout,
            grid: &self.grid_layout,
            mesh: self.meshes.layout(),
        };
        self.pipelines = Pipelines::new(device, self.format, sample_count, &layouts);
        self.sample_count = sample_count;
        self.targets = None;
        self.targets_refused = None;
    }

    pub fn set_shading(&mut self, shading: Shading) {
        self.shading = shading;
    }

    pub fn picking(&mut self) -> &mut Picking {
        &mut self.picking
    }

    pub fn is_pick_pending(&self) -> bool {
        self.picking.is_pending()
    }

    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        surface: &SurfaceTarget<'_>,
        viewport: Option<&ViewportFrame<'_>>,
    ) -> Faults {
        let mut faults = Faults {
            targets: self.ensure_targets(device, surface.width, surface.height),
            ..Faults::default()
        };
        let viewport =
            viewport.filter(|viewport| viewport.rect.width >= 1.0 && viewport.rect.height >= 1.0);
        if let Some(viewport) = viewport {
            let uploaded = self.upload(device, queue, viewport);
            faults.meshes = uploaded.meshes;
            faults.batches = uploaded.batches;
            faults.picking = uploaded.picking;
        }

        let Some(targets) = self.targets.as_ref() else {
            clear_surface(encoder, surface);
            return faults;
        };
        let (color_view, resolve_target, store) = match &targets.multisampled_color {
            Some(multisampled) => (multisampled, Some(surface.view), wgpu::StoreOp::Discard),
            None => (surface.view, None, wgpu::StoreOp::Store),
        };

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("viewport"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color_view,
                depth_slice: None,
                resolve_target,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(BACKGROUND),
                    store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &targets.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(FAR_DEPTH),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        let Some(viewport) = viewport else {
            return faults;
        };
        let Some(scissor) = scissor_rect(viewport.rect, surface.width, surface.height) else {
            return faults;
        };
        pass.set_viewport(
            viewport.rect.x,
            viewport.rect.y,
            viewport.rect.width,
            viewport.rect.height,
            0.0,
            1.0,
        );
        pass.set_scissor_rect(scissor.0, scissor.1, scissor.2, scissor.3);
        self.draw_scene(
            &mut pass,
            viewport.scene.grid.is_some(),
            &ClipWindow::new(viewport.view, WHOLE_VIEW),
        );
        drop(pass);

        if let Some(cursor) = viewport.pick_at {
            self.draw_pick(encoder, viewport.view, cursor);
        }
        faults
    }

    pub fn image_sibling(&self, device: &wgpu::Device) -> Self {
        Self {
            format: self.format,
            sample_count: self.sample_count,
            shading: self.shading,
            view_layout: self.view_layout.clone(),
            grid_layout: self.grid_layout.clone(),
            pipelines: self.pipelines.clone(),
            view_uniform: Uniform::new(device, &self.view_layout, "view", VIEW_UNIFORM_SIZE),
            pick_view_uniform: Uniform::new(
                device,
                &self.view_layout,
                "pick view",
                VIEW_UNIFORM_SIZE,
            ),
            grid_uniform: Uniform::new(device, &self.grid_layout, "grid", GRID_UNIFORM_SIZE),
            batches: Vec::new(),
            anchor: None,
            fill_order: FillOrder::default(),
            #[cfg(test)]
            work: Work::default(),
            meshes: self.meshes.sibling(device),
            translucent: self.translucent.sibling(device),
            overlay: self.overlay.sibling(device),
            flat: self.flat.sibling(device),
            staging: Bytes::default(),
            targets: None,
            targets_refused: None,
            picking: Picking::new(device, DEPTH_FORMAT),
            pick_refused: false,
            pick_window: None,
        }
    }

    pub fn prepare_image(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        request: &ImageRequest<'_>,
        tile_side: u32,
    ) -> Option<ImagePlan> {
        let order = ChannelOrder::of(self.format)?;
        let size = request.size;
        let frame = ViewportFrame {
            rect: ViewportRect {
                x: 0.0,
                y: 0.0,
                width: size.width as f32,
                height: size.height as f32,
            },
            view: request.view,
            scene: request.scene,
            pick_at: None,
            pixels_per_point: request.pixels_per_point,
        };
        if self.upload(device, queue, &frame).any() {
            return None;
        }
        let targets = ImageTargets::new(
            device,
            self.format,
            self.sample_count,
            wgpu::Extent3d {
                width: tile_side.min(size.width),
                height: tile_side.min(size.height),
                depth_or_array_layers: 1,
            },
        );
        Some(ImagePlan {
            size,
            order,
            view: *request.view,
            anchor: self.anchor.unwrap_or_else(|| request.view.eye()),
            pixels_per_point: valid_scale(request.pixels_per_point),
            clear: match request.background {
                Background::Viewport => BACKGROUND,
                Background::Transparent => wgpu::Color::TRANSPARENT,
            },
            grid: request.scene.grid.is_some(),
            targets,
        })
    }

    pub fn draw_tile(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        plan: &ImagePlan,
        tile: Tile,
        readback: &wgpu::Buffer,
    ) {
        let transform = image::tile_transform(tile, plan.size);
        view_uniform(
            &mut self.staging,
            &AnchoredView {
                view: &plan.view,
                anchor: plan.anchor,
            },
            plan.pixels_per_point,
            self.shading,
            transform,
        );
        queue.write_buffer(&self.view_uniform.buffer, 0, self.staging.as_slice());

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("image tile"),
        });
        let mut pass = plan.targets.begin_pass(&mut encoder, plan.clear);
        pass.set_viewport(0.0, 0.0, tile.width as f32, tile.height as f32, 0.0, 1.0);
        pass.set_scissor_rect(0, 0, tile.width, tile.height);
        self.draw_scene(
            &mut pass,
            plan.grid,
            &ClipWindow::new(&plan.view, transform),
        );
        drop(pass);
        encoder.copy_texture_to_buffer(
            plan.targets.resolved.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(tile.row_pitch()),
                    rows_per_image: Some(tile.height),
                },
            },
            wgpu::Extent3d {
                width: tile.width,
                height: tile.height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
    }

    fn draw_scene(&self, pass: &mut wgpu::RenderPass<'_>, grid: bool, window: &ClipWindow) {
        pass.set_bind_group(0, &self.view_uniform.bind_group, &[]);
        self.meshes.draw(pass, &self.pipelines.meshes, window);
        self.flat.draw(pass, &self.pipelines.flat_meshes, window);
        self.translucent
            .draw(pass, &self.pipelines.translucent_meshes, window);
        self.overlay
            .draw(pass, &self.pipelines.overlay_meshes, window);
        for batch in &self.batches {
            batch.draw_lines(pass, &self.pipelines.lines);
        }
        for batch in &self.batches {
            batch.draw_markers(pass, &self.pipelines.markers);
        }
        if grid {
            pass.set_pipeline(&self.pipelines.grid);
            pass.set_bind_group(1, &self.grid_uniform.bind_group, &[]);
            pass.draw(0..QUAD_VERTICES, 0..1);
        }
        self.draw_fills(pass);
    }

    fn draw_fills(&self, pass: &mut wgpu::RenderPass<'_>) {
        let mut bound = None;
        let mut behind = None;
        for draw in &self.fill_order.draws {
            let Some(batch) = self.batches.get(draw.slot) else {
                continue;
            };
            if behind != Some(draw.behind_faces) {
                pass.set_pipeline(if draw.behind_faces {
                    &self.pipelines.reference_fills
                } else {
                    &self.pipelines.fills
                });
                behind = Some(draw.behind_faces);
            }
            if bound != Some(draw.slot) {
                batch.bind_fills(pass);
                bound = Some(draw.slot);
            }
            pass.draw(draw.vertices.clone(), 0..1);
        }
    }

    fn draw_pick(&mut self, encoder: &mut wgpu::CommandEncoder, view: &View, cursor: DVec2) {
        let (Some(targets), Some(window)) = (self.picking.prepared(), self.pick_window) else {
            return;
        };
        let mut behind = begin_pick_pass(encoder, targets, "pick reference fills", true);
        behind.set_bind_group(0, &self.pick_view_uniform.bind_group, &[]);
        for batch in &self.batches {
            batch.draw_pick_fills(
                &mut behind,
                &self.pipelines.pick_reference_fills,
                batch.reference_pick_fills(),
            );
        }
        drop(behind);
        let mut pass = begin_pick_pass(encoder, targets, "pick", false);
        pass.set_bind_group(0, &self.pick_view_uniform.bind_group, &[]);
        self.meshes
            .draw(&mut pass, &self.pipelines.pick_meshes, &window);
        self.flat
            .draw(&mut pass, &self.pipelines.pick_meshes, &window);
        self.translucent
            .draw(&mut pass, &self.pipelines.pick_translucent_meshes, &window);
        for batch in &self.batches {
            batch.draw_pick_fills(
                &mut pass,
                &self.pipelines.pick_fills,
                batch.nearer_pick_fills(),
            );
        }
        for batch in &self.batches {
            batch.draw_lines(&mut pass, &self.pipelines.pick_lines);
        }
        for batch in &self.batches {
            batch.draw_markers(&mut pass, &self.pipelines.pick_markers);
        }
        drop(pass);
        self.picking.encode_readback(encoder, *view, cursor);
    }

    fn ensure_targets(&mut self, device: &wgpu::Device, width: u32, height: u32) -> bool {
        let current = self
            .targets
            .as_ref()
            .is_some_and(|targets| targets.width == width && targets.height == height);
        if current || self.targets_refused == Some((width, height)) {
            return false;
        }
        let (targets, error) = gpu::scoped(device, || {
            SceneTargets::new(device, self.format, self.sample_count, width, height)
        });
        match error {
            None => {
                self.targets = Some(targets);
                self.targets_refused = None;
                false
            }
            Some(error) => {
                log::warn!(
                    "the graphics device refused {width}x{height} viewport targets at {}x multisampling: {error}",
                    self.sample_count
                );
                self.targets = None;
                self.targets_refused = Some((width, height));
                true
            }
        }
    }

    fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        viewport: &ViewportFrame<'_>,
    ) -> Faults {
        let mut faults = Faults::default();
        let view = viewport.view;
        let scene = viewport.scene;
        let anchored = AnchoredView {
            view,
            anchor: self.anchor_for(view),
        };

        let pixels_per_point = valid_scale(viewport.pixels_per_point);
        view_uniform(
            &mut self.staging,
            &anchored,
            pixels_per_point,
            self.shading,
            WHOLE_VIEW,
        );
        queue.write_buffer(&self.view_uniform.buffer, 0, self.staging.as_slice());
        let prepared = viewport.pick_at.map(|cursor| {
            (
                cursor,
                self.picking
                    .prepare(device, PickWindow::for_scale(pixels_per_point)),
            )
        });
        let refused = matches!(prepared, Some((_, PickPrepared::Refused)));
        faults.picking = refused && !self.pick_refused;
        self.pick_refused = refused;
        self.pick_window = None;
        if let Some((cursor, PickPrepared::Ready(window))) = prepared {
            let transform = picking::pick_transform(cursor, view.size(), window);
            self.pick_window = Some(ClipWindow::new(view, transform));
            view_uniform(
                &mut self.staging,
                &anchored,
                pixels_per_point,
                self.shading,
                transform,
            );
            queue.write_buffer(&self.pick_view_uniform.buffer, 0, self.staging.as_slice());
        }
        if let Some(grid) = &scene.grid {
            grid_uniform(&mut self.staging, grid, view);
            queue.write_buffer(&self.grid_uniform.buffer, 0, self.staging.as_slice());
        }

        faults.meshes = self
            .meshes
            .prepare(device, queue, &scene.meshes, view.eye())
            .saturating_add(self.translucent.prepare(
                device,
                queue,
                &scene.translucent_meshes,
                view.eye(),
            ))
            .saturating_add(
                self.overlay
                    .prepare(device, queue, &scene.overlay_meshes, view.eye()),
            )
            .saturating_add(
                self.flat
                    .prepare(device, queue, &scene.flat_meshes, view.eye()),
            );
        let (changed, refused_batches) =
            self.upload_batches(device, queue, &scene.batches, anchored.anchor);
        faults.batches = refused_batches;
        self.order_fills(Facing::of(view), changed);
        faults
    }

    fn anchor_for(&mut self, view: &View) -> Point3 {
        let eye = view.eye();
        let reach = reanchor_reach(view);
        let anchor = self
            .anchor
            .filter(|anchor| anchor.distance(eye) <= reach)
            .unwrap_or(eye);
        self.anchor = Some(anchor);
        anchor
    }

    fn upload_batches(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        batches: &[Arc<Batch>],
        anchor: Point3,
    ) -> (bool, u32) {
        let mut changed = self.batches.len() != batches.len();
        let mut refused = 0;
        self.batches.truncate(batches.len());
        for (slot, batch) in batches.iter().enumerate() {
            if slot >= self.batches.len() {
                self.batches.push(GpuBatch::new(device));
            }
            let Some(gpu) = self.batches.get_mut(slot) else {
                continue;
            };
            if !gpu.holds(batch, anchor) {
                let staging = &mut self.staging;
                let ((), error) = gpu::scoped(device, || {
                    gpu.upload(
                        device,
                        queue,
                        staging,
                        Uploaded {
                            batch,
                            anchor,
                            slot,
                        },
                    );
                });
                if let Some(error) = error {
                    log::warn!(
                        "the graphics device refused a batch of {} lines, {} markers and {} fills, so it is not drawn: {error}",
                        batch.lines.len(),
                        batch.markers.len(),
                        batch.fills.len()
                    );
                    *gpu = GpuBatch::refused(device, batch, anchor);
                    refused += 1;
                }
                changed = true;
                #[cfg(test)]
                {
                    self.work.uploads += 1;
                }
            }
        }
        (changed, refused)
    }

    fn order_fills(&mut self, facing: Facing, changed: bool) {
        if !changed && self.fill_order.sorted_for == Some(facing) {
            return;
        }
        let mut spans: Vec<FillSpan> = self
            .batches
            .iter()
            .flat_map(|batch| batch.fill_spans.iter().cloned())
            .collect();
        sort_back_to_front(&mut spans, facing);
        #[cfg(test)]
        {
            self.work.sorts += 1;
        }
        self.fill_order = FillOrder {
            sorted_for: Some(facing),
            draws: coalesced(spans),
        };
    }

    #[cfg(test)]
    pub fn work(&self) -> Work {
        self.work
    }

    #[cfg(test)]
    pub fn uploaded(&self) -> Vec<Option<Arc<Batch>>> {
        self.batches
            .iter()
            .map(|batch| batch.shown.clone())
            .collect()
    }

    #[cfg(test)]
    pub fn anchor(&self) -> Option<Point3> {
        self.anchor
    }

    #[cfg(test)]
    pub fn fill_draws(&self) -> Vec<(usize, Range<u32>)> {
        self.fill_order
            .draws
            .iter()
            .map(|draw| (draw.slot, draw.vertices.clone()))
            .collect()
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Work {
    pub uploads: usize,
    pub sorts: usize,
}

struct AnchoredView<'a> {
    view: &'a View,
    anchor: Point3,
}

fn clear_surface(encoder: &mut wgpu::CommandEncoder, surface: &SurfaceTarget<'_>) {
    drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("viewport background"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: surface.view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(BACKGROUND),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    }));
}

fn sort_back_to_front(spans: &mut [FillSpan], facing: Facing) {
    let depth = |span: &FillSpan| {
        span.centroid
            .map_or(f64::NEG_INFINITY, |centroid| facing.depth(centroid))
    };
    spans.sort_by(|a, b| {
        a.in_front
            .cmp(&b.in_front)
            .then(depth(b).total_cmp(&depth(a)))
    });
}

fn coalesced(spans: Vec<FillSpan>) -> Vec<FillDraw> {
    let mut draws: Vec<FillDraw> = Vec::with_capacity(spans.len());
    for span in spans {
        match draws.last_mut() {
            Some(draw)
                if draw.slot == span.slot
                    && draw.behind_faces == span.behind_faces
                    && draw.vertices.end == span.vertices.start =>
            {
                draw.vertices.end = span.vertices.end;
            }
            _ => draws.push(FillDraw {
                slot: span.slot,
                vertices: span.vertices,
                behind_faces: span.behind_faces,
            }),
        }
    }
    draws
}

struct Layouts<'a> {
    view: &'a wgpu::BindGroupLayout,
    grid: &'a wgpu::BindGroupLayout,
    mesh: &'a wgpu::BindGroupLayout,
}

impl Pipelines {
    fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        sample_count: u32,
        layouts: &Layouts<'_>,
    ) -> Self {
        let module = device.create_shader_module(wgpu::include_wgsl!("viewport.wgsl"));
        let pipeline_layout = |label, groups: &[Option<&wgpu::BindGroupLayout>]| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: groups,
                immediate_size: 0,
            })
        };
        let scene_layout = pipeline_layout("scene", &[Some(layouts.view)]);
        let grid_pipeline_layout =
            pipeline_layout("grid", &[Some(layouts.view), Some(layouts.grid)]);
        let mesh_pipeline_layout =
            pipeline_layout("mesh", &[Some(layouts.view), Some(layouts.mesh)]);

        let line_attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32, 4 => Uint32, 5 => Float32, 6 => Float32, 7 => Uint32];
        let marker_attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4, 2 => Float32, 3 => Uint32, 4 => Float32, 5 => Uint32];
        let fill_attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4, 2 => Uint32, 3 => Float32, 4 => Uint32];
        let mesh_attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Uint32];
        let lines = [Some(wgpu::VertexBufferLayout {
            array_stride: LINE_STRIDE,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &line_attributes,
        })];
        let markers = [Some(wgpu::VertexBufferLayout {
            array_stride: MARKER_STRIDE,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &marker_attributes,
        })];
        let fills = [Some(wgpu::VertexBufferLayout {
            array_stride: FILL_VERTEX_STRIDE,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &fill_attributes,
        })];

        let meshes = [Some(wgpu::VertexBufferLayout {
            array_stride: MESH_VERTEX_STRIDE,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &mesh_attributes,
        })];

        let color_target = [Some(wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let pick_targets = [
            Some(wgpu::ColorTargetState::from(picking::ID_FORMAT)),
            Some(wgpu::ColorTargetState::from(picking::DEPTH_VALUE_FORMAT)),
        ];
        let bias_of = |label: &str, vertex: &str| match (label, vertex) {
            (_, "vs_mesh") => FACE_DEPTH_BIAS,
            ("reference fills" | "pick reference fills", _) | (_, "vs_grid") => {
                BEHIND_FACES_DEPTH_BIAS
            }
            _ => wgpu::DepthBiasState::default(),
        };
        let color = |label, layout, vertex, buffers, fragment, depth_write| {
            build_pipeline(
                device,
                &PipelineSpec {
                    label,
                    layout,
                    module: &module,
                    vertex,
                    buffers,
                    fragment,
                    targets: &color_target,
                    depth_write,
                    depth_compare: wgpu::CompareFunction::GreaterEqual,
                    bias: bias_of(label, vertex),
                    sample_count,
                },
            )
        };
        let pick = |label, layout, vertex, buffers, fragment, depth_write| {
            build_pipeline(
                device,
                &PipelineSpec {
                    label,
                    layout,
                    module: &module,
                    vertex,
                    buffers,
                    fragment,
                    targets: &pick_targets,
                    depth_write,
                    depth_compare: wgpu::CompareFunction::GreaterEqual,
                    bias: bias_of(label, vertex),
                    sample_count: 1,
                },
            )
        };

        Self {
            meshes: color(
                "meshes",
                &mesh_pipeline_layout,
                "vs_mesh",
                &meshes,
                "fs_mesh",
                true,
            ),
            translucent_meshes: color(
                "translucent meshes",
                &mesh_pipeline_layout,
                "vs_mesh",
                &meshes,
                "fs_mesh",
                false,
            ),
            overlay_meshes: build_pipeline(
                device,
                &PipelineSpec {
                    label: "overlay meshes",
                    layout: &mesh_pipeline_layout,
                    module: &module,
                    vertex: "vs_mesh",
                    buffers: &meshes,
                    fragment: "fs_mesh",
                    targets: &color_target,
                    depth_write: false,
                    depth_compare: wgpu::CompareFunction::Always,
                    bias: wgpu::DepthBiasState::default(),
                    sample_count,
                },
            ),
            flat_meshes: color(
                "flat meshes",
                &mesh_pipeline_layout,
                "vs_mesh",
                &meshes,
                "fs_color",
                true,
            ),
            lines: color("lines", &scene_layout, "vs_line", &lines, "fs_line", true),
            markers: color(
                "markers",
                &scene_layout,
                "vs_marker",
                &markers,
                "fs_marker",
                true,
            ),
            fills: color("fills", &scene_layout, "vs_fill", &fills, "fs_color", false),
            reference_fills: color(
                "reference fills",
                &scene_layout,
                "vs_fill",
                &fills,
                "fs_color",
                false,
            ),
            grid: color(
                "grid",
                &grid_pipeline_layout,
                "vs_grid",
                &[],
                "fs_grid",
                false,
            ),
            pick_lines: pick(
                "pick lines",
                &scene_layout,
                "vs_line",
                &lines,
                "fs_pick",
                true,
            ),
            pick_markers: pick(
                "pick markers",
                &scene_layout,
                "vs_marker",
                &markers,
                "fs_marker_pick",
                true,
            ),
            pick_fills: pick(
                "pick fills",
                &scene_layout,
                "vs_fill",
                &fills,
                "fs_pick",
                false,
            ),
            pick_reference_fills: pick(
                "pick reference fills",
                &scene_layout,
                "vs_fill",
                &fills,
                "fs_pick",
                true,
            ),
            pick_meshes: pick(
                "pick meshes",
                &mesh_pipeline_layout,
                "vs_mesh",
                &meshes,
                "fs_mesh_pick",
                true,
            ),
            pick_translucent_meshes: pick(
                "pick translucent meshes",
                &mesh_pipeline_layout,
                "vs_mesh",
                &meshes,
                "fs_pick",
                true,
            ),
        }
    }
}

struct PipelineSpec<'a> {
    label: &'a str,
    layout: &'a wgpu::PipelineLayout,
    module: &'a wgpu::ShaderModule,
    vertex: &'a str,
    buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
    fragment: &'a str,
    targets: &'a [Option<wgpu::ColorTargetState>],
    depth_write: bool,
    depth_compare: wgpu::CompareFunction,
    bias: wgpu::DepthBiasState,
    sample_count: u32,
}

fn build_pipeline(device: &wgpu::Device, spec: &PipelineSpec<'_>) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(spec.label),
        layout: Some(spec.layout),
        vertex: wgpu::VertexState {
            module: spec.module,
            entry_point: Some(spec.vertex),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: spec.buffers,
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(spec.depth_write),
            depth_compare: Some(spec.depth_compare),
            stencil: wgpu::StencilState::default(),
            bias: spec.bias,
        }),
        multisample: wgpu::MultisampleState {
            count: spec.sample_count,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: spec.module,
            entry_point: Some(spec.fragment),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: spec.targets,
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn stage_fill(bytes: &mut Bytes, fill: &Fill, anchor: Point3) -> u32 {
    let depth_bias = fill.layer.depth_bias(Primitive::Fill);
    let in_front = u32::from(fill.layer.draws_in_front());
    let mut written = 0u32;
    for corner in fill.triangles.iter().flatten() {
        bytes
            .vec3(relative_to_eye(*corner, anchor))
            .floats(&fill.color.to_array())
            .u32(PickId::raw(fill.pick))
            .f32(depth_bias)
            .u32(in_front);
        written = written.saturating_add(1);
    }
    written
}

fn reanchor_reach(view: &View) -> f64 {
    let distance = view.viewpoint().distance;
    let pixel = view.units_per_pixel_at(distance);
    let within_error = ANCHOR_ERROR_PIXELS * pixel / F32_ROUNDING;
    (REANCHOR_DISTANCES * distance).max(within_error)
}

pub fn relative_to_eye(point: Point3, eye: Point3) -> Vec3 {
    (point - eye).as_vec3()
}

fn count(uploaded: u64) -> u32 {
    u32::try_from(uploaded).unwrap_or(u32::MAX)
}

fn valid_scale(pixels_per_point: f32) -> f32 {
    if pixels_per_point.is_finite() && pixels_per_point > 0.0 {
        pixels_per_point
    } else {
        1.0
    }
}

fn view_uniform(
    bytes: &mut Bytes,
    anchored: &AnchoredView<'_>,
    pixels_per_point: f32,
    shading: Shading,
    transform: [f32; 4],
) {
    let view = anchored.view;
    let size = view.size();
    bytes.clear();
    bytes
        .mat4(view.rotation_projection().as_mat4())
        .vec4(view.forward().as_vec3(), view.near_plane() as f32)
        .floats(&[
            size.x as f32,
            size.y as f32,
            pixels_per_point,
            if view.is_orthographic() { 1.0 } else { 0.0 },
        ])
        .floats(&transform)
        .vec4(key_light(view).as_vec3(), shading.uniform_flag())
        .vec4(fill_light(view).as_vec3(), 0.0)
        .vec4(relative_to_eye(anchored.anchor, view.eye()), 0.0);
}

fn key_light(view: &View) -> Vector3 {
    let viewpoint = view.viewpoint();
    (-viewpoint.forward() + viewpoint.up() * KEY_LIGHT_UP - viewpoint.right() * KEY_LIGHT_LEFT)
        .normalize_or(-viewpoint.forward())
}

fn fill_light(view: &View) -> Vector3 {
    let viewpoint = view.viewpoint();
    (-viewpoint.forward() - viewpoint.up() * FILL_LIGHT_DOWN + viewpoint.right() * FILL_LIGHT_RIGHT)
        .normalize_or(-viewpoint.forward())
}

pub fn grid_spacing(scale: f64) -> f64 {
    10f64.powf((scale / GRID_CELLS_ACROSS_SCALE).log10().floor())
}

fn grid_scale(plane: &Plane, view: &View) -> f64 {
    let distance = view.viewpoint().distance;
    let height = match view.projection() {
        Projection::Perspective => plane.signed_distance(view.eye()).abs(),
        Projection::Orthographic => distance,
    };
    height
        .max(distance * GRID_MIN_SCALE_PER_DISTANCE)
        .max(f64::MIN_POSITIVE)
}

pub fn grid_minor_spacing(grid: &Grid, view: &View) -> f64 {
    grid_spacing(grid_scale(&grid.plane, view))
}

fn grid_uniform(bytes: &mut Bytes, grid: &Grid, view: &View) {
    let plane = grid.plane;
    let eye = view.eye();
    let scale = grid_scale(&plane, view);
    let spacing = grid_spacing(scale);
    let snap = spacing * 100.0;
    let foot = plane.to_local(eye);
    let snapped = Point2::new(
        (foot.x / snap).round() * snap,
        (foot.y / snap).round() * snap,
    );
    let extent = scale * GRID_EXTENT_PER_SCALE;

    bytes.clear();
    bytes
        .vec4(relative_to_eye(plane.to_world(snapped), eye), extent as f32)
        .vec4(plane.x_axis().as_vec3(), spacing as f32)
        .vec4(plane.y_axis().as_vec3(), extent as f32)
        .floats(&grid.color.to_array());
}

fn begin_pick_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    targets: &PickTargets,
    label: &'static str,
    first: bool,
) -> wgpu::RenderPass<'a> {
    let load = if first {
        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
    } else {
        wgpu::LoadOp::Load
    };
    let attachment = |view| {
        Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            },
        })
    };
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[attachment(&targets.ids), attachment(&targets.depths)],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &targets.depth_buffer,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(FAR_DEPTH),
                store: wgpu::StoreOp::Discard,
            }),
            stencil_ops: None,
        }),
        ..Default::default()
    })
}

fn scissor_rect(rect: ViewportRect, width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
    let clamp = |value: f32, limit: u32| value.clamp(0.0, limit as f32) as u32;
    let left = clamp(rect.x.floor(), width);
    let top = clamp(rect.y.floor(), height);
    let right = clamp((rect.x + rect.width).ceil(), width);
    let bottom = clamp((rect.y + rect.height).ceil(), height);
    (right > left && bottom > top).then(|| (left, top, right - left, bottom - top))
}

#[cfg(test)]
mod tests {
    use caditor_geometry::Vector3;

    use super::*;
    use crate::camera::Viewpoint;

    #[test]
    fn relative_to_eye_keeps_micrometres_far_from_the_origin() {
        let eye = Point3::new(1.0e8, -2.5e7, 3.0e6);
        let a = eye + Vector3::new(40.0, 10.0, -5.0);
        let b = a + Vector3::new(1e-3, 0.0, 0.0);

        let separation = relative_to_eye(b, eye) - relative_to_eye(a, eye);
        assert!((separation.x - 1e-3).abs() < 1e-6);
        assert_eq!(a.as_vec3(), b.as_vec3());
    }

    #[test]
    fn shader_parses_and_validates() {
        let module = naga::front::wgsl::parse_str(include_str!("viewport.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .unwrap();
    }

    #[test]
    fn grid_spacing_is_a_power_of_ten_that_grows_with_scale() {
        assert_eq!(grid_spacing(100.0), 1.0);
        assert_eq!(grid_spacing(999.0), 1.0);
        assert_eq!(grid_spacing(1000.0), 10.0);
        assert_eq!(grid_spacing(5.0), 0.01);
    }

    fn looking_down() -> Facing {
        let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 100.0).unwrap();
        Facing::of(&View::new(viewpoint, 100.0, 100.0))
    }

    fn span(slot: usize, first: u32, z: f64, layer: Layer) -> FillSpan {
        let fill = Fill::convex(
            &[
                Point3::new(0.0, 0.0, z),
                Point3::new(1.0, 0.0, z),
                Point3::new(0.0, 1.0, z),
            ],
            crate::scene::Color::from_rgb8(0, 0, 0),
            layer,
            None,
        );
        FillSpan {
            slot,
            vertices: first..first + 3,
            centroid: fill.centroid(),
            in_front: layer.draws_in_front(),
            behind_faces: layer == Layer::Reference,
        }
    }

    fn heights(spans: &[FillSpan]) -> Vec<f64> {
        spans
            .iter()
            .filter_map(|span| span.centroid.map(|centroid| centroid.z))
            .collect()
    }

    #[test]
    fn sorts_fills_from_the_farthest_to_the_nearest() {
        let mut spans = [
            span(0, 0, 10.0, Layer::Reference),
            span(0, 3, -10.0, Layer::Reference),
            span(0, 6, 0.0, Layer::Reference),
        ];

        sort_back_to_front(&mut spans, looking_down());

        assert_eq!(heights(&spans), vec![-10.0, 0.0, 10.0]);
    }

    #[test]
    fn front_fills_draw_after_every_other_fill_whatever_their_depth() {
        let mut spans = [
            span(0, 0, -20.0, Layer::Front),
            span(0, 3, 10.0, Layer::Model),
            span(1, 0, -30.0, Layer::Front),
            span(1, 3, 0.0, Layer::Reference),
        ];

        sort_back_to_front(&mut spans, looking_down());

        assert_eq!(heights(&spans), vec![0.0, 10.0, -30.0, -20.0]);
        assert_eq!(
            spans.iter().map(|span| span.in_front).collect::<Vec<_>>(),
            vec![false, false, true, true]
        );
    }

    #[test]
    fn fills_in_order_in_one_buffer_share_a_draw() {
        let in_order = vec![
            span(0, 0, -10.0, Layer::Model),
            span(0, 3, 0.0, Layer::Model),
            span(1, 0, 5.0, Layer::Model),
            span(1, 3, 6.0, Layer::Model),
        ];
        let interleaved = vec![
            span(0, 0, -10.0, Layer::Model),
            span(1, 0, 5.0, Layer::Model),
            span(0, 3, 0.0, Layer::Model),
        ];
        let reversed = vec![
            span(0, 3, 0.0, Layer::Model),
            span(0, 0, -10.0, Layer::Model),
        ];

        let draws = |spans| {
            coalesced(spans)
                .into_iter()
                .map(|draw| (draw.slot, draw.vertices))
                .collect::<Vec<_>>()
        };
        assert_eq!(draws(in_order), vec![(0, 0..6), (1, 0..6)]);
        assert_eq!(draws(interleaved), vec![(0, 0..3), (1, 0..3), (0, 3..6)]);
        assert_eq!(draws(reversed), vec![(0, 3..6), (0, 0..3)]);
        let mixed = vec![
            span(0, 0, 0.0, Layer::Reference),
            span(0, 3, 1.0, Layer::Model),
        ];
        assert_eq!(
            coalesced(mixed)
                .into_iter()
                .map(|draw| draw.behind_faces)
                .collect::<Vec<_>>(),
            vec![true, false]
        );
    }

    #[test]
    fn scissor_stays_inside_the_surface() {
        let rect = ViewportRect {
            x: -5.0,
            y: 10.5,
            width: 300.0,
            height: 50.0,
        };
        assert_eq!(scissor_rect(rect, 200, 100), Some((0, 10, 200, 51)));
        let outside = ViewportRect { x: 250.0, ..rect };
        assert_eq!(scissor_rect(outside, 200, 100), None);
    }
}
