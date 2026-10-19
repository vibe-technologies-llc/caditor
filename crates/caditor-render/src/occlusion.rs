use caditor_geometry::{Point3, Vector3};

use crate::{
    camera::{Projection, View},
    gpu::{self, Pack},
    viewport::DEPTH_FORMAT,
};

pub(crate) const SURFACE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Uint;
pub(crate) const OCCLUSION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
pub(crate) const MOST_PIXELS: u32 = 96;
const REACH_OF_SCENE: f64 = 0.04;
const NEAREST_DEPTH: f32 = 0.0;
const SURFACES_BINDING: u32 = 8;
const RAW_BINDING: u32 = 9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct OcclusionUniform {
    occlusion: [f32; 4],
    frame: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
}

impl OcclusionUniform {
    pub(crate) const NONE: Self = Self {
        occlusion: [0.0; 4],
        frame: [0.0; 4],
        right: [0.0; 4],
        up: [0.0; 4],
    };

    pub(crate) fn of(
        strength: f32,
        view: &View,
        casters: impl IntoIterator<Item = [Point3; 8]>,
    ) -> Self {
        let mut low = Vector3::splat(f64::INFINITY);
        let mut high = Vector3::splat(f64::NEG_INFINITY);
        for corner in casters.into_iter().flatten() {
            low = low.min(corner);
            high = high.max(corner);
        }
        let reach = (high - low).length() * REACH_OF_SCENE;
        let occludes = strength.is_finite() && strength > 0.0 && reach.is_finite() && reach > 0.0;
        let pixel_size = match view.projection() {
            Projection::Perspective => view.units_per_pixel_at(1.0),
            Projection::Orthographic => view.units_per_pixel_at(view.viewpoint().distance),
        };
        let viewpoint = view.viewpoint();
        let right = viewpoint.right().as_vec3();
        let up = viewpoint.up().as_vec3();
        Self {
            occlusion: match occludes {
                true => [strength, reach as f32, pixel_size as f32, 0.0],
                false => [0.0; 4],
            },
            frame: [0.0, 0.0, MOST_PIXELS as f32, 0.0],
            right: [right.x, right.y, right.z, 0.0],
            up: [up.x, up.y, up.z, 0.0],
        }
    }

    #[must_use]
    pub(crate) fn read_from(mut self, margin: u32) -> Self {
        self.frame[0] = margin as f32;
        self.frame[1] = margin as f32;
        self
    }

    pub(crate) fn occludes(&self) -> bool {
        self.occlusion[0] > 0.0
    }

    pub(crate) fn write(&self, bytes: &mut impl Pack) {
        bytes
            .floats(&self.occlusion)
            .floats(&self.frame)
            .floats(&self.right)
            .floats(&self.up);
    }
}

pub(crate) struct OcclusionLayouts {
    pub(crate) surfaces: wgpu::BindGroupLayout,
    pub(crate) raw: wgpu::BindGroupLayout,
    pub(crate) shading: wgpu::BindGroupLayout,
}

pub(crate) struct Occlusion {
    layouts: OcclusionLayouts,
    stand_in: wgpu::BindGroup,
    made: Option<Made>,
    refused: Option<(u32, u32)>,
}

struct Made {
    size: (u32, u32),
    depth: wgpu::TextureView,
    surfaces: wgpu::TextureView,
    raw: wgpu::TextureView,
    occlusion: wgpu::TextureView,
    surfaces_group: wgpu::BindGroup,
    raw_group: wgpu::BindGroup,
    shading_group: wgpu::BindGroup,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PassArea {
    pub(crate) viewport: [f32; 4],
    pub(crate) scissor: [u32; 4],
}

pub(crate) enum OcclusionReady {
    Ready,
    Refused,
    NewlyRefused,
}

pub(crate) struct OcclusionPasses<'a> {
    pub(crate) view: &'a wgpu::BindGroup,
    pub(crate) occlusion: &'a wgpu::RenderPipeline,
    pub(crate) blur: &'a wgpu::RenderPipeline,
    pub(crate) area: Option<PassArea>,
}

