use std::{
    fs,
    io::BufWriter,
    path::{Path, PathBuf},
};

use caditor_document::{Feature, FeatureId};
use caditor_render::{SurfaceTarget, ViewportFrame, ViewportRenderer};
use egui::{Key, Modifiers};
use tempfile::TempDir;

use super::{CAMERA_SETTLE, Harness, Painted};
use crate::{
    app::Workspace,
    editing::EditingCommand,
    export::ExportCommand,
    files::FileCommand,
    model::Action,
    panels::Renaming,
    preferences::{Preferences, PreferencesCommand, Theme},
    samples::Sample,
    selection::Pickable,
};

const OUTPUT: &str = "CADITOR_SCREENSHOTS";
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const SAMPLES: u32 = 4;

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl Gpu {
    fn open() -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .expect("a graphics adapter is available");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("the adapter opens a device");
        Self { device, queue }
    }
}

#[derive(Clone, Copy)]
struct Look {
    name: &'static str,
    theme: Theme,
    high_contrast: bool,
    scale: f32,
}

const LOOKS: [Look; 6] = [
    Look {
        name: "dark",
        theme: Theme::Dark,
        high_contrast: false,
        scale: 1.0,
    },
    Look {
        name: "light",
        theme: Theme::Light,
        high_contrast: false,
        scale: 1.0,
    },
    Look {
        name: "dark-contrast",
        theme: Theme::Dark,
        high_contrast: true,
        scale: 1.0,
    },
    Look {
        name: "light-contrast",
        theme: Theme::Light,
        high_contrast: true,
        scale: 1.0,
    },
    Look {
        name: "dark-200",
        theme: Theme::Dark,
        high_contrast: false,
        scale: 2.0,
    },
    Look {
        name: "light-150",
        theme: Theme::Light,
        high_contrast: false,
        scale: 1.5,
    },
];

impl Look {
    fn preferences(self, mut preferences: Preferences) -> Preferences {
        preferences.appearance.theme = self.theme;
        preferences.appearance.high_contrast = self.high_contrast;
        preferences.appearance.scale = self.scale;
        preferences
    }
}

impl Harness {
    fn styled(look: Look, dir: &Path, first_run: bool) -> Self {
        let mut workspace = if first_run {
            Workspace::with_preferences(Preferences::from_settings(
                caditor_file::Settings::default(),
            ))
        } else {
            Workspace::new()
        };
        workspace.preferences = look.preferences(workspace.preferences.clone());
        let mut harness =
            Self::starting(Some(dir), caditor_document::Document::default(), workspace);
        harness.painted = Some(Painted {
            shapes: Vec::new(),
            pixels_per_point: 1.0,
        });
        harness.settle();
        harness
    }

    fn open_sample(&mut self, sample: Sample) {
        let session = self.model.session();
        self.command(FileCommand::OpenSample(sample));
        self.wait_until("the sample opens", |harness| {
            harness.model.session() != session
        });
        self.settle();
        self.workspace.viewport.advance(CAMERA_SETTLE);
        self.let_animations_finish();
    }

    fn still(&mut self) {
        self.workspace.viewport.advance(CAMERA_SETTLE);
        self.let_animations_finish();
        self.settle();
    }

    fn screenshot(&mut self, gpu: &Gpu, path: &Path) {
        self.still();
        let Some(painted) = self.painted.as_ref() else {
            return;
        };
        let device = &gpu.device;
        let queue = &gpu.queue;
        let pixels_per_point = painted.pixels_per_point;
        let width = super::SCREEN.width() as u32;
        let height = super::SCREEN.height() as u32;
        let primitives = self
            .context
            .tessellate(painted.shapes.clone(), pixels_per_point);

        let mut egui =
            egui_wgpu::Renderer::new(device, FORMAT, egui_wgpu::RendererOptions::PREDICTABLE);
        for (id, delta) in self.textures.whole() {
            egui.update_texture(device, queue, id, &delta);
        }
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("screenshot"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());

        let mut viewport = ViewportRenderer::new(device, FORMAT, SAMPLES);
        let request = self.workspace.viewport.request(false);
        let frame =
            request
                .as_ref()
                .zip(self.workspace.viewport.scene())
                .map(|(request, scene)| ViewportFrame {
                    rect: request.rect,
                    view: &request.view,
                    scene,
                    pick_at: None,
                    pixels_per_point: request.pixels_per_point,
                });
        let _ = viewport.draw(
            device,
            queue,
            &mut encoder,
            &SurfaceTarget {
                view: &view,
                width,
                height,
            },
            frame.as_ref(),
        );

        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [width, height],
            pixels_per_point,
        };
        let prepared = egui.update_buffers(device, queue, &mut encoder, &primitives, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            egui.render(&mut pass, &primitives, &screen);
        }

        let pitch = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screenshot readback"),
            size: u64::from(pitch * height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(pitch),
                    rows_per_image: Some(height),
                },
            },
            target.size(),
        );
        queue.submit(prepared.into_iter().chain([encoder.finish()]));
        buffer.map_async(wgpu::MapMode::Read, .., |result| {
            result.expect("the readback maps");
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the device finishes");
        let mapped = buffer.get_mapped_range(..).expect("the readback is mapped");
        let row = width as usize * 4;
        let pixels: Vec<u8> = mapped
            .chunks(pitch as usize)
            .flat_map(|line| line.iter().take(row))
            .copied()
            .collect();

        let file = fs::File::create(path).expect("the screenshot file is created");
        let mut png = png::Encoder::new(BufWriter::new(file), width, height);
        png.set_color(png::ColorType::Rgba);
        png.set_depth(png::BitDepth::Eight);
        png.write_header()
            .and_then(|mut writer| writer.write_image_data(&pixels))
            .expect("the screenshot is written");
    }
}

