use caditor_geometry::{Point3, Vector3};
use glam::{DMat4, DVec4, Mat4};

use crate::{
    camera::View,
    gpu::{self, Pack},
};

pub const SHADOW_MAP_SIDE: u32 = 2048;
const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const BOUNCE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const BOUNCE_REACH: f64 = 0.12;
const NEAREST_BOUNCE_SHARE: f64 = 0.01;
const BOUNCE_SAMPLES: f64 = 24.0;
const SHADOW_SIDE_MARGIN: f64 = 0.04;
const SHADOW_DEPTH_MARGIN: f64 = 0.05;
const NORMAL_OFFSET_TEXELS: f64 = 1.5;
const SHADOW_DEPTH_BIAS: wgpu::DepthBiasState = wgpu::DepthBiasState {
    constant: 2,
    slope_scale: 2.0,
    clamp: 0.0,
};
const FARTHEST_SHADOW_DEPTH: f32 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lighting {
    pub direction: LightDirection,
    pub colour: [f32; 3],
    pub strength: f32,
    pub environment: f32,
    pub exposure: f32,
    pub shadows: bool,
    pub bounce: f32,
}

impl Lighting {
    pub fn needs_light_pass(&self) -> bool {
        self.shadows || self.bounce > 0.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LightDirection {
    View { azimuth: f64, elevation: f64 },
    Model { azimuth: f64, elevation: f64 },
}

impl LightDirection {
    pub fn toward_light(self, view: &View) -> Vector3 {
        match self {
            Self::View { azimuth, elevation } => {
                let viewpoint = view.viewpoint();
                let level = elevation.cos();
                (-viewpoint.forward() * azimuth.cos() * level
                    + viewpoint.right() * azimuth.sin() * level
                    + viewpoint.up() * elevation.sin())
                .normalize_or(-viewpoint.forward())
            }
            Self::Model { azimuth, elevation } => {
                let level = elevation.cos();
                Vector3::new(
                    azimuth.cos() * level,
                    azimuth.sin() * level,
                    elevation.sin(),
                )
                .normalize_or(Vector3::Z)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LightFrame {
    matrix: Mat4,
    relative_from_light: Mat4,
    normal_offset: f32,
    side: f64,
}

impl LightFrame {
    pub(crate) fn fitted(
        toward_light: Vector3,
        eye: Point3,
        casters: impl IntoIterator<Item = [Point3; 8]>,
    ) -> Option<Self> {
        let (right, up) = toward_light.any_orthonormal_pair();
        let along = -toward_light;
        let mut low = Vector3::splat(f64::INFINITY);
        let mut high = Vector3::splat(f64::NEG_INFINITY);
        for corner in casters.into_iter().flatten() {
            let light = Vector3::new(right.dot(corner), up.dot(corner), along.dot(corner));
            low = low.min(light);
            high = high.max(light);
        }
        if !(low.is_finite() && high.is_finite()) {
            return None;
        }
        let middle = (low + high) * 0.5;
        let side = (high.x - low.x).max(high.y - low.y) * (1.0 + SHADOW_SIDE_MARGIN);
        let depth_margin = (high.z - low.z).max(side) * SHADOW_DEPTH_MARGIN;
        let near = low.z - depth_margin;
        let depth = (high.z + depth_margin - near).max(f64::MIN_POSITIVE);
        let half = (side * 0.5).max(f64::MIN_POSITIVE);
        let row = |axis: Vector3, offset: f64, scale: f64| {
            DVec4::new(
                axis.x * scale,
                axis.y * scale,
                axis.z * scale,
                (axis.dot(eye) - offset) * scale,
            )
        };
        let rows = [
            row(right, middle.x, 1.0 / half),
            row(up, middle.y, 1.0 / half),
            row(along, near, 1.0 / depth),
            DVec4::W,
        ];
        let matrix = DMat4::from_cols(rows[0], rows[1], rows[2], rows[3]).transpose();
        let texel = side / f64::from(SHADOW_MAP_SIDE);
        Some(Self {
            matrix: matrix.as_mat4(),
            relative_from_light: matrix.inverse().as_mat4(),
            normal_offset: (texel * NORMAL_OFFSET_TEXELS) as f32,
            side,
        })
    }

    #[cfg(test)]
    pub(crate) fn place(&self, relative: glam::Vec3) -> glam::Vec3 {
        self.matrix.project_point3(relative)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LightUniform {
    sun: [f32; 4],
    colour: [f32; 4],
    exposure_shadow: [f32; 4],
    bounce: [f32; 4],
    shadow_matrix: Mat4,
    relative_from_light: Mat4,
}

impl LightUniform {
    pub(crate) const NONE: Self = Self {
        sun: [0.0; 4],
        colour: [0.0; 4],
        exposure_shadow: [0.0; 4],
        bounce: [0.0; 4],
        shadow_matrix: Mat4::ZERO,
        relative_from_light: Mat4::ZERO,
    };

    pub(crate) fn of(
        lighting: &Lighting,
        toward_light: Vector3,
        frame: Option<LightFrame>,
    ) -> Self {
        let shadow = frame.filter(|_| lighting.shadows);
        let bounce = finite_at_least_zero(lighting.bounce);
        let toward = toward_light.as_vec3();
        let [red, green, blue] = lighting.colour.map(finite_at_least_zero);
        Self {
            sun: [
                toward.x,
                toward.y,
                toward.z,
                finite_at_least_zero(lighting.strength),
            ],
            colour: [red, green, blue, finite_at_least_zero(lighting.environment)],
            exposure_shadow: [
                exposure_scale(lighting.exposure),
                if shadow.is_some() { 1.0 } else { 0.0 },
                1.0 / SHADOW_MAP_SIDE as f32,
                frame.map_or(0.0, |frame| frame.normal_offset),
            ],
            bounce: match frame.filter(|_| bounce > 0.0) {
                Some(frame) => bounce_uniform(bounce, frame.side),
                None => [0.0; 4],
            },
            shadow_matrix: frame.map_or(Mat4::ZERO, |frame| frame.matrix),
            relative_from_light: frame.map_or(Mat4::ZERO, |frame| frame.relative_from_light),
        }
    }

    pub(crate) fn needs_light_pass(&self) -> bool {
        self.exposure_shadow[1] > 0.0 || self.bounce[0] > 0.0
    }

    pub(crate) fn write(&self, bytes: &mut impl Pack) {
        bytes
            .floats(&self.sun)
            .floats(&self.colour)
            .floats(&self.exposure_shadow)
            .floats(&self.bounce)
            .mat4(self.shadow_matrix)
            .mat4(self.relative_from_light);
    }
}

fn bounce_uniform(strength: f32, side: f64) -> [f32; 4] {
    let reach = side * BOUNCE_REACH;
    let nearest = side * NEAREST_BOUNCE_SHARE;
    let radiance_per_sample = reach * reach / BOUNCE_SAMPLES;
    [
        strength,
        BOUNCE_REACH as f32,
        (nearest * nearest) as f32,
        radiance_per_sample as f32,
    ]
}

fn finite_at_least_zero(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn exposure_scale(stops: f32) -> f32 {
    let scale = if stops.is_finite() { stops.exp2() } else { 1.0 };
    scale.max(f32::MIN_POSITIVE)
}

pub(crate) struct LightPass {
    layout: wgpu::BindGroupLayout,
    stand_in: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    made: Option<Made>,
    refused: bool,
}

struct Made {
    targets: LightTargets,
    bind_group: wgpu::BindGroup,
}

struct LightTargets {
    depth: wgpu::TextureView,
    albedo: wgpu::TextureView,
    normals: wgpu::TextureView,
}

impl LightTargets {
    fn new(device: &wgpu::Device, side: u32) -> Self {
        Self {
            depth: light_texture(device, "shadow map", side, SHADOW_FORMAT),
            albedo: light_texture(device, "lit albedo", side, BOUNCE_FORMAT),
            normals: light_texture(device, "lit normals", side, BOUNCE_FORMAT),
        }
    }
}

pub(crate) enum LightPassReady {
    Ready,
    Refused,
    NewlyRefused,
}

impl LightPass {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow map"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                bounce_entry(2),
                bounce_entry(3),
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow map"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let stand_in = bind_group(device, &layout, &sampler, &LightTargets::new(device, 1));
        Self {
            layout,
            stand_in,
            sampler,
            made: None,
            refused: false,
        }
    }

    pub(crate) fn sibling(&self, device: &wgpu::Device) -> Self {
        Self {
            layout: self.layout.clone(),
            stand_in: bind_group(
                device,
                &self.layout,
                &self.sampler,
                &LightTargets::new(device, 1),
            ),
            sampler: self.sampler.clone(),
            made: None,
            refused: false,
        }
    }

    pub(crate) fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    pub(crate) fn ensure(&mut self, device: &wgpu::Device) -> LightPassReady {
        if self.made.is_some() {
            return LightPassReady::Ready;
        }
        if self.refused {
            return LightPassReady::Refused;
        }
        let side = SHADOW_MAP_SIDE.min(device.limits().max_texture_dimension_2d);
        let (targets, error) = gpu::scoped(device, || LightTargets::new(device, side));
        match error {
            None => {
                self.made = Some(Made {
                    bind_group: bind_group(device, &self.layout, &self.sampler, &targets),
                    targets,
                });
                LightPassReady::Ready
            }
            Some(error) => {
                log::warn!(
                    "the graphics device refused a {side}x{side} shadow map, so the rendered view has no shadows or bounced light: {error}"
                );
                self.refused = true;
                LightPassReady::NewlyRefused
            }
        }
    }

    pub(crate) fn bind_group(&self) -> &wgpu::BindGroup {
        self.made
            .as_ref()
            .map_or(&self.stand_in, |made| &made.bind_group)
    }

    pub(crate) fn begin_pass<'a>(
        &self,
        encoder: &'a mut wgpu::CommandEncoder,
    ) -> Option<wgpu::RenderPass<'a>> {
        let made = self.made.as_ref()?;
        let cleared = |view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })
        };
        Some(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shadow map"),
            color_attachments: &[
                cleared(&made.targets.albedo),
                cleared(&made.targets.normals),
            ],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &made.targets.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(FARTHEST_SHADOW_DEPTH),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        }))
    }
}

fn bounce_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn light_texture(
    device: &wgpu::Device,
    label: &'static str,
    side: u32,
    format: wgpu::TextureFormat,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

fn bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    targets: &LightTargets,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("shadow map"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&targets.depth),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&targets.albedo),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&targets.normals),
            },
        ],
    })
}