impl Occlusion {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let layouts = OcclusionLayouts {
            surfaces: layout(device, "occlusion surfaces", &[(SURFACES_BINDING, true)]),
            raw: layout(
                device,
                "raw occlusion",
                &[(SURFACES_BINDING, true), (RAW_BINDING, false)],
            ),
            shading: layout(device, "occlusion", &[(0, false), (1, true)]),
        };
        let stand_in = shading_group(
            device,
            &layouts.shading,
            &texture(device, "occlusion", (1, 1), OCCLUSION_FORMAT),
            &texture(device, "occlusion surfaces", (1, 1), SURFACE_FORMAT),
        );
        Self {
            layouts,
            stand_in,
            made: None,
            refused: None,
        }
    }

    pub(crate) fn sibling(&self, device: &wgpu::Device) -> Self {
        let layouts = OcclusionLayouts {
            surfaces: self.layouts.surfaces.clone(),
            raw: self.layouts.raw.clone(),
            shading: self.layouts.shading.clone(),
        };
        let stand_in = shading_group(
            device,
            &layouts.shading,
            &texture(device, "occlusion", (1, 1), OCCLUSION_FORMAT),
            &texture(device, "occlusion surfaces", (1, 1), SURFACE_FORMAT),
        );
        Self {
            layouts,
            stand_in,
            made: None,
            refused: None,
        }
    }

    pub(crate) fn layouts(&self) -> &OcclusionLayouts {
        &self.layouts
    }

    pub(crate) fn ensure(&mut self, device: &wgpu::Device, size: (u32, u32)) -> OcclusionReady {
        if self.made.as_ref().is_some_and(|made| made.size == size) {
            return OcclusionReady::Ready;
        }
        if self.refused == Some(size) {
            return OcclusionReady::Refused;
        }
        self.made = None;
        let (made, error) = gpu::scoped(device, || Made::new(device, &self.layouts, size));
        match error {
            None => {
                self.made = Some(made);
                self.refused = None;
                OcclusionReady::Ready
            }
            Some(error) => {
                let (width, height) = size;
                log::warn!(
                    "the graphics device refused {width}x{height} ambient occlusion targets, so the rendered view has none: {error}"
                );
                self.refused = Some(size);
                OcclusionReady::NewlyRefused
            }
        }
    }

    pub(crate) fn bind_group(&self) -> &wgpu::BindGroup {
        self.made
            .as_ref()
            .map_or(&self.stand_in, |made| &made.shading_group)
    }

    pub(crate) fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        passes: &OcclusionPasses<'_>,
        draw_surfaces: impl FnOnce(&mut wgpu::RenderPass<'_>),
    ) {
        let Some(made) = &self.made else {
            return;
        };
        let place = |pass: &mut wgpu::RenderPass<'_>| {
            if let Some(PassArea {
                viewport: [x, y, width, height],
                scissor: [left, top, across, down],
            }) = passes.area
            {
                pass.set_viewport(x, y, width, height, 0.0, 1.0);
                pass.set_scissor_rect(left, top, across, down);
            }
            pass.set_bind_group(0, passes.view, &[]);
        };

        let mut surfaces = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("occlusion surfaces"),
            color_attachments: &[Some(cleared(&made.surfaces, wgpu::Color::TRANSPARENT))],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &made.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(NEAREST_DEPTH),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        place(&mut surfaces);
        draw_surfaces(&mut surfaces);
        drop(surfaces);

        let mut occlusion = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ambient occlusion"),
            color_attachments: &[Some(cleared(&made.raw, wgpu::Color::WHITE))],
            ..Default::default()
        });
        place(&mut occlusion);
        occlusion.set_pipeline(passes.occlusion);
        occlusion.set_bind_group(1, &made.surfaces_group, &[]);
        occlusion.draw(0..3, 0..1);
        drop(occlusion);

        let mut blur = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ambient occlusion blur"),
            color_attachments: &[Some(cleared(&made.occlusion, wgpu::Color::WHITE))],
            ..Default::default()
        });
        place(&mut blur);
        blur.set_pipeline(passes.blur);
        blur.set_bind_group(1, &made.raw_group, &[]);
        blur.draw(0..3, 0..1);
    }
}

impl Made {
    fn new(device: &wgpu::Device, layouts: &OcclusionLayouts, size: (u32, u32)) -> Self {
        let surfaces = texture(device, "occlusion surfaces", size, SURFACE_FORMAT);
        let raw = texture(device, "raw occlusion", size, OCCLUSION_FORMAT);
        let occlusion = texture(device, "occlusion", size, OCCLUSION_FORMAT);
        Self {
            size,
            depth: texture(device, "occlusion depth", size, DEPTH_FORMAT),
            surfaces_group: group(device, &layouts.surfaces, &[(SURFACES_BINDING, &surfaces)]),
            raw_group: group(
                device,
                &layouts.raw,
                &[(SURFACES_BINDING, &surfaces), (RAW_BINDING, &raw)],
            ),
            shading_group: shading_group(device, &layouts.shading, &occlusion, &surfaces),
            surfaces,
            raw,
            occlusion,
        }
    }
}

