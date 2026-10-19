use std::sync::Arc;

use caditor_geometry::{Point3, Vector3};
use glam::{DMat4, DVec4, Mat4};

use crate::{
    camera::View,
    environment::Environment,
    environment_textures::{self, EnvironmentViews},
    gpu::{self, Pack},
    scene::Color,
};

pub const SHADOW_MAP_SIDE: u32 = 2048;
const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const BOUNCE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const LIT_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
pub(crate) const PEEL_LAYERS: usize = 2;
pub(crate) const CROSS_MAP_SIDE: u32 = 1024;
const PEELED_BINDING: u32 = 8;
const PREVIOUS_LAYER_BINDING: u32 = 10;
const CROSSED_BINDING: u32 = 11;
const CROSS_TILT: f64 = 1.0;
const SKEW_ACROSS: (f64, f64) = (0.8, 0.6);
const MOST_VERTICAL_SUN_FOR_LEVEL: f64 = 0.99;
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
const LEVEL_HEADING: f64 = 1e-6;
const MOST_ENVIRONMENT_GAIN: f32 = 1024.0;
const LONGEST_GROUND_SHADOW: f64 = 3.0;
const LOWEST_SUN_FOR_GROUND_SHADOWS: f64 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lighting {
    pub direction: LightDirection,
    pub colour: [f32; 3],
    pub strength: f32,
    pub environment: f32,
    pub exposure: f32,
    pub shadows: bool,
    pub bounce: f32,
    pub occlusion: f32,
    pub surroundings: Surroundings,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Surroundings {
    pub backdrop: Option<Color>,
    pub floor: bool,
}

impl Surroundings {
    fn backdrop_uniform(self) -> [f32; 4] {
        match self.backdrop {
            Some(colour) => {
                let [red, green, blue] = [colour.red, colour.green, colour.blue]
                    .map(|channel| finite_at_least_zero(channel).min(1.0));
                [red, green, blue, 1.0]
            }
            None => [0.0; 4],
        }
    }

    fn floor_uniform(self) -> [f32; 4] {
        [if self.floor { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0]
    }
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
                let behind = behind_the_viewer(view);
                let right = Vector3::Z.cross(behind);
                let level = elevation.cos();
                ((behind * azimuth.cos() + right * azimuth.sin()) * level
                    + Vector3::Z * elevation.sin())
                .normalize_or(Vector3::Z)
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

fn behind_the_viewer(view: &View) -> Vector3 {
    let viewpoint = view.viewpoint();
    let level = |direction: Vector3| Vector3::new(direction.x, direction.y, 0.0);
    let back = level(-viewpoint.forward());
    let back = if back.length() > LEVEL_HEADING {
        back
    } else {
        level(-viewpoint.up())
    };
    back.normalize_or(Vector3::NEG_Y)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Ground {
    height: f64,
    centre: Point3,
    radius: f64,
}

impl Ground {
    pub(crate) fn under(casters: &[[Point3; 8]]) -> Option<Self> {
        let mut low = Vector3::splat(f64::INFINITY);
        let mut high = Vector3::splat(f64::NEG_INFINITY);
        for corner in casters.iter().flatten() {
            low = low.min(*corner);
            high = high.max(*corner);
        }
        let radius = (high - low).length() * 0.5;
        (low.is_finite() && high.is_finite() && radius > 0.0).then(|| Self {
            height: low.z,
            centre: (low + high) * 0.5,
            radius,
        })
    }

    pub(crate) fn shadows_of(
        self,
        toward_light: Vector3,
        casters: &[[Point3; 8]],
    ) -> Vec<[Point3; 8]> {
        if toward_light.z < LOWEST_SUN_FOR_GROUND_SHADOWS {
            return Vec::new();
        }
        let longest = self.radius * LONGEST_GROUND_SHADOW;
        casters
            .iter()
            .map(|corners| {
                corners.map(|corner| {
                    let reach = ((corner.z - self.height) / toward_light.z).min(longest);
                    corner - toward_light * reach
                })
            })
            .collect()
    }

    fn uniform(self, eye: Point3) -> [f32; 4] {
        [
            (self.height - eye.z) as f32,
            self.radius as f32,
            (self.centre.x - eye.x) as f32,
            (self.centre.y - eye.y) as f32,
        ]
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
    pub(crate) fn crossing(
        toward_light: Vector3,
        eye: Point3,
        casters: impl IntoIterator<Item = [Point3; 8]>,
    ) -> Option<Self> {
        Self::fitted(cross_direction(toward_light), eye, casters, CROSS_MAP_SIDE)
    }

    pub(crate) fn fitted(
        toward_light: Vector3,
        eye: Point3,
        casters: impl IntoIterator<Item = [Point3; 8]>,
        map_side: u32,
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
        let texel = side / f64::from(map_side);
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

fn cross_direction(toward_light: Vector3) -> Vector3 {
    let level_across = if toward_light.z.abs() < MOST_VERTICAL_SUN_FOR_LEVEL {
        toward_light.cross(Vector3::Z).normalize_or(Vector3::X)
    } else {
        Vector3::new(SKEW_ACROSS.0, SKEW_ACROSS.1, 0.0)
    };
    (toward_light + level_across * CROSS_TILT).normalize_or(toward_light)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LightUniform {
    sun: [f32; 4],
    colour: [f32; 4],
    exposure_shadow: [f32; 4],
    bounce: [f32; 4],
    ground: [f32; 4],
    sky_view: [f32; 4],
    environment: [f32; 4],
    backdrop_colour: [f32; 4],
    floor: [f32; 4],
    shadow_matrix: Mat4,
    relative_from_light: Mat4,
    cross_matrix: Mat4,
    relative_from_cross: Mat4,
    cross_frame: [f32; 4],
}

impl LightUniform {
    pub(crate) const NONE: Self = Self {
        sun: [0.0; 4],
        colour: [0.0; 4],
        exposure_shadow: [0.0; 4],
        bounce: [0.0; 4],
        ground: [0.0; 4],
        sky_view: [0.0; 4],
        environment: [0.0; 4],
        backdrop_colour: [0.0; 4],
        floor: [0.0; 4],
        shadow_matrix: Mat4::ZERO,
        relative_from_light: Mat4::ZERO,
        cross_matrix: Mat4::ZERO,
        relative_from_cross: Mat4::ZERO,
        cross_frame: [0.0; 4],
    };

    pub(crate) fn of(
        (lighting, view): (&Lighting, &View),
        toward_light: Vector3,
        (frame, cross, ground): (Option<LightFrame>, Option<LightFrame>, Option<Ground>),
    ) -> Self {
        let tan_y = view.tan_half_fov_y();
        let shadow = frame.filter(|_| lighting.shadows);
        let bounce = finite_at_least_zero(lighting.bounce);
        let cross = cross.filter(|_| frame.is_some() && bounce > 0.0);
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
            bounce: match frame.filter(|_| bounce > 0.0 && cross.is_some()) {
                Some(frame) => bounce_uniform(bounce, frame.side),
                None => [0.0; 4],
            },
            ground: ground.map_or([0.0; 4], |ground| ground.uniform(view.eye())),
            sky_view: [(tan_y * view.aspect()) as f32, tan_y as f32, 0.0, 0.0],
            environment: [0.0; 4],
            backdrop_colour: lighting.surroundings.backdrop_uniform(),
            floor: lighting.surroundings.floor_uniform(),
            shadow_matrix: frame.map_or(Mat4::ZERO, |frame| frame.matrix),
            relative_from_light: frame.map_or(Mat4::ZERO, |frame| frame.relative_from_light),
            cross_matrix: cross.map_or(Mat4::ZERO, |cross| cross.matrix),
            relative_from_cross: cross.map_or(Mat4::ZERO, |cross| cross.relative_from_light),
            cross_frame: [
                cross.map_or(0.0, |cross| cross.normal_offset),
                0.0,
                0.0,
                0.0,
            ],
        }
    }

    pub(crate) fn crossing(&self) -> Self {
        Self {
            shadow_matrix: self.cross_matrix,
            relative_from_light: self.relative_from_cross,
            exposure_shadow: [
                self.exposure_shadow[0],
                self.exposure_shadow[1],
                self.exposure_shadow[2],
                self.cross_frame[0],
            ],
            ..*self
        }
    }

    pub(crate) fn shows_floor(&self) -> bool {
        self.floor[0] > 0.5
    }

    pub(crate) fn with_environment(self, environment: [f32; 4]) -> Self {
        Self {
            environment,
            ..self
        }
    }

    pub(crate) fn lit(&self) -> bool {
        self.exposure_shadow[0] > 0.0
    }

    pub(crate) fn needs_light_pass(&self) -> bool {
        self.exposure_shadow[1] > 0.0 || self.bounces()
    }

    pub(crate) fn bounces(&self) -> bool {
        self.bounce[0] > 0.0
    }

    pub(crate) fn write(&self, bytes: &mut impl Pack) {
        bytes
            .floats(&self.sun)
            .floats(&self.colour)
            .floats(&self.exposure_shadow)
            .floats(&self.bounce)
            .floats(&self.ground)
            .floats(&self.sky_view)
            .floats(&self.environment)
            .floats(&self.backdrop_colour)
            .floats(&self.floor)
            .mat4(self.shadow_matrix)
            .mat4(self.relative_from_light)
            .mat4(self.cross_matrix)
            .mat4(self.relative_from_cross)
            .floats(&self.cross_frame);
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
    peel_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    environment_sampler: wgpu::Sampler,
    stand_in_targets: LightTargets,
    stand_in_environment: EnvironmentViews,
    targets: Option<LightTargets>,
    environment: Option<BoundEnvironment>,
    refused_environment: Option<Arc<Environment>>,
    bind_group: wgpu::BindGroup,
    peel_groups: Option<Vec<wgpu::BindGroup>>,
    refused: bool,
}

struct BoundEnvironment {
    source: Arc<Environment>,
    views: EnvironmentViews,
}

struct LightTargets {
    depth: wgpu::TextureView,
    albedo: wgpu::TextureView,
    normals: wgpu::TextureView,
    lit_depth: wgpu::TextureView,
    peeled: Vec<wgpu::TextureView>,
    peel_depth: wgpu::TextureView,
    crossed: wgpu::TextureView,
    cross_depth: wgpu::TextureView,
}

impl LightTargets {
    fn new(device: &wgpu::Device, (side, cross_side): (u32, u32)) -> Self {
        Self {
            peeled: (0..PEEL_LAYERS)
                .map(|_| light_texture(device, "peeled depth", side, LIT_DEPTH_FORMAT))
                .collect(),
            peel_depth: light_texture(device, "peel depth", side, SHADOW_FORMAT),
            crossed: light_texture(device, "crossed depth", cross_side, LIT_DEPTH_FORMAT),
            cross_depth: light_texture(device, "cross depth", cross_side, SHADOW_FORMAT),
            depth: light_texture(device, "shadow map", side, SHADOW_FORMAT),
            albedo: light_texture(device, "lit albedo", side, BOUNCE_FORMAT),
            normals: light_texture(device, "lit normals", side, BOUNCE_FORMAT),
            lit_depth: light_texture(device, "lit depth", side, LIT_DEPTH_FORMAT),
        }
    }
}

pub(crate) enum LightPassReady {
    Ready,
    Refused,
    NewlyRefused,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct EnvironmentBound {
    pub rebound: bool,
    pub newly_refused: bool,
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
                bounce_entry(2, wgpu::TextureSampleType::Float { filterable: false }),
                bounce_entry(3, wgpu::TextureSampleType::Float { filterable: false }),
                bounce_entry(4, wgpu::TextureSampleType::Uint),
                bounce_entry(5, wgpu::TextureSampleType::Float { filterable: true }),
                bounce_entry(6, wgpu::TextureSampleType::Float { filterable: true }),
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                bounce_entry(PEELED_BINDING, wgpu::TextureSampleType::Uint),
                bounce_entry(PEELED_BINDING + 1, wgpu::TextureSampleType::Uint),
                bounce_entry(CROSSED_BINDING, wgpu::TextureSampleType::Uint),
            ],
        });
        let peel_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("previous layer"),
            entries: &[bounce_entry(
                PREVIOUS_LAYER_BINDING,
                wgpu::TextureSampleType::Uint,
            )],
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
        Self::with_layout(device, (layout, peel_layout), sampler)
    }

    fn with_layout(
        device: &wgpu::Device,
        (layout, peel_layout): (wgpu::BindGroupLayout, wgpu::BindGroupLayout),
        sampler: wgpu::Sampler,
    ) -> Self {
        let environment_sampler = environment_textures::sampler(device);
        let stand_in_targets = LightTargets::new(device, (1, 1));
        let stand_in_environment = environment_textures::stand_in(device);
        let bind_group = bind_group(
            device,
            &layout,
            (&sampler, &environment_sampler),
            (&stand_in_targets, &stand_in_environment),
        );
        Self {
            layout,
            peel_layout,
            sampler,
            environment_sampler,
            stand_in_targets,
            stand_in_environment,
            targets: None,
            environment: None,
            refused_environment: None,
            bind_group,
            peel_groups: None,
            refused: false,
        }
    }

    pub(crate) fn sibling(&self, device: &wgpu::Device) -> Self {
        Self::with_layout(
            device,
            (self.layout.clone(), self.peel_layout.clone()),
            self.sampler.clone(),
        )
    }

    pub(crate) fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    pub(crate) fn peel_layout(&self) -> &wgpu::BindGroupLayout {
        &self.peel_layout
    }

    fn rebind(&mut self, device: &wgpu::Device) {
        self.bind_group = bind_group(
            device,
            &self.layout,
            (&self.sampler, &self.environment_sampler),
            (
                self.targets.as_ref().unwrap_or(&self.stand_in_targets),
                self.environment
                    .as_ref()
                    .map_or(&self.stand_in_environment, |bound| &bound.views),
            ),
        );
    }

    pub(crate) fn ensure(&mut self, device: &wgpu::Device) -> LightPassReady {
        if self.targets.is_some() {
            return LightPassReady::Ready;
        }
        if self.refused {
            return LightPassReady::Refused;
        }
        let side = SHADOW_MAP_SIDE.min(device.limits().max_texture_dimension_2d);
        let cross_side = CROSS_MAP_SIDE.min(device.limits().max_texture_dimension_2d);
        let (targets, error) =
            gpu::scoped(device, || LightTargets::new(device, (side, cross_side)));
        match error {
            None => {
                self.peel_groups = Some(peel_groups(
                    device,
                    &self.peel_layout,
                    std::iter::once(&targets.lit_depth)
                        .chain(&targets.peeled)
                        .take(PEEL_LAYERS),
                ));
                self.targets = Some(targets);
                self.rebind(device);
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

    pub(crate) fn bind_environment(
        &mut self,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        wanted: Option<&Arc<Environment>>,
    ) -> EnvironmentBound {
        let Some(wanted) = wanted else {
            self.refused_environment = None;
            let rebound = self.environment.take().is_some();
            if rebound {
                self.rebind(device);
            }
            return EnvironmentBound {
                rebound,
                newly_refused: false,
            };
        };
        let held = self
            .environment
            .as_ref()
            .is_some_and(|bound| Arc::ptr_eq(&bound.source, wanted));
        let declined = self
            .refused_environment
            .as_ref()
            .is_some_and(|refused| Arc::ptr_eq(refused, wanted));
        if held || declined {
            return EnvironmentBound::default();
        }
        let (views, error) = gpu::scoped(device, || {
            environment_textures::upload(device, queue, wanted)
        });
        let views = views.filter(|_| error.is_none());
        let newly_refused = views.is_none();
        if let Some(error) = error {
            log::warn!("the graphics device refused the environment picture: {error}");
        }
        self.refused_environment = newly_refused.then(|| Arc::clone(wanted));
        self.environment = views.map(|views| BoundEnvironment {
            source: Arc::clone(wanted),
            views,
        });
        self.rebind(device);
        EnvironmentBound {
            rebound: true,
            newly_refused,
        }
    }

    pub(crate) fn environment_uniform(&self, turn: f32, stops: f32) -> [f32; 4] {
        let Some(bound) = &self.environment else {
            return [0.0; 4];
        };
        let gain = if stops.is_finite() { stops.exp2() } else { 1.0 };
        [
            if turn.is_finite() { turn } else { 0.0 },
            gain.clamp(f32::MIN_POSITIVE, MOST_ENVIRONMENT_GAIN),
            bound.source.level_count().saturating_sub(1) as f32,
            0.0,
        ]
    }

    pub(crate) fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    pub(crate) fn begin_pass<'a>(
        &self,
        encoder: &'a mut wgpu::CommandEncoder,
    ) -> Option<wgpu::RenderPass<'a>> {
        let targets = self.targets.as_ref()?;
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
                cleared(&targets.albedo),
                cleared(&targets.normals),
                cleared(&targets.lit_depth),
            ],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &targets.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(FARTHEST_SHADOW_DEPTH),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        }))
    }

    pub(crate) fn begin_peel_pass<'a>(
        &'a self,
        encoder: &'a mut wgpu::CommandEncoder,
        layer: usize,
    ) -> Option<(wgpu::RenderPass<'a>, &'a wgpu::BindGroup)> {
        let targets = self.targets.as_ref()?;
        let previous = self.peel_groups.as_ref()?.get(layer)?;
        let peeled = targets.peeled.get(layer)?;
        let pass = layer_pass(encoder, "peeled depth", peeled, &targets.peel_depth);
        Some((pass, previous))
    }

    pub(crate) fn begin_cross_pass<'a>(
        &'a self,
        encoder: &'a mut wgpu::CommandEncoder,
    ) -> Option<wgpu::RenderPass<'a>> {
        let targets = self.targets.as_ref()?;
        Some(layer_pass(
            encoder,
            "crossed depth",
            &targets.crossed,
            &targets.cross_depth,
        ))
    }
}