pub(crate) fn shadow_pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("shadow map"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_shadow"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers,
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: SHADOW_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: SHADOW_DEPTH_BIAS,
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs_shadow"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[
                Some(wgpu::ColorTargetState::from(BOUNCE_FORMAT)),
                Some(wgpu::ColorTargetState::from(BOUNCE_FORMAT)),
            ],
        }),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use caditor_geometry::Aabb;

    use super::*;
    use crate::camera::Viewpoint;

    fn view() -> View {
        View::new(
            Viewpoint::looking_from(Vector3::new(1.0, -1.0, 0.5), Point3::ZERO, 100.0)
                .expect("the direction is not zero"),
            800.0,
            600.0,
        )
    }

    #[test]
    fn a_light_at_no_turn_and_no_height_in_the_view_frame_shines_from_the_eye() {
        let view = view();
        let toward = LightDirection::View {
            azimuth: 0.0,
            elevation: 0.0,
        }
        .toward_light(&view);
        assert!(toward.distance(-view.forward()) < 1e-12);

        let overhead = LightDirection::View {
            azimuth: 0.3,
            elevation: std::f64::consts::FRAC_PI_2,
        }
        .toward_light(&view);
        assert!(overhead.distance(view.viewpoint().up()) < 1e-12);

        let right = LightDirection::View {
            azimuth: std::f64::consts::FRAC_PI_2,
            elevation: 0.0,
        }
        .toward_light(&view);
        assert!(right.distance(view.viewpoint().right()) < 1e-12);
    }

    #[test]
    fn a_light_in_the_model_frame_turns_from_x_toward_y_and_rises_toward_z() {
        let along_x = LightDirection::Model {
            azimuth: 0.0,
            elevation: 0.0,
        }
        .toward_light(&view());
        let along_y = LightDirection::Model {
            azimuth: std::f64::consts::FRAC_PI_2,
            elevation: 0.0,
        }
        .toward_light(&view());
        let up = LightDirection::Model {
            azimuth: 1.0,
            elevation: std::f64::consts::FRAC_PI_2,
        }
        .toward_light(&view());

        assert!(along_x.distance(Vector3::X) < 1e-12);
        assert!(along_y.distance(Vector3::Y) < 1e-12);
        assert!(up.distance(Vector3::Z) < 1e-12);
    }

    #[test]
    fn the_light_frame_holds_every_caster_corner_with_the_nearest_toward_the_light_first() {
        let eye = Point3::new(5_000.0, -3_000.0, 2_000.0);
        let toward_light = Vector3::new(0.3, -0.4, 0.8).normalize();
        let boxes = [
            [
                Point3::new(4_000.0, 0.0, 0.0),
                Point3::new(4_100.0, 50.0, 20.0),
            ],
            [
                Point3::new(4_200.0, 30.0, 0.0),
                Point3::new(4_260.0, 90.0, 300.0),
            ],
        ];
        let corners: Vec<[Point3; 8]> = boxes
            .into_iter()
            .map(|ends| {
                Aabb::from_points(ends)
                    .expect("two ends make a box")
                    .corners()
            })
            .collect();
        let frame = LightFrame::fitted(toward_light, eye, corners.iter().copied())
            .expect("the casters have a frame");

        for corner in corners.iter().flatten() {
            let placed = frame.place((*corner - eye).as_vec3());
            assert!(placed.x.abs() <= 1.0 && placed.y.abs() <= 1.0, "{placed}");
            assert!((0.0..=1.0).contains(&placed.z), "{placed}");
        }
        let top = frame.place((Point3::new(4_230.0, 60.0, 300.0) - eye).as_vec3());
        let bottom = frame.place((Point3::new(4_230.0, 60.0, 0.0) - eye).as_vec3());
        assert!(top.z < bottom.z);
    }

    #[test]
    fn no_caster_leaves_no_light_frame() {
        assert_eq!(
            LightFrame::fitted(Vector3::Z, Point3::ZERO, std::iter::empty()),
            None
        );
    }

    #[test]
    fn exposure_doubles_per_stop_and_odd_values_light_nothing_wrongly() {
        assert_eq!(exposure_scale(0.0), 1.0);
        assert_eq!(exposure_scale(2.0), 4.0);
        assert_eq!(exposure_scale(f32::NAN), 1.0);
        assert_eq!(finite_at_least_zero(-3.0), 0.0);
        assert_eq!(finite_at_least_zero(f32::INFINITY), 0.0);
    }
}