fn cleared(view: &wgpu::TextureView, color: wgpu::Color) -> wgpu::RenderPassColorAttachment<'_> {
    wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(color),
            store: wgpu::StoreOp::Store,
        },
    }
}

fn layout(
    device: &wgpu::Device,
    label: &'static str,
    entries: &[(u32, bool)],
) -> wgpu::BindGroupLayout {
    let entries: Vec<wgpu::BindGroupLayoutEntry> = entries
        .iter()
        .map(|(binding, unsigned)| wgpu::BindGroupLayoutEntry {
            binding: *binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: match unsigned {
                    true => wgpu::TextureSampleType::Uint,
                    false => wgpu::TextureSampleType::Float { filterable: false },
                },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        })
        .collect();
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &entries,
    })
}

fn group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    views: &[(u32, &wgpu::TextureView)],
) -> wgpu::BindGroup {
    let entries: Vec<wgpu::BindGroupEntry<'_>> = views
        .iter()
        .map(|(binding, view)| wgpu::BindGroupEntry {
            binding: *binding,
            resource: wgpu::BindingResource::TextureView(view),
        })
        .collect();
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("occlusion"),
        layout,
        entries: &entries,
    })
}

fn shading_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    occlusion: &wgpu::TextureView,
    surfaces: &wgpu::TextureView,
) -> wgpu::BindGroup {
    group(device, layout, &[(0, occlusion), (1, surfaces)])
}

fn texture(
    device: &wgpu::Device,
    label: &'static str,
    (width, height): (u32, u32),
    format: wgpu::TextureFormat,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
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

pub(crate) fn screen_pipeline(
    device: &wgpu::Device,
    (module, layout): (&wgpu::ShaderModule, &wgpu::PipelineLayout),
    label: &'static str,
    fragment: &'static str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_screen"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fragment),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState::from(OCCLUSION_FORMAT))],
        }),
        multiview_mask: None,
        cache: None,
    })
}

pub(crate) fn surfaces_pipeline(
    device: &wgpu::Device,
    (module, layout): (&wgpu::ShaderModule, &wgpu::PipelineLayout),
    buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
    bias: wgpu::DepthBiasState,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("occlusion surfaces"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_mesh"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers,
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: wgpu::StencilState::default(),
            bias,
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs_surface"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState::from(SURFACE_FORMAT))],
        }),
        multiview_mask: None,
        cache: None,
    })
}

pub(crate) fn floor_surfaces_pipeline(
    device: &wgpu::Device,
    (module, layout): (&wgpu::ShaderModule, &wgpu::PipelineLayout),
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("occlusion floor"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_sky"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs_floor_surface"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState::from(SURFACE_FORMAT))],
        }),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Viewpoint;

    #[test]
    fn occlusion_reaches_a_share_of_the_scene_and_nothing_or_no_strength_gives_none() {
        let view = View::new(
            Viewpoint::looking_from(Vector3::new(1.0, -1.0, 1.0), Point3::ZERO, 200.0)
                .expect("the direction is not zero"),
            800.0,
            600.0,
        );
        let cube = [Point3::ZERO, Point3::splat(100.0)];
        let corners = || {
            caditor_geometry::Aabb::from_points(cube)
                .map(|bounds| bounds.corners())
                .into_iter()
        };

        let uniform = OcclusionUniform::of(1.0, &view, corners());
        let reach = f64::from(uniform.occlusion[1]);

        assert!(uniform.occludes());
        assert!((reach - 100.0 * 3f64.sqrt() * REACH_OF_SCENE).abs() < 1e-3);
        assert!((f64::from(uniform.occlusion[2]) - view.units_per_pixel_at(1.0)).abs() < 1e-9);
        let off = OcclusionUniform::of(0.0, &view, corners());

        assert!(!off.occludes());
        assert_eq!(off.right, uniform.right);
        assert!(!OcclusionUniform::of(1.0, &view, std::iter::empty()).occludes());
        assert_eq!(uniform.read_from(96).frame[0], 96.0);
    }
}