fn layer_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    label: &'static str,
    layer: &'a wgpu::TextureView,
    depth: &'a wgpu::TextureView,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: layer,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(FARTHEST_SHADOW_DEPTH),
                store: wgpu::StoreOp::Discard,
            }),
            stencil_ops: None,
        }),
        ..Default::default()
    })
}

fn peel_groups<'a>(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    previous_layers: impl Iterator<Item = &'a wgpu::TextureView>,
) -> Vec<wgpu::BindGroup> {
    previous_layers
        .map(|previous| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("previous layer"),
                layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: PREVIOUS_LAYER_BINDING,
                    resource: wgpu::BindingResource::TextureView(previous),
                }],
            })
        })
        .collect()
}

fn bounce_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
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
    (sampler, environment_sampler): (&wgpu::Sampler, &wgpu::Sampler),
    (targets, environment): (&LightTargets, &EnvironmentViews),
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
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&targets.lit_depth),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(&environment.radiance),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(&environment.irradiance),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::Sampler(environment_sampler),
            },
        ]
        .into_iter()
        .chain(
            targets
                .peeled
                .iter()
                .zip(PEELED_BINDING..)
                .map(|(layer, binding)| wgpu::BindGroupEntry {
                    binding,
                    resource: wgpu::BindingResource::TextureView(layer),
                })
                .chain([wgpu::BindGroupEntry {
                    binding: CROSSED_BINDING,
                    resource: wgpu::BindingResource::TextureView(&targets.crossed),
                }]),
        )
        .collect::<Vec<_>>(),
    })
}