fn shoot(harness: &mut Harness, gpu: &Gpu, out: &Path, scene: &str, look: Look) {
    harness.screenshot(gpu, &out.join(format!("{scene}-{}.png", look.name)));
}

#[test]
#[ignore = "writes screenshots of the interface to the directory named by CADITOR_SCREENSHOTS"]
fn screenshots() {
    let Some(out) = std::env::var_os(OUTPUT).map(PathBuf::from) else {
        panic!("set {OUTPUT} to the directory the screenshots go to");
    };
    fs::create_dir_all(&out).expect("the screenshot directory is created");
    let gpu = Gpu::open();
    let only = std::env::var("CADITOR_SCREENSHOT_LOOKS").ok();

    for look in LOOKS {
        if only
            .as_deref()
            .is_some_and(|only| !only.split(',').any(|name| name == look.name))
        {
            continue;
        }
        let dir = TempDir::new().expect("a temporary directory");

        let mut welcome = Harness::styled(look, dir.path(), true);
        shoot(&mut welcome, &gpu, &out, "welcome", look);
        drop(welcome);

        let mut empty = Harness::styled(look, dir.path(), false);
        shoot(&mut empty, &gpu, &out, "empty", look);
        drop(empty);

        let dir = TempDir::new().expect("a temporary directory");
        let mut model = Harness::styled(look, dir.path(), false);
        model.open_sample(Sample::Bracket);
        shoot(&mut model, &gpu, &out, "model", look);

        let extrusion = model
            .document()
            .features()
            .find(|feature| feature.kind.solid().is_some())
            .map(|feature| feature.id());
        if let Some(extrusion) = extrusion {
            model.perform(Action::Editing(EditingCommand::OpenSolid(extrusion)));
            model.settle();
            shoot(&mut model, &gpu, &out, "feature", look);
            model.perform(Action::Editing(EditingCommand::CloseSolid));
            model.settle();
        }

        model.key(Key::I, Modifiers::NONE);
        model.frame();
        shoot(&mut model, &gpu, &out, "measure", look);
        model.key(Key::I, Modifiers::NONE);
        model.frame();

        let sketch = model
            .document()
            .features()
            .find(|feature| feature.kind.sketch().is_some())
            .map(|feature| feature.id());
        if let Some(sketch) = sketch {
            model.edit(sketch);
            shoot(&mut model, &gpu, &out, "sketch", look);
            model.key(Key::Escape, Modifiers::NONE);
            model.key(Key::Escape, Modifiers::NONE);
            model.frame();
            model.perform(Action::Editing(EditingCommand::Finish));
            model.settle();
        }

        model.key(Key::P, Modifiers::COMMAND | Modifiers::SHIFT);
        model.show_new_windows();
        model.type_text("ex");
        shoot(&mut model, &gpu, &out, "palette", look);
        model.key(Key::Escape, Modifiers::NONE);
        model.frame();

        model.perform(Action::Preferences(PreferencesCommand::Show));
        shoot(&mut model, &gpu, &out, "preferences", look);
        model.key(Key::Escape, Modifiers::NONE);
        model.frame();

        model.command(FileCommand::Export(ExportCommand::Show));
        shoot(&mut model, &gpu, &out, "export", look);
        model.key(Key::Escape, Modifiers::NONE);
        model.frame();

        tree_scenes(&mut model, &gpu, &out, look);
    }
}

fn tree_scenes(model: &mut Harness, gpu: &Gpu, out: &Path, look: Look) {
    let rows: Vec<FeatureId> = model.document().features().map(Feature::id).collect();
    if let [first, second, ..] = rows.as_slice() {
        model.workspace.panels.choose_only(*first);
        model.workspace.panels.toggle_chosen(*second);
        model.frame();
        shoot(model, gpu, out, "tree-selection", look);

        model.workspace.panels.deleting = Some(vec![*first]);
        model.show_new_windows();
        shoot(model, gpu, out, "delete", look);
        model.key(Key::Escape, Modifiers::NONE);
        model.frame();

        let suppression = model.document().suppression(&[*first], true, "Suppress");
        model.perform(Action::Apply(suppression));
        model.settle();
        shoot(model, gpu, out, "failed", look);
        model.perform(Action::Undo);
        model.settle();

        model.workspace.panels.renaming = Some(Renaming {
            feature: *second,
            focus_pending: true,
        });
        model.frame();
        shoot(model, gpu, out, "rename", look);
        model.key(Key::Escape, Modifiers::NONE);
        model.frame();
    }

    let body = model
        .document()
        .features()
        .find(|feature| feature.kind.solid().is_some())
        .map(Feature::id);
    let Some(body) = body else {
        return;
    };
    let keys: Vec<_> = model
        .workspace
        .viewport
        .bodies()
        .get(body)
        .map(|mesh| mesh.vertices.iter().map(|vertex| vertex.key).collect())
        .unwrap_or_default();
    if let (Some(first), Some(last)) = (keys.first(), keys.last()) {
        model.key(Key::I, Modifiers::NONE);
        model.frame();
        model.select([first, last].map(|vertex| Pickable::Vertex {
            body,
            vertex: *vertex,
        }));
        model.wait_until("the vertices are measured", |harness| {
            harness.shows("Between them")
        });
        shoot(model, gpu, out, "measure-two", look);
        model.key(Key::I, Modifiers::NONE);
        model.frame();
    }
}
