use crate::scene::ViewportRect;

const KEPT_BINDING: u32 = 0;
const COPY_TRIANGLE_CORNERS: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Shown {
    pub rect: Option<ViewportRect>,
    pub grid: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Holding {
    Nothing,
    Unsubmitted(Shown),
    Submitted(Shown),
}

pub(crate) struct KeptView {
    view: wgpu::TextureView,
    linear: Option<wgpu::TextureView>,
    bind_group: wgpu::BindGroup,
    pub holding: Holding,
}

impl KeptView {
    pub(crate) fn new(
        device: &wgpu::Device,
        copy: &ViewCopy,
        format: wgpu::TextureFormat,
        extent: wgpu::Extent3d,
        linear_format: Option<wgpu::TextureFormat>,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("kept viewport"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: linear_format.as_slice(),
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("kept viewport"),
            layout: &copy.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: KEPT_BINDING,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        Self {
            linear: linear_format.map(|format| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    format: Some(format),
                    ..Default::default()
                })
            }),
            view,
            bind_group,
            holding: Holding::Nothing,
        }
    }

    pub(crate) fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub(crate) fn linear(&self) -> Option<&wgpu::TextureView> {
        self.linear.as_ref()
    }
}

impl Holding {
    pub(crate) fn holds(self, shown: Shown) -> bool {
        self == Self::Submitted(shown)
    }

    pub(crate) fn drawn(&mut self, shown: Shown) {
        *self = Self::Unsubmitted(shown);
    }

    pub(crate) fn after_submit(&mut self) {
        if let Self::Unsubmitted(shown) = *self {
            *self = Self::Submitted(shown);
        }
    }

    pub(crate) fn abandon_unsubmitted(&mut self) {
        if matches!(self, Self::Unsubmitted(_)) {
            *self = Self::Nothing;
        }
    }
}

#[derive(Clone)]
pub(crate) struct ViewCopy {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}

impl ViewCopy {
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kept viewport"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: KEPT_BINDING,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let module = device.create_shader_module(wgpu::include_wgsl!("kept.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("kept viewport copy"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("kept viewport copy"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_copy"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_copy"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self { layout, pipeline }
    }

    pub(crate) fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        kept: &KeptView,
        target: &wgpu::TextureView,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("kept viewport copy"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &kept.bind_group, &[]);
        pass.draw(0..COPY_TRIANGLE_CORNERS, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_copy_shader_parses_and_validates() {
        let module = naga::front::wgsl::parse_str(include_str!("kept.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .unwrap();
    }

    #[test]
    fn a_kept_view_is_reused_only_once_its_frame_was_submitted_for_the_same_rect_and_grid() {
        let rect = ViewportRect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 80.0,
        };
        let shown = Shown {
            rect: Some(rect),
            grid: true,
        };
        let moved = Shown {
            rect: Some(ViewportRect { x: 10.0, ..rect }),
            ..shown
        };
        let mut holding = Holding::Nothing;

        holding.drawn(shown);
        let unsubmitted = holding.holds(shown);
        holding.after_submit();
        let submitted = holding.holds(shown);
        let elsewhere = holding.holds(moved);
        let without_grid = holding.holds(Shown {
            grid: false,
            ..shown
        });
        holding.drawn(moved);
        holding.abandon_unsubmitted();
        holding.after_submit();
        let abandoned = holding.holds(moved) || holding.holds(shown);

        assert!(!unsubmitted);
        assert!(submitted);
        assert!(!elsewhere);
        assert!(!without_grid);
        assert!(!abandoned);
    }
}