pub(crate) fn peel_pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
) -> wgpu::RenderPipeline {
    layer_pipeline(
        device,
        module,
        (layout, buffers),
        ("peeled depth", "fs_peel"),
    )
}

fn layer_pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    (layout, buffers): (
        &wgpu::PipelineLayout,
        &[Option<wgpu::VertexBufferLayout<'_>>],
    ),
    (label, fragment): (&'static str, &'static str),
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
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
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fragment),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState::from(LIT_DEPTH_FORMAT))],
        }),
        multiview_mask: None,
        cache: None,
    })
}

pub(crate) fn depth_pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
) -> wgpu::RenderPipeline {
    layer_pipeline(
        device,
        module,
        (layout, buffers),
        ("crossed depth", "fs_depth"),
    )
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
                Some(wgpu::ColorTargetState::from(LIT_DEPTH_FORMAT)),
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
    fn a_light_following_the_view_turns_with_its_heading_and_rises_from_the_world_horizon() {
        let view = view();
        let level = |direction: Vector3| Vector3::new(direction.x, direction.y, 0.0).normalize();
        let light = |azimuth: f64, elevation: f64| {
            LightDirection::View { azimuth, elevation }.toward_light(&view)
        };

        let behind = light(0.0, 0.0);
        let overhead = light(0.3, std::f64::consts::FRAC_PI_2);
        let right = light(std::f64::consts::FRAC_PI_2, 0.0);

        assert!(behind.distance(level(-view.forward())) < 1e-12);
        assert!(overhead.distance(Vector3::Z) < 1e-12);
        assert!(right.distance(level(view.viewpoint().right())) < 1e-12);
    }

    #[test]
    fn looking_straight_down_a_light_following_the_view_still_lights_the_near_faces() {
        let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 100.0)
            .expect("the direction is not zero");
        let view = View::new(viewpoint, 800.0, 600.0);
        let near_side = Vector3::new(-view.viewpoint().up().x, -view.viewpoint().up().y, 0.0);

        let toward = LightDirection::View {
            azimuth: 0.0,
            elevation: 40f64.to_radians(),
        }
        .toward_light(&view);

        assert!(toward.dot(near_side) > 0.5, "{toward}");
        assert!(toward.z > 0.5, "{toward}");
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
        let frame = LightFrame::fitted(toward_light, eye, corners.iter().copied(), SHADOW_MAP_SIDE)
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
    fn the_cross_direction_sees_every_wall_the_sun_grazes_at_a_slant() {
        let suns = [
            Vector3::new(0.5, 0.0, 0.866),
            Vector3::new(0.0, 1.0, 0.2),
            Vector3::new(0.3, -0.4, 0.8),
            Vector3::new(0.0, 0.0, 1.0),
            Vector3::new(0.001, 0.0, 1.0),
        ];

        for sun in suns {
            let sun = sun.normalize();
            let cross = cross_direction(sun);
            let walls: Vec<Vector3> = [
                sun.cross(Vector3::Z).normalize_or(Vector3::X),
                Vector3::X,
                Vector3::Y,
            ]
            .into_iter()
            .filter(|wall| wall.dot(sun).abs() < 1e-2)
            .collect();

            assert!((cross.length() - 1.0).abs() < 1e-9, "{sun} {cross}");
            assert!(cross.dot(sun) > 0.5, "{sun} {cross}");
            for wall in walls {
                assert!(wall.dot(cross).abs() > 0.2, "{sun} {wall} {cross}");
            }
        }
    }

    #[test]
    fn no_caster_leaves_no_light_frame() {
        assert_eq!(
            LightFrame::fitted(
                Vector3::Z,
                Point3::ZERO,
                std::iter::empty(),
                SHADOW_MAP_SIDE
            ),
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
