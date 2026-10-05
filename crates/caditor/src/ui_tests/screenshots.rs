use std::{
    fs,
    io::BufWriter,
    path::{Path, PathBuf},
};

use caditor_document::{
    AxisReference, BlendKind, Datum, DatumAxis, Feature, FeatureId, FeatureKind, PrincipalAxis,
    SolidFeature,
};
use caditor_file::{JournalEntry, Start, Storage, StorageConfig};
use caditor_render::{SurfaceTarget, ViewportFrame, ViewportRenderer};
use egui::{Event, Key, Modifiers};
use tempfile::TempDir;

use super::{CAMERA_SETTLE, Harness, Painted, combine_nearly_touching_blocks};
use crate::{
    app::Workspace,
    blend_tools, datum_tools,
    editing::EditingCommand,
    export::ExportCommand,
    files::FileCommand,
    history::HistoryCommand,
    image_export::ImageCommand,
    mirror_tools,
    model::Action,
    panels::{Painting, Renaming},
    pattern_tools::{self, Shape},
    preferences::{Preferences, PreferencesCommand, PreferencesTab, Theme},
    reference_picking::{Picking, Slot},
    samples::Sample,
    scale_tools,
    selection::{Pickable, Selection},
    shell_tools, view_cube,
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
        combine_nearly_touching_blocks(&mut empty);
        empty.click("Show where");
        empty.frame();
        empty.workspace.viewport.advance(CAMERA_SETTLE);
        empty.frame();
        shoot(&mut empty, &gpu, &out, "boolean-failure", look);
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

        canvas_scenes(&mut model, &gpu, &out, look);
        tree_scenes(&mut model, &gpu, &out, look);
        feature_panel_scenes(&mut model, &gpu, &out, look);
        dialog_scenes(&gpu, &out, look);
    }
}

