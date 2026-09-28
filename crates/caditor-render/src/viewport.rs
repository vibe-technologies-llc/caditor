use std::ops::Range;

use caditor_geometry::{Point2, Point3, Vector3};
use glam::{DVec2, Vec3};

use crate::{
    camera::View,
    gpu::{Bytes, GrowableBuffer},
    mesh::{MESH_VERTEX_STRIDE, MeshCache},
    picking::{self, PickTargets, Picking},
    scene::{Fill, Grid, Layer, PickId, Primitive, Scene, ViewportRect},
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
const LINE_STRIDE: u64 = 52;
const MARKER_STRIDE: u64 = 40;
const FILL_VERTEX_STRIDE: u64 = 36;
const VIEW_UNIFORM_SIZE: u64 = 128;
const KEY_LIGHT_UP: f64 = 0.8;
const KEY_LIGHT_LEFT: f64 = 0.5;
const GRID_UNIFORM_SIZE: u64 = 64;
const GRID_CELLS_ACROSS_SCALE: f64 = 100.0;
const GRID_EXTENT_PER_SCALE: f64 = 40.0;
const GRID_MIN_SCALE_PER_DISTANCE: f64 = 0.25;

pub struct ViewportFrame<'a> {
    pub rect: ViewportRect,
    pub view: &'a View,
    pub scene: &'a Scene,
    pub pick_at: Option<DVec2>,
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

struct Pipelines {
    meshes: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    markers: wgpu::RenderPipeline,
    fills: wgpu::RenderPipeline,
    grid: wgpu::RenderPipeline,
    pick_lines: wgpu::RenderPipeline,
    pick_markers: wgpu::RenderPipeline,
    pick_fills: wgpu::RenderPipeline,
    pick_reference_fills: wgpu::RenderPipeline,
    pick_meshes: wgpu::RenderPipeline,
}

struct SceneTargets {
    width: u32,
    height: u32,
    multisampled_color: Option<wgpu::TextureView>,
    depth: wgpu::TextureView,
}

#[derive(Default)]
struct Counts {
    lines: u32,
    markers: u32,
    fill_vertices: u32,
    pick_fills: PickFills,
}

#[derive(Default)]
struct PickFills {
    reference_vertices: u32,
    model_vertices: u32,
}

pub struct ViewportRenderer {
    format: wgpu::TextureFormat,
    sample_count: u32,
    pipelines: Pipelines,
    view_uniform: Uniform,
    pick_view_uniform: Uniform,
    grid_uniform: Uniform,
    lines: GrowableBuffer,
    markers: GrowableBuffer,
    fills: GrowableBuffer,
    pick_fills: GrowableBuffer,
    meshes: MeshCache,
    staging: Bytes,
    targets: Option<SceneTargets>,
    picking: Picking,
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
            pipelines: Pipelines::new(device, format, sample_count, &layouts),
            view_uniform: Uniform::new(device, &view_layout, "view", VIEW_UNIFORM_SIZE),
            pick_view_uniform: Uniform::new(device, &view_layout, "pick view", VIEW_UNIFORM_SIZE),
            grid_uniform: Uniform::new(device, &grid_layout, "grid", GRID_UNIFORM_SIZE),
            lines: GrowableBuffer::new(device, "lines", wgpu::BufferUsages::VERTEX),
            markers: GrowableBuffer::new(device, "markers", wgpu::BufferUsages::VERTEX),
            fills: GrowableBuffer::new(device, "fills", wgpu::BufferUsages::VERTEX),
            pick_fills: GrowableBuffer::new(device, "pick fills", wgpu::BufferUsages::VERTEX),
            meshes,
            staging: Bytes::default(),
            targets: None,
            picking: Picking::new(device, DEPTH_FORMAT),
        }
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
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
    ) {
        self.ensure_targets(device, surface.width, surface.height);
        let viewport =
            viewport.filter(|viewport| viewport.rect.width >= 1.0 && viewport.rect.height >= 1.0);
        let counts = match viewport {
            Some(viewport) => self.upload(device, queue, viewport),
            None => {
                self.meshes.clear();
                Counts::default()
            }
        };

        let Some(targets) = self.targets.as_ref() else {
            return;
        };
        let (color_view, resolve_target) = match &targets.multisampled_color {
            Some(multisampled) => (multisampled, Some(surface.view)),
            None => (surface.view, None),
        };

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("viewport"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color_view,
                depth_slice: None,
                resolve_target,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(BACKGROUND),
                    store: wgpu::StoreOp::Store,
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
            return;
        };
        let Some(scissor) = scissor_rect(viewport.rect, surface.width, surface.height) else {
            return;
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
        pass.set_bind_group(0, &self.view_uniform.bind_group, &[]);
        self.meshes.draw(&mut pass, &self.pipelines.meshes);
        self.draw_lines(&mut pass, &self.pipelines.lines, &counts);
        self.draw_markers(&mut pass, &self.pipelines.markers, &counts);
        if viewport.scene.grid.is_some() {
            pass.set_pipeline(&self.pipelines.grid);
            pass.set_bind_group(1, &self.grid_uniform.bind_group, &[]);
            pass.draw(0..QUAD_VERTICES, 0..1);
        }
        self.draw_fills(&mut pass, &self.pipelines.fills, &counts);
        drop(pass);

        if let Some(cursor) = viewport.pick_at {
            self.draw_pick(encoder, viewport.view, cursor, &counts);
        }
    }

    fn draw_lines(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipeline: &wgpu::RenderPipeline,
        counts: &Counts,
    ) {
        if counts.lines == 0 {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_vertex_buffer(0, self.lines.slice(u64::from(counts.lines) * LINE_STRIDE));
        pass.draw(0..QUAD_VERTICES, 0..counts.lines);
    }

    fn draw_markers(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipeline: &wgpu::RenderPipeline,
        counts: &Counts,
    ) {
        if counts.markers == 0 {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_vertex_buffer(
            0,
            self.markers
                .slice(u64::from(counts.markers) * MARKER_STRIDE),
        );
        pass.draw(0..QUAD_VERTICES, 0..counts.markers);
    }

    fn draw_fills(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipeline: &wgpu::RenderPipeline,
        counts: &Counts,
    ) {
        if counts.fill_vertices == 0 {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_vertex_buffer(
            0,
            self.fills
                .slice(u64::from(counts.fill_vertices) * FILL_VERTEX_STRIDE),
        );
        pass.draw(0..counts.fill_vertices, 0..1);
    }

    fn draw_pick(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &View,
        cursor: DVec2,
        counts: &Counts,
    ) {
        if !self.picking.is_idle() {
            return;
        }
        let fills = &counts.pick_fills;
        let targets = self.picking.targets();
        let mut behind = begin_pick_pass(encoder, targets, "pick reference fills", true);
        behind.set_bind_group(0, &self.pick_view_uniform.bind_group, &[]);
        self.draw_pick_fills(
            &mut behind,
            &self.pipelines.pick_reference_fills,
            0..fills.reference_vertices,
        );
        drop(behind);
        let mut pass = begin_pick_pass(encoder, targets, "pick", false);
        pass.set_bind_group(0, &self.pick_view_uniform.bind_group, &[]);
        self.meshes.draw(&mut pass, &self.pipelines.pick_meshes);
        self.draw_pick_fills(
            &mut pass,
            &self.pipelines.pick_fills,
            fills.reference_vertices..fills.reference_vertices + fills.model_vertices,
        );
        self.draw_lines(&mut pass, &self.pipelines.pick_lines, counts);
        self.draw_markers(&mut pass, &self.pipelines.pick_markers, counts);
        drop(pass);
        self.picking.encode_readback(encoder, *view, cursor);
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

    fn ensure_targets(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let current = self
            .targets
            .as_ref()
            .is_some_and(|targets| targets.width == width && targets.height == height);
        if current {
            return;
        }
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
                    sample_count: self.sample_count,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        self.targets = Some(SceneTargets {
            width,
            height,
            multisampled_color: (self.sample_count > 1)
                .then(|| texture("multisampled viewport color", self.format)),
            depth: texture("viewport depth", DEPTH_FORMAT),
        });
    }

    fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        viewport: &ViewportFrame<'_>,
    ) -> Counts {
        let view = viewport.view;
        let scene = viewport.scene;
        let eye = view.eye();

        view_uniform(&mut self.staging, view, None);
        queue.write_buffer(&self.view_uniform.buffer, 0, self.staging.as_slice());
        if let Some(cursor) = viewport.pick_at {
            view_uniform(&mut self.staging, view, Some(cursor));
            queue.write_buffer(&self.pick_view_uniform.buffer, 0, self.staging.as_slice());
        }
        if let Some(grid) = &scene.grid {
            grid_uniform(&mut self.staging, grid, view);
            queue.write_buffer(&self.grid_uniform.buffer, 0, self.staging.as_slice());
        }

        self.meshes.prepare(device, queue, &scene.meshes, eye);

        self.staging.clear();
        for line in &scene.lines {
            self.staging
                .vec3(relative_to_eye(line.start, eye))
                .vec3(relative_to_eye(line.end, eye))
                .floats(&line.color.to_array())
                .f32(line.width)
                .u32(PickId::raw(line.pick))
                .f32(line.layer.depth_bias(Primitive::Line));
        }
        self.lines.upload(device, queue, &self.staging);

        self.staging.clear();
        for marker in &scene.markers {
            self.staging
                .vec3(relative_to_eye(marker.position, eye))
                .floats(&marker.color.to_array())
                .f32(marker.diameter)
                .u32(PickId::raw(marker.pick))
                .f32(marker.layer.depth_bias(Primitive::Marker));
        }
        self.markers.upload(device, queue, &self.staging);

        self.staging.clear();
        let mut fill_vertices = 0u32;
        for fill in fills_back_to_front(&scene.fills, view) {
            let depth_bias = fill.layer.depth_bias(Primitive::Fill);
            for corner in fill.triangles.iter().flatten() {
                self.staging
                    .vec3(relative_to_eye(*corner, eye))
                    .floats(&fill.color.to_array())
                    .u32(PickId::raw(fill.pick))
                    .f32(depth_bias);
                fill_vertices = fill_vertices.saturating_add(1);
            }
        }
        self.fills.upload(device, queue, &self.staging);

        let pick_fills = match viewport.pick_at {
            Some(_) => self.upload_pick_fills(device, queue, &scene.fills, eye),
            None => PickFills::default(),
        };

        Counts {
            lines: u32::try_from(scene.lines.len()).unwrap_or(u32::MAX),
            markers: u32::try_from(scene.markers.len()).unwrap_or(u32::MAX),
            fill_vertices,
            pick_fills,
        }
    }

    fn upload_pick_fills(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        fills: &[Fill],
        eye: Point3,
    ) -> PickFills {
        self.staging.clear();
        let mut counts = PickFills::default();
        for layer in [Layer::Reference, Layer::Model] {
            let mut written = 0u32;
            for fill in fills
                .iter()
                .filter(|fill| fill.layer == layer && fill.pick.is_some())
            {
                let depth_bias = fill.layer.depth_bias(Primitive::Fill);
                for corner in fill.triangles.iter().flatten() {
                    self.staging
                        .vec3(relative_to_eye(*corner, eye))
                        .floats(&fill.color.to_array())
                        .u32(PickId::raw(fill.pick))
                        .f32(depth_bias);
                    written = written.saturating_add(1);
                }
            }
            match layer {
                Layer::Reference => counts.reference_vertices = written,
                Layer::Model => counts.model_vertices = written,
            }
        }
        self.pick_fills.upload(device, queue, &self.staging);
        counts
    }
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

        let line_attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32, 4 => Uint32, 5 => Float32];
        let marker_attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4, 2 => Float32, 3 => Uint32, 4 => Float32];
        let fill_attributes =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4, 2 => Uint32, 3 => Float32];
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
            lines: color("lines", &scene_layout, "vs_line", &lines, "fs_color", true),
            markers: color(
                "markers",
                &scene_layout,
                "vs_marker",
                &markers,
                "fs_marker",
                true,
            ),
            fills: color("fills", &scene_layout, "vs_fill", &fills, "fs_color", false),
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
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
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

pub fn relative_to_eye(point: Point3, eye: Point3) -> Vec3 {
    (point - eye).as_vec3()
}

fn view_uniform(bytes: &mut Bytes, view: &View, pick_cursor: Option<DVec2>) {
    let size = view.size();
    let pick_transform = match pick_cursor {
        Some(cursor) => picking::pick_transform(cursor, size),
        None => [1.0, 1.0, 0.0, 0.0],
    };
    bytes.clear();
    bytes
        .mat4(view.rotation_projection().as_mat4())
        .vec4(view.forward().as_vec3(), view.near_plane() as f32)
        .floats(&[size.x as f32, size.y as f32, 0.0, 0.0])
        .floats(&pick_transform)
        .vec4(key_light(view).as_vec3(), 0.0);
}

fn key_light(view: &View) -> Vector3 {
    let viewpoint = view.viewpoint();
    (-viewpoint.forward() + viewpoint.up() * KEY_LIGHT_UP - viewpoint.right() * KEY_LIGHT_LEFT)
        .normalize_or(-viewpoint.forward())
}

pub fn grid_spacing(scale: f64) -> f64 {
    10f64.powf((scale / GRID_CELLS_ACROSS_SCALE).log10().floor())
}

fn grid_uniform(bytes: &mut Bytes, grid: &Grid, view: &View) {
    let plane = grid.plane;
    let eye = view.eye();
    let scale = plane
        .signed_distance(eye)
        .abs()
        .max(view.viewpoint().distance * GRID_MIN_SCALE_PER_DISTANCE)
        .max(f64::MIN_POSITIVE);
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

fn fills_back_to_front<'a>(fills: &'a [Fill], view: &View) -> Vec<&'a Fill> {
    let depth = |fill: &Fill| {
        fill.centroid()
            .map_or(f64::NEG_INFINITY, |centroid| view.view_depth(centroid))
    };
    let mut sorted: Vec<(f64, &Fill)> = fills.iter().map(|fill| (depth(fill), fill)).collect();
    sorted.sort_by(|a, b| b.0.total_cmp(&a.0));
    sorted.into_iter().map(|(_, fill)| fill).collect()
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

    #[test]
    fn sorts_fills_from_the_farthest_to_the_nearest() {
        let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 100.0).unwrap();
        let view = View::new(viewpoint, 100.0, 100.0);
        let square_at = |z: f64| {
            Fill::convex(
                &[
                    Point3::new(0.0, 0.0, z),
                    Point3::new(1.0, 0.0, z),
                    Point3::new(0.0, 1.0, z),
                ],
                crate::scene::Color::from_rgb8(0, 0, 0),
                crate::scene::Layer::Reference,
                None,
            )
        };
        let fills = [square_at(10.0), square_at(-10.0), square_at(0.0)];

        let order: Vec<f64> = fills_back_to_front(&fills, &view)
            .iter()
            .filter_map(|fill| fill.centroid().map(|centroid| centroid.z))
            .collect();
        assert_eq!(order, vec![-10.0, 0.0, 10.0]);
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