fn canvas_scenes(model: &mut Harness, gpu: &Gpu, out: &Path, look: Look) {
    let Some(viewport) = model.workspace.viewport.rect() else {
        return;
    };
    let cube = view_cube::area(viewport);
    model.events.push(Event::PointerMoved(
        cube.center_top() + egui::vec2(0.0, 40.0),
    ));
    model.frame();
    shoot(model, gpu, out, "cube-hover", look);
    model.events.push(Event::PointerMoved(viewport.center()));
    model.frame();

    let sketch = model
        .document()
        .features()
        .find(|feature| feature.kind.sketch().is_some())
        .map(|feature| feature.id());
    if let Some(sketch) = sketch {
        model.edit(sketch);
        model.use_tool(Key::R);
        shoot(model, gpu, out, "drawing", look);
        model.type_text("10");
        shoot(model, gpu, out, "typed-point", look);
        model.key(Key::Escape, Modifiers::NONE);
        model.frame();
        model.key(Key::Escape, Modifiers::NONE);
        model.frame();
        model.perform(Action::Editing(EditingCommand::Finish));
        model.settle();
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
    model.workspace.panels.painting = Some(Painting {
        body,
        focus_pending: true,
    });
    model.frame();
    model.frame();
    shoot(model, gpu, out, "body-appearance", look);
    model.workspace.panels.painting = None;
    model.frame();
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

fn shoot_open(harness: &mut Harness, gpu: &Gpu, out: &Path, scene: &str, look: Look) {
    harness.settle();
    shoot(harness, gpu, out, scene, look);
    harness.perform(Action::Editing(EditingCommand::CloseSolid));
    harness.settle();
}

fn open_kind(harness: &mut Harness, kind: impl Fn(&FeatureKind) -> bool) {
    let feature = harness
        .document()
        .features()
        .find(|feature| kind(&feature.kind))
        .map(|feature| feature.id());
    if let Some(feature) = feature {
        harness.perform(Action::Editing(EditingCommand::OpenSolid(feature)));
    }
}

fn perform_all(harness: &mut Harness, actions: Vec<Action>) {
    for action in actions {
        harness.perform(action);
    }
}

fn only(pickable: Pickable) -> Selection {
    let mut selection = Selection::default();
    selection.replace_with(pickable);
    selection
}

fn feature_panel_scenes(model: &mut Harness, gpu: &Gpu, out: &Path, look: Look) {
    open_kind(model, |kind| {
        matches!(kind.solid(), Some(SolidFeature::Extrude(_)))
    });
    shoot_open(model, gpu, out, "panel-extrude", look);

    let pickables: Vec<Pickable> = model.built().picks.pickables().collect();
    let edge = pickables.iter().find_map(|pickable| {
        let selection = only(*pickable);
        blend_tools::selected_edges(&selection).ok()
    });
    if let Some(source) = edge {
        let actions = blend_tools::create_actions(
            model.document(),
            model.model.evaluation(),
            BlendKind::Fillet,
            &source,
            model.model.length_unit(),
        );
        perform_all(model, actions);
        shoot_open(model, gpu, out, "panel-fillet", look);
    }

    let faces = pickables.iter().find_map(|pickable| {
        let selection = only(*pickable);
        let source = shell_tools::selected_faces(&model.model, &selection).ok()?;
        shell_tools::create(
            model.document(),
            model.model.evaluation(),
            &source,
            model.model.length_unit(),
        )
        .ok()
        .map(|_| source)
    });
    if let Some(source) = faces {
        let actions = shell_tools::create_actions(
            model.document(),
            model.model.evaluation(),
            &source,
            model.model.length_unit(),
        );
        perform_all(model, actions);
        shoot_open(model, gpu, out, "panel-shell", look);
    }

    let mirrored = pickables
        .iter()
        .find_map(|pickable| mirror_tools::source(&model.model, &only(*pickable)).ok());
    if let Some(source) = mirrored {
        let actions = mirror_tools::create_actions(&model.model, &source);
        perform_all(model, actions);
        shoot_open(model, gpu, out, "panel-mirror", look);
        let actions = scale_tools::create_actions(&model.model, source.body);
        perform_all(model, actions);
        shoot_open(model, gpu, out, "panel-scale", look);
    }

    for (shape, scene) in [
        (Shape::Linear, "panel-linear-pattern"),
        (Shape::Circular, "panel-circular-pattern"),
    ] {
        if let Ok(source) = pattern_tools::source(&model.model, &Selection::default(), None) {
            let actions = pattern_tools::create_actions(&model.model, shape, &source);
            perform_all(model, actions);
            shoot_open(model, gpu, out, scene, look);
        }
    }

    let end = model.document().features().len();
    if let Ok(plane) = datum_tools::plane_from_selection(&model.model, &Selection::default(), end) {
        let actions = datum_tools::create_actions(model.document(), Datum::Plane(plane));
        perform_all(model, actions);
        shoot_open(model, gpu, out, "panel-datum-plane", look);
    }
    let axis = Datum::Axis(DatumAxis::Along(AxisReference::Principal(PrincipalAxis::Z)));
    let actions = datum_tools::create_actions(model.document(), axis);
    perform_all(model, actions);
    if let Some(datum) = model.workspace.editing.solid() {
        model.settle();
        shoot(model, gpu, out, "panel-datum-axis", look);
        model.perform(Action::Editing(EditingCommand::Pick(Picking::new(
            datum,
            Slot::DatumBase,
        ))));
        shoot_open(model, gpu, out, "panel-picking", look);
    }

    let dir = TempDir::new().expect("a temporary directory");
    let mut spool = Harness::styled(look, dir.path(), false);
    spool.open_sample(Sample::Spool);
    open_kind(&mut spool, |kind| {
        matches!(kind.solid(), Some(SolidFeature::Revolve(_)))
    });
    shoot_open(&mut spool, gpu, out, "panel-revolve", look);
}

fn close_dialog(harness: &mut Harness) {
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.settle();
}

fn dialog_scenes(gpu: &Gpu, out: &Path, look: Look) {
    let dir = TempDir::new().expect("a temporary directory");
    let mut model = Harness::styled(look, dir.path(), false);
    model.open_sample(Sample::Plate);

    for tab in PreferencesTab::ALL {
        model.perform(Action::Preferences(PreferencesCommand::Tab(tab)));
        model.perform(Action::Preferences(PreferencesCommand::Show));
        let scene = format!("preferences-{}", tab.label().to_lowercase());
        shoot(&mut model, gpu, out, &scene, look);
        close_dialog(&mut model);
    }

    model.perform(Action::Preferences(PreferencesCommand::ShowShortcuts));
    shoot(&mut model, gpu, out, "shortcuts", look);
    close_dialog(&mut model);

    model.perform(Action::Preferences(PreferencesCommand::ShowAbout));
    shoot(&mut model, gpu, out, "about", look);
    close_dialog(&mut model);

    model.command(FileCommand::ExportImage(ImageCommand::Show));
    shoot(&mut model, gpu, out, "image-export", look);
    close_dialog(&mut model);

    let path = dir.path().join("plate.caditor");
    model.answer_dialog(Some(path.clone()));
    model.command(FileCommand::SaveAs);
    model.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_saving()
    });
    for width in ["45 mm", "50 mm"] {
        model.edit_width(width);
        model.command(FileCommand::Save);
        model.wait_until("the change is saved", |harness| !harness.model.is_dirty());
    }
    model.command(FileCommand::History(HistoryCommand::Show));
    model.wait_until("the versions are listed", |harness| {
        harness.shows("Restore")
    });
    shoot(&mut model, gpu, out, "history", look);
    close_dialog(&mut model);

    model.edit_width("55 mm");
    model.command(FileCommand::New);
    shoot(&mut model, gpu, out, "unsaved", look);
    close_dialog(&mut model);
    model.command(FileCommand::Save);
    model.wait_until("the change is saved", |harness| !harness.model.is_dirty());

    let damaged = dir.path().join("damaged.caditor");
    let bytes = caditor_file::encode(&super::sample_document().expect("the sample builds"))
        .expect("the sample encodes");
    std::fs::write(&damaged, super::damage_chunk(&bytes, 4)).expect("the damaged file is written");
    model.command(FileCommand::OpenPath(damaged.clone()));
    model.wait_until("the damaged file opens", |harness| {
        harness.model.path() == Some(damaged.as_path())
    });
    model.settle();
    shoot(&mut model, gpu, out, "report", look);
    close_dialog(&mut model);
    drop(model);

    let mut tip = Harness::styled(look, dir.path(), false);
    tip.workspace.preferences.onboarding.hints = true;
    tip.settle();
    shoot(&mut tip, gpu, out, "tip", look);
    drop(tip);

    let mut welcome = Harness::styled(look, dir.path(), true);
    welcome.wait_until("the recent files are read", |harness| {
        !harness.files.recent().is_empty()
    });
    shoot(&mut welcome, gpu, out, "welcome-recent", look);
    drop(welcome);

    let crash = TempDir::new().expect("a temporary directory");
    crashed_session(crash.path());
    let mut recovery = Harness::styled(look, crash.path(), false);
    recovery.wait_until("recovery is offered", |harness| {
        harness.files.has_recoverable()
    });
    recovery.perform(Action::File(FileCommand::ShowRecovery));
    shoot(&mut recovery, gpu, out, "recovery", look);
}

fn crashed_session(dir: &Path) {
    let base = super::sample_document().expect("the sample builds");
    let width = base.parameter_named("width").expect("a width").id();
    let change = caditor_document::Transaction::single(
        "Edit width",
        caditor_document::Edit::SetParameterExpression {
            id: width,
            expression: base.parse("55 mm").expect("a length"),
        },
    );
    let crashed = Storage::spawn(
        StorageConfig {
            recovery_dir: Some(dir.join("recovery")),
            ..StorageConfig::default()
        },
        Start {
            file: None,
            on_disk: None,
            loaded_with_problems: false,
            base,
            folded: 0,
            entries: vec![JournalEntry::Apply(change)],
            replaces: None,
            after: None,
        },
        || {},
    )
    .expect("the storage starts");
    assert!(crashed.flusher().flush(super::FILE_TIMEOUT));
    assert!(crashed.close(false).wait(super::FILE_TIMEOUT));
}
