use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use caditor_geometry::{Plane, Point3, Vector3};
use glam::DVec2;

use crate::{
    SurfaceSize,
    camera::{Projection, View, Viewpoint},
    gpu::{self, Bytes, DeviceLoss, GrowableBuffer},
    image::{Background, Image, ImagePoll, ImageRequest, PendingImage},
    mesh::{FaceStyle, MeshFace, MeshInstance, MeshPoint, ShadedMesh},
    scene::{
        Batch, Color, Fill, Grid, Layer, Line, Marker, PickId, PickResult, Scene, Stroke,
        ViewportRect,
    },
    settings::{Msaa, Shading},
    viewport::{SurfaceTarget, ViewportFrame, ViewportRenderer, Work},
};

const SIZE: u32 = 200;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const ROW_PITCH: u32 = 1024;
const LINE_COLOR: Color = Color::from_rgb8(250, 20, 20);

const REQUIRE_GPU: &str = "CADITOR_REQUIRE_GPU";

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    gpu_with(wgpu::Limits::default())
}

fn gpu_with(limits: wgpu::Limits) -> Option<(wgpu::Device, wgpu::Queue)> {
    let found = device(limits);
    if found.is_none() {
        assert!(
            std::env::var_os(REQUIRE_GPU).is_none(),
            "no graphics adapter is available, and {REQUIRE_GPU} says the offscreen tests must run"
        );
        eprintln!("no graphics adapter available, skipping the offscreen test");
    }
    found
}

fn device(limits: wgpu::Limits) -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: limits,
        ..wgpu::DeviceDescriptor::default()
    }))
    .ok()
}

fn scene() -> Scene {
    Scene {
        meshes: Vec::new(),
        grid: None,
        batches: vec![Arc::new(Batch {
            lines: vec![Line {
                start: Point3::new(-20.0, 0.0, 0.0),
                end: Point3::new(20.0, 0.0, 0.0),
                color: LINE_COLOR,
                width: 3.0,
                layer: Layer::Model,
                pick: PickId::from_index(0),
                stroke: Stroke::Solid,
            }],
            fills: vec![Fill::convex(
                &[
                    Point3::new(-30.0, -30.0, 0.0),
                    Point3::new(30.0, -30.0, 0.0),
                    Point3::new(30.0, 30.0, 0.0),
                    Point3::new(-30.0, 30.0, 0.0),
                ],
                Color::from_rgba8(0, 0, 255, 40),
                Layer::Reference,
                PickId::from_index(1),
            )],
            markers: vec![Marker {
                position: Point3::new(10.0, 10.0, 0.0),
                color: Color::from_rgb8(255, 255, 255),
                diameter: 7.0,
                layer: Layer::Model,
                pick: PickId::from_index(2),
            }],
        })],
    }
}

struct Rendered {
    pick: PickResult,
    pixels: Vec<u8>,
}

fn full_frame<'a>(view: &'a View, scene: &'a Scene, pick_at: DVec2) -> ViewportFrame<'a> {
    ViewportFrame {
        rect: ViewportRect {
            x: 0.0,
            y: 0.0,
            width: SIZE as f32,
            height: SIZE as f32,
        },
        view,
        scene,
        pick_at: Some(pick_at),
        pixels_per_point: 1.0,
    }
}

fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    view: &View,
    scene: &Scene,
    pick_at: DVec2,
) -> Rendered {
    render_frame(device, queue, &full_frame(view, scene, pick_at))
}

fn render_frame(device: &wgpu::Device, queue: &wgpu::Queue, frame: &ViewportFrame<'_>) -> Rendered {
    let mut renderer = ViewportRenderer::new(device, FORMAT, 4);
    render_with(&mut renderer, device, queue, frame)
}

fn render_with(
    renderer: &mut ViewportRenderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &ViewportFrame<'_>,
) -> Rendered {
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen target"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("offscreen readback"),
        size: u64::from(ROW_PITCH * SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    renderer.draw(
        device,
        queue,
        &mut encoder,
        &SurfaceTarget {
            view: &target_view,
            width: SIZE,
            height: SIZE,
        },
        Some(frame),
    );
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(ROW_PITCH),
                rows_per_image: Some(SIZE),
            },
        },
        target.size(),
    );
    queue.submit([encoder.finish()]);
    renderer.picking().after_submit();
    readback.map_async(wgpu::MapMode::Read, .., |result| result.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();

    let crate::PickPoll::Ready(pick) = renderer.picking().poll(device) else {
        panic!("the pick should be read back");
    };
    let pixels = readback.get_mapped_range(..).unwrap().to_vec();
    Rendered { pick, pixels }
}

fn pixel(rendered: &Rendered, at: DVec2) -> [u8; 4] {
    let offset = at.y as usize * ROW_PITCH as usize + at.x as usize * 4;
    rendered.pixels[offset..offset + 4].try_into().unwrap()
}

fn channel_coverage(
    rendered: &Rendered,
    channel: usize,
    pixels: impl Iterator<Item = DVec2>,
) -> f64 {
    let background = (crate::viewport::BACKGROUND.g * 255.0).round();
    pixels
        .map(|at| {
            (f64::from(pixel(rendered, at)[channel]) - background).max(0.0) / (255.0 - background)
        })
        .sum()
}

fn column(x: f64, around: f64, reach: f64) -> impl Iterator<Item = DVec2> {
    let rows = (around - reach).floor() as i64..=(around + reach).ceil() as i64;
    rows.map(move |row| DVec2::new(x, row as f64))
}

fn row(y: f64, around: f64, reach: f64) -> impl Iterator<Item = DVec2> {
    let columns = (around - reach).floor() as i64..=(around + reach).ceil() as i64;
    columns.map(move |column| DVec2::new(column as f64, y))
}

fn batch_of(scene: &mut Scene) -> &mut Batch {
    Arc::make_mut(&mut scene.batches[0])
}

fn line_and_marker() -> Scene {
    Scene {
        batches: vec![Arc::new(Batch {
            lines: vec![Line {
                start: Point3::new(-20.0, 0.0, 0.0),
                end: Point3::new(20.0, 0.0, 0.0),
                color: LINE_COLOR,
                width: 3.0,
                layer: Layer::Model,
                pick: PickId::from_index(0),
                stroke: Stroke::Solid,
            }],
            markers: vec![Marker {
                position: Point3::new(10.0, 20.0, 0.0),
                color: Color::from_rgb8(255, 255, 255),
                diameter: 7.0,
                layer: Layer::Model,
                pick: PickId::from_index(1),
            }],
            ..Batch::default()
        })],
        ..Scene::default()
    }
}

#[test]
fn lines_and_markers_keep_their_size_in_points_at_every_scale() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 100.0).unwrap();
    let view = View::new(viewpoint, f64::from(SIZE), f64::from(SIZE));
    let scene = line_and_marker();
    let across_line = view.project(Point3::new(-10.0, 0.0, 0.0)).unwrap();
    let marker = view.project(Point3::new(10.0, 20.0, 0.0)).unwrap();
    let measure = |pixels_per_point: f32| {
        let rendered = render_frame(
            &device,
            &queue,
            &ViewportFrame {
                pixels_per_point,
                ..full_frame(&view, &scene, across_line)
            },
        );
        let line_width = channel_coverage(
            &rendered,
            0,
            column(across_line.x.floor(), across_line.y, 12.0),
        );
        let marker_width = channel_coverage(&rendered, 1, row(marker.y.floor(), marker.x, 20.0));
        (line_width, marker_width)
    };

    let (line_at_one, marker_at_one) = measure(1.0);
    let (line_at_two, marker_at_two) = measure(2.0);

    assert!((line_at_one - 3.0).abs() < 0.6, "{line_at_one}");
    assert!((line_at_two - 6.0).abs() < 0.8, "{line_at_two}");
    assert!((marker_at_one - 7.0).abs() < 1.0, "{marker_at_one}");
    assert!((marker_at_two - 14.0).abs() < 1.5, "{marker_at_two}");
}

#[test]
fn a_dashed_line_leaves_gaps_that_still_pick_it() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 100.0).unwrap();
    let view = View::new(viewpoint, f64::from(SIZE), f64::from(SIZE));
    let mut scene = line_and_marker();
    batch_of(&mut scene).markers.clear();
    let from = view.project(Point3::new(-15.0, 0.0, 0.0)).unwrap();
    let to = view.project(Point3::new(15.0, 0.0, 0.0)).unwrap();
    let lit_along = |scene: &Scene, pick_at: DVec2| {
        let rendered = render(&device, &queue, &view, scene, pick_at);
        let lit: Vec<bool> = row(from.y.floor(), (from.x + to.x) / 2.0, (to.x - from.x) / 2.0)
            .map(|at| pixel(&rendered, at)[0] > 128)
            .collect();
        (lit, rendered.pick)
    };

    let (solid, _) = lit_along(&scene, from);
    batch_of(&mut scene).lines[0].stroke = Stroke::Dashed { along: 0.0 };
    let (dashed, _) = lit_along(&scene, from);
    let drawn = dashed.iter().filter(|lit| **lit).count() as f64 / dashed.len() as f64;
    let dashes = dashed.windows(2).filter(|pair| pair[0] && !pair[1]).count();
    let gap = dashed.iter().position(|lit| !lit).unwrap();
    let in_gap = DVec2::new(from.x.floor() + gap as f64, from.y.floor());
    let (_, pick) = lit_along(&scene, in_gap);
    let hit = pick
        .hits
        .iter()
        .find(|hit| Some(hit.id) == PickId::from_index(0))
        .unwrap();

    assert!(solid.iter().all(|lit| *lit));
    assert!((0.5..0.7).contains(&drawn), "{drawn}");
    assert!(dashes >= 8, "{dashes}");
    assert!(hit.offset_points < 1.5, "{hit:?}");
}

#[test]
fn picks_reach_as_far_in_points_at_a_doubled_scale() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 100.0).unwrap();
    let view = View::new(viewpoint, f64::from(SIZE), f64::from(SIZE));
    let scene = line_and_marker();
    let above_line = view.project(Point3::new(-10.0, 0.0, 0.0)).unwrap() - DVec2::new(0.0, 12.0);
    let line = PickId::from_index(0).unwrap();
    let pick = |pixels_per_point: f32| {
        render_frame(
            &device,
            &queue,
            &ViewportFrame {
                pixels_per_point,
                ..full_frame(&view, &scene, above_line)
            },
        )
        .pick
    };

    let at_one = pick(1.0);
    let at_two = pick(2.0);
    let hit = at_two.hits.iter().find(|hit| hit.id == line).unwrap();

    assert!(at_one.hits.iter().all(|hit| hit.id != line), "{at_one:?}");
    assert!(
        (3.5..=5.5).contains(&hit.offset_points),
        "{}",
        hit.offset_points
    );
    assert!(hit.position.distance(Point3::new(-10.0, 0.0, 0.0)) < 1.0);
}

#[test]
fn draws_and_picks_a_line_over_a_fill() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 100.0).unwrap();
    let view = View::new(viewpoint, f64::from(SIZE), f64::from(SIZE));
    let on_line = view.project(Point3::new(5.0, 0.0, 0.0)).unwrap();
    let off_line = view.project(Point3::new(5.0, 20.0, 0.0)).unwrap();

    let rendered = render(&device, &queue, &view, &scene(), on_line);

    let nearest = rendered.pick.hits[0];
    assert_eq!(nearest.id, PickId::from_index(0).unwrap());
    assert_eq!(nearest.offset_points, 0.0);
    assert!(nearest.position.distance(Point3::new(5.0, 0.0, 0.0)) < 0.5);
    assert!(
        rendered
            .pick
            .hits
            .iter()
            .any(|hit| hit.id == PickId::from_index(1).unwrap())
    );
    assert!(
        rendered
            .pick
            .hits
            .iter()
            .all(|hit| hit.id != PickId::from_index(2).unwrap())
    );

    let [red, green, blue, _] = pixel(&rendered, on_line);
    assert!(
        red > 200 && green < 80 && blue < 80,
        "line pixel was {red} {green} {blue}"
    );
    let [red, _, blue, _] = pixel(&rendered, off_line);
    assert!(blue > red, "fill pixel was not tinted blue");

    let rendered = render(
        &device,
        &queue,
        &view,
        &scene(),
        view.project(Point3::new(10.0, 12.0, 0.0)).unwrap(),
    );
    let marker = rendered
        .pick
        .hits
        .iter()
        .find(|hit| hit.id == PickId::from_index(2).unwrap())
        .unwrap();
    assert!(marker.offset_points > 0.0 && marker.offset_points < 7.5);
}

fn box_mesh(half: f64) -> ShadedMesh {
    let face = |normal: Vector3| {
        let (u, v) = normal.any_orthonormal_pair();
        let corner = |a: f64, b: f64| MeshPoint {
            position: Point3::ZERO + (normal + u * a + v * b) * half,
            normal,
        };
        MeshFace {
            points: vec![
                corner(-1.0, -1.0),
                corner(1.0, -1.0),
                corner(1.0, 1.0),
                corner(-1.0, 1.0),
            ],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
        }
    };
    ShadedMesh::new(
        [
            Vector3::X,
            Vector3::NEG_X,
            Vector3::Y,
            Vector3::NEG_Y,
            Vector3::Z,
            Vector3::NEG_Z,
        ]
        .map(face),
    )
}

#[test]
fn draws_shaded_faces_that_hide_what_is_behind_them_and_picks_the_face_in_front() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    draws_and_picks_a_box(&device, &queue);
}

#[test]
fn a_downlevel_device_without_vertex_storage_draws_and_picks_faces() {
    let Some((device, queue)) = gpu_with(wgpu::Limits::downlevel_webgl2_defaults()) else {
        return;
    };

    assert_eq!(device.limits().max_storage_buffers_per_shader_stage, 0);
    draws_and_picks_a_box(&device, &queue);
}

#[test]
fn a_lost_device_is_reported_and_a_new_one_draws() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let Some(lost) = opened_device(&instance) else {
        return;
    };
    let woken = Arc::new(AtomicBool::new(false));
    let wake = Arc::clone(&woken);
    let loss = DeviceLoss::watch(
        &lost.device,
        Arc::new(move || wake.store(true, Ordering::SeqCst)),
    );

    assert!(!loss.is_lost());

    lost.device.destroy();
    let _ = lost.device.poll(wgpu::PollType::wait_indefinitely());

    assert!(loss.is_lost());
    assert!(woken.load(Ordering::SeqCst));

    let reopened = opened_device(&instance).unwrap();
    let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 100.0).unwrap();
    let view = View::new(viewpoint, f64::from(SIZE), f64::from(SIZE));
    let on_line = view.project(Point3::new(5.0, 0.0, 0.0)).unwrap();

    let rendered = render(&reopened.device, &reopened.queue, &view, &scene(), on_line);

    assert_eq!(rendered.pick.hits[0].id, PickId::from_index(0).unwrap());
    let [red, green, _, _] = pixel(&rendered, on_line);
    assert!(red > 200 && green < 80, "line pixel was {red} {green}");
}

fn opened_device(instance: &wgpu::Instance) -> Option<gpu::OpenedDevice> {
    let opened = pollster::block_on(gpu::open_device(instance, None)).ok();
    if opened.is_none() {
        assert!(
            std::env::var_os(REQUIRE_GPU).is_none(),
            "no graphics adapter is available, and {REQUIRE_GPU} says the offscreen tests must run"
        );
    }
    opened
}

fn draws_and_picks_a_box(device: &wgpu::Device, queue: &wgpu::Queue) {
    let mesh = Arc::new(box_mesh(20.0));
    let styles: Vec<FaceStyle> = (0..6)
        .map(|index| FaceStyle {
            color: Color::from_rgb8(40, 200, 40),
            pick: PickId::from_index(10 + index),
        })
        .collect();
    let scene = Scene {
        meshes: vec![MeshInstance {
            mesh,
            faces: styles,
        }],
        batches: vec![Arc::new(Batch {
            lines: vec![Line {
                start: Point3::new(-50.0, 0.0, 0.0),
                end: Point3::new(50.0, 0.0, 0.0),
                color: LINE_COLOR,
                width: 3.0,
                layer: Layer::Model,
                pick: PickId::from_index(0),
                stroke: Stroke::Solid,
            }],
            ..Batch::default()
        })],
        ..Scene::default()
    };
    let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 150.0).unwrap();
    let view = View::new(viewpoint, f64::from(SIZE), f64::from(SIZE));
    let on_top = view.project(Point3::new(0.0, 10.0, 20.0)).unwrap();
    let hidden_line = view.project(Point3::new(5.0, 0.0, 20.0)).unwrap();
    let visible_line = view.project(Point3::new(40.0, 0.0, 0.0)).unwrap();

    let rendered = render(device, queue, &view, &scene, on_top);

    let nearest = rendered.pick.hits[0];
    assert_eq!(nearest.id, PickId::from_index(14).unwrap());
    assert_eq!(nearest.offset_points, 0.0);
    assert!(nearest.position.distance(Point3::new(0.0, 10.0, 20.0)) < 0.5);
    let [red, green, blue, _] = pixel(&rendered, on_top);
    assert!(
        green > 60 && green > red * 2 && green > blue * 2,
        "face pixel was {red} {green} {blue}"
    );
    let [red, green, _, _] = pixel(&rendered, hidden_line);
    assert!(
        green > red,
        "the line inside the box showed through its top"
    );
    let [red, green, _, _] = pixel(&rendered, visible_line);
    assert!(
        red > 200 && green < 80,
        "the line outside the box was hidden"
    );

    let rendered = render(device, queue, &view, &scene, hidden_line);
    assert!(
        rendered
            .pick
            .hits
            .iter()
            .all(|hit| hit.id != PickId::from_index(0).unwrap())
    );
}

#[test]
fn face_colours_and_the_eye_follow_every_frame_with_one_renderer() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let mut renderer = ViewportRenderer::new(&device, FORMAT, 4);
    let mesh = Arc::new(box_mesh(20.0));
    let painted = |color: Color| Scene {
        meshes: vec![MeshInstance {
            mesh: Arc::clone(&mesh),
            faces: vec![FaceStyle { color, pick: None }; 6],
        }],
        ..Scene::default()
    };
    let green = painted(Color::from_rgb8(40, 200, 40));
    let red = painted(Color::from_rgb8(200, 40, 40));
    let centred = View::new(
        Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 150.0).unwrap(),
        f64::from(SIZE),
        f64::from(SIZE),
    );
    let aside = View::new(
        Viewpoint::looking_from(Vector3::Z, Point3::new(60.0, 0.0, 0.0), 150.0).unwrap(),
        f64::from(SIZE),
        f64::from(SIZE),
    );
    let middle = DVec2::splat(f64::from(SIZE) / 2.0);
    let mut shown = |view: &View, scene: &Scene| {
        let rendered = render_with(
            &mut renderer,
            &device,
            &queue,
            &full_frame(view, scene, middle),
        );
        pixel(&rendered, middle)
    };
    let greenish = |[red, green, blue, _]: [u8; 4]| green > 2 * red && green > 2 * blue;
    let reddish = |[red, green, blue, _]: [u8; 4]| red > 2 * green && red > 2 * blue;

    let first = shown(&centred, &green);
    let repainted = shown(&centred, &red);
    let again = shown(&centred, &red);
    let moved_away = shown(&aside, &red);
    let back = shown(&centred, &green);

    assert!(greenish(first), "{first:?}");
    assert!(reddish(repainted), "{repainted:?}");
    assert!(reddish(again), "{again:?}");
    assert!(is_background(moved_away), "{moved_away:?}");
    assert!(greenish(back), "{back:?}");
}

#[test]
fn faces_that_cannot_be_picked_still_hide_what_is_behind_them_from_picking() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let scene = Scene {
        meshes: vec![MeshInstance {
            mesh: Arc::new(box_mesh(20.0)),
            faces: vec![
                FaceStyle {
                    color: Color::from_rgb8(90, 90, 90),
                    pick: None,
                };
                6
            ],
        }],
        batches: vec![Arc::new(Batch {
            lines: vec![Line {
                start: Point3::new(-50.0, 0.0, 0.0),
                end: Point3::new(50.0, 0.0, 0.0),
                color: LINE_COLOR,
                width: 3.0,
                layer: Layer::Model,
                pick: PickId::from_index(0),
                stroke: Stroke::Solid,
            }],
            ..Batch::default()
        })],
        ..Scene::default()
    };
    let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 150.0).unwrap();
    let view = View::new(viewpoint, f64::from(SIZE), f64::from(SIZE));
    let hidden_line = view.project(Point3::new(5.0, 0.0, 20.0)).unwrap();
    let visible_line = view.project(Point3::new(40.0, 0.0, 0.0)).unwrap();

    let behind = render(&device, &queue, &view, &scene, hidden_line);
    let beside = render(&device, &queue, &view, &scene, visible_line);

    assert!(behind.pick.hits.is_empty(), "{:?}", behind.pick.hits);
    assert_eq!(beside.pick.hits[0].id, PickId::from_index(0).unwrap());
}

#[test]
fn the_front_layer_draws_and_picks_over_faces_in_front_of_it() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let scene = Scene {
        meshes: vec![MeshInstance {
            mesh: Arc::new(box_mesh(20.0)),
            faces: vec![
                FaceStyle {
                    color: Color::from_rgb8(90, 90, 90),
                    pick: None,
                };
                6
            ],
        }],
        grid: None,
        batches: vec![Arc::new(Batch {
            lines: vec![Line {
                start: Point3::new(-50.0, 0.0, 0.0),
                end: Point3::new(50.0, 0.0, 0.0),
                color: LINE_COLOR,
                width: 3.0,
                layer: Layer::Front,
                pick: PickId::from_index(0),
                stroke: Stroke::Solid,
            }],
            markers: vec![Marker {
                position: Point3::new(-15.0, 0.0, 0.0),
                color: Color::from_rgb8(255, 255, 255),
                diameter: 9.0,
                layer: Layer::Front,
                pick: PickId::from_index(1),
            }],
            fills: vec![square_fill(-5.0, 10.0, Layer::Front, 2)],
        })],
    };
    let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 150.0).unwrap();
    let view = View::new(viewpoint, f64::from(SIZE), f64::from(SIZE));
    let line_inside = view.project(Point3::new(5.0, 0.0, 0.0)).unwrap();
    let marker_inside = view.project(Point3::new(-15.0, 0.0, 0.0)).unwrap();
    let fill_inside = view.project(Point3::new(5.0, 6.0, -5.0)).unwrap();

    let on_line = render(&device, &queue, &view, &scene, line_inside);
    let on_marker = render(&device, &queue, &view, &scene, marker_inside);
    let on_fill = render(&device, &queue, &view, &scene, fill_inside);

    let [red, green, blue, _] = pixel(&on_line, line_inside);
    assert!(
        red > 230 && green < 40 && blue < 40,
        "the line inside the box was hidden or tinted: {red} {green} {blue}"
    );
    let [red, green, blue, _] = pixel(&on_line, marker_inside);
    assert!(
        red > 200 && green > 200 && blue > 200,
        "the marker inside the box was hidden: {red} {green} {blue}"
    );
    let [red, _, blue, _] = pixel(&on_line, fill_inside);
    assert!(blue > red + 10, "the fill inside the box was hidden");
    assert_eq!(on_line.pick.hits[0].id, PickId::from_index(0).unwrap());
    assert_eq!(on_line.pick.hits[0].offset_points, 0.0);
    assert!(
        on_line.pick.hits[0]
            .position
            .distance(Point3::new(5.0, 0.0, 0.0))
            < 0.5
    );
    assert!(
        on_marker
            .pick
            .hits
            .iter()
            .any(|hit| hit.id == PickId::from_index(1).unwrap() && hit.offset_points == 0.0)
    );
    assert_eq!(on_fill.pick.hits[0].id, PickId::from_index(2).unwrap());
    assert_eq!(on_fill.pick.hits[0].offset_points, 0.0);
}

fn square_fill(z: f64, half: f64, layer: Layer, index: usize) -> Fill {
    Fill::convex(
        &[
            Point3::new(-half, -half, z),
            Point3::new(half, -half, z),
            Point3::new(half, half, z),
            Point3::new(-half, half, z),
        ],
        Color::from_rgba8(0, 0, 255, 40),
        layer,
        PickId::from_index(index),
    )
}

#[test]
fn reference_fills_are_picked_only_where_nothing_else_is() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let top_face = PickId::from_index(14).unwrap();
    let styles: Vec<FaceStyle> = (0..6)
        .map(|index| FaceStyle {
            color: Color::from_rgb8(40, 200, 40),
            pick: PickId::from_index(10 + index),
        })
        .collect();
    let scene = Scene {
        meshes: vec![MeshInstance {
            mesh: Arc::new(box_mesh(20.0)),
            faces: styles,
        }],
        batches: vec![Arc::new(Batch {
            fills: vec![
                square_fill(40.0, 60.0, Layer::Reference, 1),
                square_fill(30.0, 60.0, Layer::Reference, 2),
                square_fill(20.0, 5.0, Layer::Model, 3),
            ],
            ..Batch::default()
        })],
        ..Scene::default()
    };
    let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 150.0).unwrap();
    let view = View::new(viewpoint, f64::from(SIZE), f64::from(SIZE));
    let first_hit = |at: Point3| {
        render(&device, &queue, &view, &scene, view.project(at).unwrap())
            .pick
            .hits
            .first()
            .map(|hit| (hit.id, hit.position))
    };

    let (id, position) = first_hit(Point3::new(12.0, 12.0, 20.0)).unwrap();
    assert_eq!(id, top_face);
    assert!(position.distance(Point3::new(12.0, 12.0, 20.0)) < 0.5);

    let (id, _) = first_hit(Point3::new(0.0, 0.0, 20.0)).unwrap();
    assert_eq!(id, PickId::from_index(3).unwrap());

    let (id, position) = first_hit(Point3::new(45.0, 45.0, 40.0)).unwrap();
    assert_eq!(id, PickId::from_index(1).unwrap());
    assert!((position.z - 40.0).abs() < 0.5, "{position:?}");
}

#[test]
fn a_pick_whose_frame_was_never_submitted_fails_and_the_next_one_is_read() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 100.0).unwrap();
    let view = View::new(viewpoint, f64::from(SIZE), f64::from(SIZE));
    let on_line = view.project(Point3::new(5.0, 0.0, 0.0)).unwrap();
    let mut renderer = ViewportRenderer::new(&device, FORMAT, 4);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen target"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let scene = scene();
    let draw = |renderer: &mut ViewportRenderer| {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        renderer.draw(
            &device,
            &queue,
            &mut encoder,
            &SurfaceTarget {
                view: &target_view,
                width: SIZE,
                height: SIZE,
            },
            Some(&ViewportFrame {
                rect: ViewportRect {
                    x: 0.0,
                    y: 0.0,
                    width: SIZE as f32,
                    height: SIZE as f32,
                },
                view: &view,
                scene: &scene,
                pick_at: Some(on_line),
                pixels_per_point: 1.0,
            }),
        );
        encoder
    };

    drop(draw(&mut renderer));
    let pending = renderer.picking().poll(&device);
    renderer.picking().abandon_unsubmitted();
    let abandoned = renderer.picking().poll(&device);
    let encoder = draw(&mut renderer);
    queue.submit([encoder.finish()]);
    renderer.picking().after_submit();
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let read = renderer.picking().poll(&device);

    assert_eq!(pending, crate::PickPoll::Pending);
    assert_eq!(abandoned, crate::PickPoll::Failed);
    assert!(matches!(read, crate::PickPoll::Ready(pick) if !pick.hits.is_empty()));
}

#[test]
fn a_viewport_of_no_size_keeps_its_meshes_until_the_scene_drops_them() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let mut renderer = ViewportRenderer::new(&device, FORMAT, 4);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen target"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 150.0).unwrap();
    let view = View::new(viewpoint, f64::from(SIZE), f64::from(SIZE));
    let mut draw = |scene: &Scene, size: f32| {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        renderer.draw(
            &device,
            &queue,
            &mut encoder,
            &SurfaceTarget {
                view: &target_view,
                width: SIZE,
                height: SIZE,
            },
            Some(&ViewportFrame {
                rect: ViewportRect {
                    x: 0.0,
                    y: 0.0,
                    width: size,
                    height: size,
                },
                view: &view,
                scene,
                pick_at: None,
                pixels_per_point: 1.0,
            }),
        );
        queue.submit([encoder.finish()]);
    };
    let mesh = Arc::new(box_mesh(20.0));
    let scene = Scene {
        meshes: vec![MeshInstance {
            mesh: Arc::clone(&mesh),
            faces: vec![
                FaceStyle {
                    color: Color::from_rgb8(40, 200, 40),
                    pick: None,
                };
                6
            ],
        }],
        ..Scene::default()
    };

    draw(&scene, SIZE as f32);
    drop(scene);
    assert_eq!(Arc::strong_count(&mesh), 2);

    draw(&Scene::default(), 0.0);
    assert_eq!(Arc::strong_count(&mesh), 2);

    draw(&Scene::default(), SIZE as f32);
    assert_eq!(Arc::strong_count(&mesh), 1);
}

#[test]
fn a_buffer_shrinks_back_once_it_has_stayed_mostly_empty_for_a_while() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let mut buffer = GrowableBuffer::new(&device, "test", wgpu::BufferUsages::VERTEX);
    let mut large = Bytes::default();
    large.floats(&[1.0; 100_000]);
    let mut small = Bytes::default();
    small.floats(&[1.0; 10]);

    buffer.upload(&device, &queue, &large, 4);
    let grown = buffer.size();
    assert!(grown >= large.len());

    for _ in 0..GrowableBuffer::SHRINK_AFTER_UPLOADS - 1 {
        buffer.upload(&device, &queue, &small, 4);
    }
    buffer.upload(&device, &queue, &large, 4);
    assert_eq!(buffer.size(), grown);

    for _ in 0..GrowableBuffer::SHRINK_AFTER_UPLOADS {
        buffer.upload(&device, &queue, &small, 4);
    }
    assert_eq!(buffer.size(), GrowableBuffer::INITIAL_SIZE);
}

fn is_background([red, green, blue, _]: [u8; 4]) -> bool {
    let expected = [
        crate::viewport::BACKGROUND.r,
        crate::viewport::BACKGROUND.g,
        crate::viewport::BACKGROUND.b,
    ]
    .map(|channel| (channel * 255.0).round());
    [red, green, blue]
        .into_iter()
        .zip(expected)
        .all(|(actual, expected)| (f64::from(actual) - expected).abs() <= 1.0)
}

fn looking_down(distance: f64, width: f64, height: f64) -> View {
    let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, distance).unwrap();
    View::new(viewpoint, width, height)
}

#[test]
fn a_viewport_away_from_the_corner_draws_and_picks_inside_its_rect_only() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let rect = ViewportRect {
        x: 60.0,
        y: 40.0,
        width: 120.0,
        height: 100.0,
    };
    let corner = DVec2::new(f64::from(rect.x), f64::from(rect.y));
    let view = looking_down(100.0, f64::from(rect.width), f64::from(rect.height));
    let mut scene = line_and_marker();
    batch_of(&mut scene)
        .fills
        .push(square_fill(-1.0, 500.0, Layer::Reference, 5));
    let on_line = view.project(Point3::new(-10.0, 0.0, 0.0)).unwrap();
    let marker = view.project(Point3::new(10.0, 20.0, 0.0)).unwrap();

    let rendered = render_frame(
        &device,
        &queue,
        &ViewportFrame {
            rect,
            view: &view,
            scene: &scene,
            pick_at: Some(on_line),
            pixels_per_point: 1.0,
        },
    );
    let nearest = rendered.pick.hits[0];

    assert_eq!(nearest.id, PickId::from_index(0).unwrap());
    assert_eq!(nearest.offset_points, 0.0);
    assert!(nearest.position.distance(Point3::new(-10.0, 0.0, 0.0)) < 0.5);
    let [red, green, blue, _] = pixel(&rendered, corner + on_line);
    assert!(
        red > 200 && green < 80 && blue < 80,
        "line pixel was {red} {green} {blue}"
    );
    let [red, green, blue, _] = pixel(&rendered, corner + marker);
    assert!(
        red > 200 && green > 200 && blue > 200,
        "marker pixel was {red} {green} {blue}"
    );
    let [red, _, blue, _] = pixel(&rendered, corner + DVec2::new(5.0, 5.0));
    assert!(blue > red, "the fill did not reach the viewport's corner");
    for outside in [
        DVec2::new(10.0, 10.0),
        DVec2::new(59.0, 100.0),
        DVec2::new(181.0, 100.0),
        DVec2::new(100.0, 39.0),
        DVec2::new(100.0, 141.0),
    ] {
        assert!(
            is_background(pixel(&rendered, outside)),
            "{outside} outside the viewport was drawn on"
        );
    }
}

#[test]
fn markers_are_round_and_picked_only_where_they_cover() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let view = looking_down(100.0, f64::from(SIZE), f64::from(SIZE));
    let scene = Scene {
        batches: vec![Arc::new(Batch {
            markers: vec![Marker {
                position: Point3::ZERO,
                color: Color::from_rgb8(255, 255, 255),
                diameter: 9.0,
                layer: Layer::Model,
                pick: PickId::from_index(0),
            }],
            ..Batch::default()
        })],
        ..Scene::default()
    };
    let center = view.project(Point3::ZERO).unwrap();
    let marker = PickId::from_index(0).unwrap();

    let at_center = render(&device, &queue, &view, &scene, center);
    let diagonal = render(
        &device,
        &queue,
        &view,
        &scene,
        center + DVec2::new(6.0, 6.0),
    );
    let beside = render(
        &device,
        &queue,
        &view,
        &scene,
        center + DVec2::new(4.0, 0.0),
    );

    let [red, green, blue, _] = pixel(&at_center, center);
    assert!(red > 240 && green > 240 && blue > 240);
    assert!(is_background(pixel(
        &at_center,
        center + DVec2::new(4.0, 4.0)
    )));
    assert!(is_background(pixel(
        &at_center,
        center + DVec2::new(6.0, 0.0)
    )));
    assert_eq!(at_center.pick.hits[0].id, marker);
    assert_eq!(at_center.pick.hits[0].offset_points, 0.0);
    assert!(at_center.pick.hits[0].position.distance(Point3::ZERO) < 0.5);
    let offset = |rendered: &Rendered| {
        rendered
            .pick
            .hits
            .iter()
            .find(|hit| hit.id == marker)
            .unwrap()
            .offset_points
    };
    assert!(
        (2.0..5.0).contains(&offset(&diagonal)),
        "{}",
        offset(&diagonal)
    );
    assert_eq!(offset(&beside), 0.0);
}

#[test]
fn a_marker_with_no_colour_draws_nothing_but_is_still_picked() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let view = looking_down(100.0, f64::from(SIZE), f64::from(SIZE));
    let unmarked = PickId::from_index(1).unwrap();
    let under = Scene {
        batches: vec![Arc::new(Batch {
            fills: vec![Fill::convex(
                &[
                    Point3::new(-10.0, -10.0, 0.0),
                    Point3::new(10.0, -10.0, 0.0),
                    Point3::new(10.0, 10.0, 0.0),
                    Point3::new(-10.0, 10.0, 0.0),
                ],
                Color::from_rgb8(0, 0, 255),
                Layer::Model,
                PickId::from_index(0),
            )],
            ..Batch::default()
        })],
        ..Scene::default()
    };
    let mut over = under.clone();
    batch_of(&mut over).markers.push(Marker {
        position: Point3::ZERO,
        color: Color::from_rgba8(0, 0, 0, 0),
        diameter: 9.0,
        layer: Layer::Model,
        pick: Some(unmarked),
    });
    let center = view.project(Point3::ZERO).unwrap();

    let without = render(&device, &queue, &view, &under, center);
    let with = render(&device, &queue, &view, &over, center);

    assert_eq!(pixel(&with, center), pixel(&without, center));
    assert!(pixel(&with, center)[2] > 200);
    assert_eq!(with.pick.hits[0].id, unmarked);
}

#[test]
fn the_grid_draws_its_major_lines_and_is_never_picked() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let view = looking_down(100.0, f64::from(SIZE), f64::from(SIZE));
    let scene = Scene {
        grid: Some(Grid {
            plane: Plane::XY,
            color: Color::from_rgb8(255, 255, 255),
        }),
        ..Scene::default()
    };
    let on_major = view.project(Point3::new(10.0, 5.0, 0.0)).unwrap();
    let mid_cell = view.project(Point3::new(5.0, 5.0, 0.0)).unwrap();

    let rendered = render(&device, &queue, &view, &scene, on_major);
    let brightest = (-1..=1)
        .map(|dx| pixel(&rendered, on_major + DVec2::new(f64::from(dx), 0.0))[1])
        .max()
        .unwrap();

    assert!(brightest > 100, "the major grid line was {brightest}");
    assert!(is_background(pixel(&rendered, mid_cell)));
    assert!(rendered.pick.hits.is_empty(), "{:?}", rendered.pick.hits);
}

#[test]
fn lines_crossing_the_near_plane_are_cut_there_and_lines_behind_the_eye_vanish() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let view = looking_down(100.0, f64::from(SIZE), f64::from(SIZE));
    let crossing = PickId::from_index(0).unwrap();
    let line = |start: Point3, end: Point3, index: usize| Line {
        start,
        end,
        color: LINE_COLOR,
        width: 3.0,
        layer: Layer::Model,
        pick: PickId::from_index(index),
        stroke: Stroke::Solid,
    };
    let scene = Scene {
        batches: vec![Arc::new(Batch {
            lines: vec![
                line(
                    Point3::new(-10.0, 0.0, 0.0),
                    Point3::new(-10.0, 0.0, 150.0),
                    0,
                ),
                line(
                    Point3::new(10.0, 0.0, 120.0),
                    Point3::new(10.0, 5.0, 180.0),
                    1,
                ),
            ],
            markers: vec![Marker {
                position: Point3::new(0.0, 0.0, 150.0),
                color: Color::from_rgb8(255, 255, 255),
                diameter: 9.0,
                layer: Layer::Model,
                pick: PickId::from_index(2),
            }],
            ..Batch::default()
        })],
        ..Scene::default()
    };
    let near_the_eye = Point3::new(-10.0, 0.0, 50.0);
    let visible = view.project(near_the_eye).unwrap();
    let start = view.project(Point3::new(-10.0, 0.0, 0.0)).unwrap();

    let rendered = render(&device, &queue, &view, &scene, visible);
    let behind = render(
        &device,
        &queue,
        &view,
        &scene,
        DVec2::new(f64::from(SIZE) * 0.5, f64::from(SIZE) * 0.5),
    );

    let [red, green, _, _] = pixel(&rendered, visible);
    assert!(
        red > 200 && green < 80,
        "the line in front of the eye was hidden"
    );
    let [red, green, _, _] = pixel(&rendered, start - DVec2::new(2.0, 0.0));
    assert!(red > 200 && green < 80, "the line's visible end was hidden");
    for column in (start.x.ceil() as u32 + 3)..SIZE {
        assert!(
            is_background(pixel(&rendered, DVec2::new(f64::from(column), start.y))),
            "column {column} beyond the line's visible end was drawn on"
        );
    }
    let hit = rendered.pick.hits[0];
    assert_eq!(hit.id, crossing);
    assert!(hit.position.distance(near_the_eye) < 1.0, "{hit:?}");
    assert!(behind.pick.hits.is_empty(), "{:?}", behind.pick.hits);
}

#[test]
fn faces_that_cannot_be_picked_hide_faces_and_reference_fills_as_pickable_ones_do() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let unpickable = MeshInstance {
        mesh: Arc::new(box_mesh(20.0)),
        faces: vec![
            FaceStyle {
                color: Color::from_rgb8(90, 90, 90),
                pick: None,
            };
            6
        ],
    };
    let behind = MeshInstance {
        mesh: Arc::new(box_mesh(10.0)),
        faces: (0..6)
            .map(|index| FaceStyle {
                color: Color::from_rgb8(40, 200, 40),
                pick: PickId::from_index(10 + index),
            })
            .collect(),
    };
    let scene = Scene {
        meshes: vec![unpickable, behind],
        batches: vec![Arc::new(Batch {
            fills: vec![square_fill(60.0, 80.0, Layer::Reference, 1)],
            ..Batch::default()
        })],
        ..Scene::default()
    };
    let view = looking_down(200.0, f64::from(SIZE), f64::from(SIZE));
    let first_hit = |at: Point3| {
        render(&device, &queue, &view, &scene, view.project(at).unwrap())
            .pick
            .hits
            .first()
            .map(|hit| hit.id)
    };

    assert_eq!(first_hit(Point3::new(0.0, 0.0, 20.0)), None);
    assert_eq!(
        first_hit(Point3::new(30.0, 30.0, 60.0)),
        PickId::from_index(1)
    );
}

#[test]
fn a_failed_readback_replaces_the_pick_buffer_and_the_next_pick_is_read() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let view = looking_down(100.0, f64::from(SIZE), f64::from(SIZE));
    let on_line = view.project(Point3::new(5.0, 0.0, 0.0)).unwrap();
    let scene = scene();
    let mut renderer = ViewportRenderer::new(&device, FORMAT, 4);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen target"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let frame = full_frame(&view, &scene, on_line);
    let pick = |renderer: &mut ViewportRenderer, lose_readback: bool| {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        renderer.draw(
            &device,
            &queue,
            &mut encoder,
            &SurfaceTarget {
                view: &target_view,
                width: SIZE,
                height: SIZE,
            },
            Some(&frame),
        );
        queue.submit([encoder.finish()]);
        renderer.picking().after_submit();
        if lose_readback {
            renderer.picking().destroy_readback();
        }
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        renderer.picking().poll(&device)
    };

    let failed = pick(&mut renderer, true);
    let read = pick(&mut renderer, false);

    assert_eq!(failed, crate::PickPoll::Failed);
    assert!(
        matches!(&read, crate::PickPoll::Ready(pick) if pick.hits.first().map(|hit| hit.id) == PickId::from_index(0)),
        "{read:?}"
    );
}

fn strip_mesh(quads: u32, length: f64) -> ShadedMesh {
    let step = length / f64::from(quads);
    let faces = (0..quads).map(|quad| {
        let x = -length * 0.5 + f64::from(quad) * step;
        let corner = |dx: f64, y: f64| MeshPoint {
            position: Point3::new(x + dx, y, 0.0),
            normal: Vector3::Z,
        };
        MeshFace {
            points: vec![
                corner(0.0, -5.0),
                corner(step, -5.0),
                corner(step, 5.0),
                corner(0.0, 5.0),
            ],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
        }
    });
    ShadedMesh::new(faces)
}

#[test]
fn a_scene_larger_than_a_buffer_draws_what_fits_and_splits_its_meshes() {
    const BUFFER_LIMIT: u64 = 256 * 1024;
    let Some((device, queue)) = gpu_with(wgpu::Limits {
        max_buffer_size: BUFFER_LIMIT,
        max_texture_dimension_2d: 256,
        ..wgpu::Limits::default()
    }) else {
        return;
    };
    let quads = 3000;
    let mesh = Arc::new(strip_mesh(quads, 160.0));
    let styles = (0..quads as usize)
        .map(|index| FaceStyle {
            color: Color::from_rgb8(40, 200, 40),
            pick: PickId::from_index(100 + index),
        })
        .collect();
    let many = 2 * BUFFER_LIMIT as usize / 52;
    let lines = (0..many)
        .map(|index| Line {
            start: Point3::new(-40.0, 20.0 + index as f64 * 1e-3, 1.0),
            end: Point3::new(40.0, 20.0 + index as f64 * 1e-3, 1.0),
            color: LINE_COLOR,
            width: 3.0,
            layer: Layer::Model,
            pick: PickId::from_index(index),
            stroke: Stroke::Solid,
        })
        .collect();
    let fills = (0..many / 6)
        .map(|index| {
            square_fill(
                2.0 + index as f64 * 1e-3,
                5.0,
                Layer::Model,
                1_000_000 + index,
            )
        })
        .collect();
    let scene = Scene {
        meshes: vec![MeshInstance {
            mesh: Arc::clone(&mesh),
            faces: styles,
        }],
        batches: vec![Arc::new(Batch {
            lines,
            fills,
            ..Batch::default()
        })],
        ..Scene::default()
    };
    let view = looking_down(400.0, f64::from(SIZE), f64::from(SIZE));
    let near_end = Point3::new(-75.0, 0.0, 0.0);
    let far_end = Point3::new(75.0, 0.0, 0.0);
    let on_lines = Point3::new(-30.0, 20.0, 1.0);

    let at_near_end = render(
        &device,
        &queue,
        &view,
        &scene,
        view.project(near_end).unwrap(),
    );
    let at_far_end = render(
        &device,
        &queue,
        &view,
        &scene,
        view.project(far_end).unwrap(),
    );
    let on_the_lines = render(
        &device,
        &queue,
        &view,
        &scene,
        view.project(on_lines).unwrap(),
    );

    for (rendered, at) in [(&at_near_end, near_end), (&at_far_end, far_end)] {
        let [red, green, blue, _] = pixel(rendered, view.project(at).unwrap());
        assert!(
            green > 60 && green > red * 2 && green > blue * 2,
            "the strip at {at} was {red} {green} {blue}"
        );
        assert!(
            rendered.pick.hits[0].id.index() >= 100,
            "{:?}",
            rendered.pick.hits
        );
    }
    let [red, green, _, _] = pixel(&on_the_lines, view.project(on_lines).unwrap());
    assert!(red > 200 && green < 80, "the lines were {red} {green}");
    assert!(on_the_lines.pick.hits[0].id.index() < many);
}

#[test]
fn an_orthographic_view_draws_and_picks_faces_behind_its_eye_with_edges_over_them() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let styles: Vec<FaceStyle> = (0..6)
        .map(|index| FaceStyle {
            color: Color::from_rgb8(40, 200, 40),
            pick: PickId::from_index(10 + index),
        })
        .collect();
    let line = |start: Point3, end: Point3, index: usize| Line {
        start,
        end,
        color: LINE_COLOR,
        width: 3.0,
        layer: Layer::Model,
        pick: PickId::from_index(index),
        stroke: Stroke::Solid,
    };
    let scene = Scene {
        meshes: vec![MeshInstance {
            mesh: Arc::new(box_mesh(20.0)),
            faces: styles,
        }],
        batches: vec![Arc::new(Batch {
            lines: vec![
                line(
                    Point3::new(-50.0, 0.0, 20.0),
                    Point3::new(50.0, 0.0, 20.0),
                    0,
                ),
                line(
                    Point3::new(0.0, -50.0, -20.0),
                    Point3::new(0.0, 50.0, -20.0),
                    1,
                ),
            ],
            ..Batch::default()
        })],
        ..Scene::default()
    };
    let viewpoint =
        Viewpoint::looking_from(Vector3::Z, Point3::new(0.0, 0.0, -100.0), 112.0).unwrap();
    let view = View::new(viewpoint, f64::from(SIZE), f64::from(SIZE))
        .with_projection(Projection::Orthographic);
    let top = Point3::new(0.0, 10.0, 20.0);
    let on_top = view.project(top).unwrap();
    let edge_on_top = view.project(Point3::new(10.0, 0.0, 20.0)).unwrap();
    let edge_beside = view.project(Point3::new(26.0, 0.0, 20.0)).unwrap();
    let under_the_box = view.project(Point3::new(0.0, 10.0, -20.0)).unwrap();
    let beside_the_box = view.project(Point3::new(0.0, 26.0, -20.0)).unwrap();
    let greenish =
        |[red, green, blue, _]: [u8; 4]| green > 60 && green > 2 * red && green > 2 * blue;
    let reddish = |[red, green, _, _]: [u8; 4]| red > 200 && green < 80;

    let rendered = render(&device, &queue, &view, &scene, on_top);

    let nearest = rendered.pick.hits[0];
    assert!(view.view_depth(top) < 0.0);
    assert_eq!(nearest.id, PickId::from_index(14).unwrap());
    assert!(
        nearest.position.distance(top) < 0.5,
        "{:?}",
        nearest.position
    );
    assert!(greenish(pixel(&rendered, on_top)));
    assert!(reddish(pixel(&rendered, edge_on_top)));
    assert!(reddish(pixel(&rendered, edge_beside)));
    assert!(greenish(pixel(&rendered, under_the_box)));
    assert!(reddish(pixel(&rendered, beside_the_box)));
    assert!(on_top.distance(under_the_box) < 1e-9);
}

fn diagonal_line(layer: Layer) -> Line {
    Line {
        start: Point3::new(-50.0, -37.5, 0.0),
        end: Point3::new(50.0, 37.5, 0.0),
        color: LINE_COLOR,
        width: 3.0,
        layer,
        pick: PickId::from_index(0),
        stroke: Stroke::Solid,
    }
}

fn partly_covered_pixels(rendered: &Rendered) -> usize {
    (0..SIZE)
        .flat_map(|y| (0..SIZE).map(move |x| DVec2::new(f64::from(x), f64::from(y))))
        .filter(|at| (70..=200).contains(&pixel(rendered, *at)[0]))
        .count()
}

#[test]
fn every_offered_anti_aliasing_level_smooths_edges_and_keeps_front_geometry_and_picks_exact() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let Some(opened) = opened_device(&instance) else {
        return;
    };
    let (device, queue) = (&opened.device, &opened.queue);
    let offered = gpu::offered_msaa(
        &opened.adapter,
        device,
        FORMAT,
        crate::viewport::DEPTH_FORMAT,
    );
    let mut renderer = ViewportRenderer::new(device, FORMAT, 1);
    let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 150.0).unwrap();
    let view = View::new(viewpoint, f64::from(SIZE), f64::from(SIZE));
    let bare = Scene {
        batches: vec![Arc::new(Batch {
            lines: vec![diagonal_line(Layer::Model)],
            ..Batch::default()
        })],
        ..Scene::default()
    };
    let through_a_box = Scene {
        meshes: vec![MeshInstance {
            mesh: Arc::new(box_mesh(20.0)),
            faces: vec![
                FaceStyle {
                    color: Color::from_rgb8(90, 90, 90),
                    pick: None,
                };
                6
            ],
        }],
        batches: vec![Arc::new(Batch {
            lines: vec![diagonal_line(Layer::Front)],
            ..Batch::default()
        })],
        ..Scene::default()
    };
    let inside = view.project(Point3::new(4.0, 3.0, 0.0)).unwrap();

    assert!(offered.contains(&Msaa::Off));
    assert!(offered.contains(&Msaa::X4), "{offered:?}");
    for level in offered {
        renderer.set_sample_count(device, level.samples());
        let edges = render_with(
            &mut renderer,
            device,
            queue,
            &full_frame(&view, &bare, inside),
        );
        let front = render_with(
            &mut renderer,
            device,
            queue,
            &full_frame(&view, &through_a_box, inside),
        );

        assert_eq!(renderer.sample_count(), level.samples());
        let softened = partly_covered_pixels(&edges);
        if level == Msaa::Off {
            assert_eq!(softened, 0, "{level:?}");
        } else {
            assert!(softened > 40, "{level:?} softened only {softened} pixels");
        }
        let [red, green, blue, _] = pixel(&front, inside);
        assert!(
            red > 230 && green < 40 && blue < 40,
            "{level:?}: the front line inside the box was {red} {green} {blue}"
        );
        assert_eq!(front.pick.hits[0].id, PickId::from_index(0).unwrap());
        assert_eq!(front.pick.hits[0].offset_points, 0.0);
        assert!(
            front.pick.hits[0]
                .position
                .distance(Point3::new(4.0, 3.0, 0.0))
                < 0.5
        );
    }
}

#[test]
fn enhanced_shading_sets_faces_apart_keeps_their_tint_and_keeps_dimmed_bodies_darker() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let mut renderer = ViewportRenderer::new(&device, FORMAT, 4);
    let mesh = Arc::new(box_mesh(20.0));
    let painted = |color: Color| Scene {
        meshes: vec![MeshInstance {
            mesh: Arc::clone(&mesh),
            faces: (0..6)
                .map(|index| FaceStyle {
                    color,
                    pick: PickId::from_index(10 + index),
                })
                .collect(),
        }],
        ..Scene::default()
    };
    let green = painted(Color::from_rgb8(40, 200, 40));
    let body = painted(Color::from_rgb8(150, 162, 180));
    let dimmed = painted(Color::from_rgb8(92, 96, 104));
    let viewpoint =
        Viewpoint::looking_from(Vector3::new(0.8, -1.0, 0.9), Point3::ZERO, 150.0).unwrap();
    let view = View::new(viewpoint, f64::from(SIZE), f64::from(SIZE));
    let faces = [
        Point3::new(0.0, 0.0, 20.0),
        Point3::new(0.0, -20.0, 0.0),
        Point3::new(20.0, 0.0, 0.0),
    ]
    .map(|point| view.project(point).unwrap());
    let mut shade = |shading: Shading, scene: &Scene| {
        renderer.set_shading(shading);
        let rendered = render_with(
            &mut renderer,
            &device,
            &queue,
            &full_frame(&view, scene, faces[0]),
        );
        (faces.map(|at| pixel(&rendered, at)), rendered.pick)
    };
    let brightness =
        |[red, green, blue, _]: [u8; 4]| u32::from(red) + u32::from(green) + u32::from(blue);

    let (standard, _) = shade(Shading::Standard, &green);
    let (enhanced, pick) = shade(Shading::Enhanced, &green);
    let (lit_body, _) = shade(Shading::Enhanced, &body);
    let (lit_dimmed, _) = shade(Shading::Enhanced, &dimmed);

    for [red, green, blue, _] in enhanced {
        assert!(
            green > 60 && green > 2 * red && green > 2 * blue,
            "{red} {green} {blue}"
        );
    }
    assert!(
        standard
            .iter()
            .zip(&enhanced)
            .any(|(a, b)| a[1].abs_diff(b[1]) > 6),
        "{standard:?} {enhanced:?}"
    );
    let greens = enhanced.map(|[_, green, _, _]| green);
    assert!(greens[0].abs_diff(greens[1]) > 5, "{greens:?}");
    assert!(greens[1].abs_diff(greens[2]) > 5, "{greens:?}");
    assert!(greens[0].abs_diff(greens[2]) > 5, "{greens:?}");
    for (body, dimmed) in lit_body.into_iter().zip(lit_dimmed) {
        assert!(
            brightness(dimmed) + 60 < brightness(body),
            "{body:?} {dimmed:?}"
        );
    }
    assert_eq!(pick.hits[0].id, PickId::from_index(14).unwrap());
}

fn export_image(
    renderer: &mut ViewportRenderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    request: &ImageRequest<'_>,
    tile_side: u32,
) -> Image {
    let readback = renderer
        .encode_image(device, queue, request, tile_side)
        .unwrap();
    let pending = PendingImage::map(readback, 0);
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();

    assert!(pending.is_mapped());
    let ImagePoll::Ready(readback) = pending.finish() else {
        panic!("the image should be read back");
    };
    readback.into_image().unwrap()
}

fn image_pixel(image: &Image, at: DVec2) -> [u8; 4] {
    let offset = (at.y as usize * image.width as usize + at.x as usize) * 4;
    image.pixels[offset..offset + 4].try_into().unwrap()
}

#[test]
fn an_exported_image_is_drawn_in_tiles_at_its_own_size_with_the_chosen_background() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let size = SurfaceSize {
        width: 300,
        height: 180,
    };
    let scene = Scene {
        meshes: vec![MeshInstance {
            mesh: Arc::new(box_mesh(20.0)),
            faces: vec![
                FaceStyle {
                    color: Color::from_rgb8(40, 200, 40),
                    pick: None,
                };
                6
            ],
        }],
        ..Scene::default()
    };
    let view = looking_down(150.0, f64::from(size.width), f64::from(size.height));
    let on_top = view.project(Point3::new(0.0, 0.0, 20.0)).unwrap();
    let corner = DVec2::new(2.0, 2.0);
    let request = |background| ImageRequest {
        size,
        view: &view,
        scene: &scene,
        pixels_per_point: 1.0,
        background,
    };
    let mut renderer = ViewportRenderer::new(&device, FORMAT, 4);
    let mut bgra = ViewportRenderer::new(&device, wgpu::TextureFormat::Bgra8Unorm, 1);

    let whole = export_image(
        &mut renderer,
        &device,
        &queue,
        &request(Background::Viewport),
        512,
    );
    let tiled = export_image(
        &mut renderer,
        &device,
        &queue,
        &request(Background::Viewport),
        64,
    );
    let transparent = export_image(
        &mut renderer,
        &device,
        &queue,
        &request(Background::Transparent),
        64,
    );
    let swapped = export_image(
        &mut bgra,
        &device,
        &queue,
        &request(Background::Viewport),
        128,
    );
    let differing = whole
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(tiled.pixels.as_chunks::<4>().0)
        .filter(|(a, b)| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 2))
        .count();
    let top = image_pixel(&transparent, on_top);

    assert_eq!((whole.width, whole.height), (300, 180));
    assert_eq!(whole.pixels.len(), 300 * 180 * 4);
    let [red, green, blue, alpha] = image_pixel(&whole, on_top);
    assert!(
        green > 60 && green > red * 2 && green > blue * 2 && alpha == 255,
        "the top of the box was {red} {green} {blue} {alpha}"
    );
    assert!(is_background(image_pixel(&whole, corner)));
    assert_eq!(image_pixel(&whole, corner)[3], 255);
    assert!(differing < 300 * 180 / 100, "{differing} pixels differ");
    assert_eq!(image_pixel(&transparent, corner), [0, 0, 0, 0]);
    assert_eq!(top[3], 255);
    assert!(
        top.iter()
            .zip(image_pixel(&whole, on_top))
            .all(|(a, b)| a.abs_diff(b) <= 1)
    );
    let [red, green, blue, _] = image_pixel(&swapped, on_top);
    assert!(
        green > red * 2 && green > blue * 2,
        "a BGRA target gave {red} {green} {blue}"
    );
    assert!(is_background(image_pixel(&swapped, corner)));
}

#[test]
fn a_window_less_renderer_draws_an_image_without_a_surface() {
    let Some(mut renderer) = offscreen_renderer() else {
        return;
    };
    let size = SurfaceSize {
        width: 300,
        height: 180,
    };
    let scene = Scene {
        meshes: vec![MeshInstance {
            mesh: Arc::new(box_mesh(20.0)),
            faces: vec![
                FaceStyle {
                    color: Color::from_rgb8(40, 200, 40),
                    pick: None,
                };
                6
            ],
        }],
        ..Scene::default()
    };
    let view = looking_down(150.0, f64::from(size.width), f64::from(size.height));
    let on_top = view.project(Point3::new(0.0, 0.0, 20.0)).unwrap();
    let corner = DVec2::new(2.0, 2.0);

    let image = renderer
        .render(&ImageRequest {
            size,
            view: &view,
            scene: &scene,
            pixels_per_point: 1.0,
            background: Background::Viewport,
        })
        .unwrap();
    let again = renderer
        .render(&ImageRequest {
            size,
            view: &view,
            scene: &scene,
            pixels_per_point: 1.0,
            background: Background::Transparent,
        })
        .unwrap();

    assert_eq!((image.width, image.height), (300, 180));
    let [red, green, blue, alpha] = image_pixel(&image, on_top);
    assert!(
        green > 60 && green > red * 2 && green > blue * 2 && alpha == 255,
        "the top of the box was {red} {green} {blue} {alpha}"
    );
    assert!(is_background(image_pixel(&image, corner)));
    assert_eq!(image_pixel(&again, corner), [0, 0, 0, 0]);
}

fn offscreen_renderer() -> Option<crate::OffscreenRenderer> {
    let opened = crate::OffscreenRenderer::new(crate::GraphicsSettings::default());
    if opened.is_err() {
        assert!(
            std::env::var_os(REQUIRE_GPU).is_none(),
            "no graphics adapter is available, and {REQUIRE_GPU} says the offscreen tests must run"
        );
        eprintln!("no graphics adapter available, skipping the offscreen test");
    }
    opened.ok()
}

#[test]
fn lines_in_an_exported_image_widen_with_its_pixels_per_point() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let size = SurfaceSize {
        width: 400,
        height: 400,
    };
    let view = looking_down(100.0, f64::from(size.width), f64::from(size.height));
    let scene = line_and_marker();
    let across = view.project(Point3::new(-10.0, 0.0, 0.0)).unwrap();
    let mut renderer = ViewportRenderer::new(&device, FORMAT, 1);
    let mut width = |pixels_per_point: f32| {
        let image = export_image(
            &mut renderer,
            &device,
            &queue,
            &ImageRequest {
                size,
                view: &view,
                scene: &scene,
                pixels_per_point,
                background: Background::Transparent,
            },
            100,
        );
        (0..size.height)
            .filter(|row| {
                image_pixel(&image, DVec2::new(across.x.floor(), f64::from(*row)))[3] > 128
            })
            .count()
    };

    let at_one = width(1.0);
    let at_two = width(2.0);

    assert!((2..=4).contains(&at_one), "{at_one}");
    assert!((5..=7).contains(&at_two), "{at_two}");
}

fn large_scene() -> Scene {
    let lines = (0..330_000)
        .map(|index| {
            let at = Point3::new(f64::from(index % 600), f64::from(index / 600), 0.0);
            Line {
                start: at,
                end: at + Vector3::new(0.8, 0.3, 0.0),
                color: LINE_COLOR,
                width: 2.0,
                layer: Layer::Front,
                pick: PickId::from_index(index as usize / 30),
                stroke: Stroke::Solid,
            }
        })
        .collect();
    let markers = (0..49_000)
        .map(|index| Marker {
            position: Point3::new(f64::from(index % 300) * 2.0, f64::from(index / 300), 0.0),
            color: Color::from_rgb8(255, 255, 255),
            diameter: 7.0,
            layer: Layer::Front,
            pick: PickId::from_index(20_000 + index as usize),
        })
        .collect();
    let fills = (0..30)
        .map(|index| {
            let z = f64::from(index);
            Fill::convex(
                &[
                    Point3::new(0.0, 0.0, z),
                    Point3::new(100.0, 0.0, z),
                    Point3::new(100.0, 100.0, z),
                    Point3::new(0.0, 100.0, z),
                ],
                Color::from_rgba8(100, 100, 200, 30),
                Layer::Reference,
                PickId::from_index(90_000 + index as usize),
            )
        })
        .collect();
    Scene {
        batches: vec![Arc::new(Batch {
            lines,
            markers,
            fills,
        })],
        ..Scene::default()
    }
}

#[test]
#[ignore = "a timing benchmark: cargo test --release -p caditor-render frame_costs -- --ignored --nocapture"]
fn frame_costs_of_drawing_a_large_scene() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    const FRAMES: u32 = 100;
    let size = SurfaceSize {
        width: 1600,
        height: 1000,
    };
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("timing target"),
        size: wgpu::Extent3d {
            width: size.width,
            height: size.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let scene = large_scene();
    let replaced = Scene {
        batches: scene
            .batches
            .iter()
            .map(|batch| Arc::new(Batch::clone(batch)))
            .collect(),
        ..scene.clone()
    };
    let viewpoint =
        Viewpoint::looking_from(Vector3::Z, Point3::new(300.0, 300.0, 0.0), 800.0).unwrap();
    let time = |name: &str, moving: bool, replacing: bool| {
        let mut renderer = ViewportRenderer::new(&device, FORMAT, 4);
        let mut elapsed = std::time::Duration::ZERO;
        for frame in 0..FRAMES + 3 {
            let turned = if moving {
                Viewpoint::looking_from(
                    Vector3::new(f64::from(frame) * 0.001, 0.0, 1.0),
                    viewpoint.target,
                    viewpoint.distance,
                )
                .unwrap()
            } else {
                viewpoint
            };
            let view = View::new(turned, f64::from(size.width), f64::from(size.height));
            let started = std::time::Instant::now();
            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            renderer.draw(
                &device,
                &queue,
                &mut encoder,
                &SurfaceTarget {
                    view: &target_view,
                    width: size.width,
                    height: size.height,
                },
                Some(&ViewportFrame {
                    rect: ViewportRect {
                        x: 0.0,
                        y: 0.0,
                        width: size.width as f32,
                        height: size.height as f32,
                    },
                    view: &view,
                    scene: if replacing && frame % 2 == 1 {
                        &replaced
                    } else {
                        &scene
                    },
                    pick_at: None,
                    pixels_per_point: 1.0,
                }),
            );
            queue.submit([encoder.finish()]);
            if frame >= 3 {
                elapsed += started.elapsed();
            }
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        }
        eprintln!("{name}: {:?} per frame on the UI thread", elapsed / FRAMES);
    };

    time("large scene, idle", false, false);
    time("large scene, camera moving", true, false);
    time("large scene, replaced every frame", false, true);
}

fn differing_pixels(a: &Rendered, b: &Rendered) -> usize {
    a.pixels
        .chunks(4)
        .zip(b.pixels.chunks(4))
        .filter(|(a, b)| a.iter().zip(b.iter()).any(|(a, b)| a.abs_diff(*b) > 2))
        .count()
}

fn marker_at(scene: &Scene) -> Point3 {
    scene.markers().next().unwrap().position
}

#[test]
fn an_unchanged_scene_is_uploaded_once_and_the_camera_moves_without_uploading_it() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let scene = scene();
    let first_view = looking_down(100.0, f64::from(SIZE), f64::from(SIZE));
    let turned_view = View::new(
        Viewpoint::looking_from(
            Vector3::new(0.3, -0.2, 1.0),
            Point3::new(5.0, 2.0, 0.0),
            90.0,
        )
        .unwrap(),
        f64::from(SIZE),
        f64::from(SIZE),
    );
    let first_at = first_view.project(marker_at(&scene)).unwrap();
    let turned_at = turned_view.project(marker_at(&scene)).unwrap();
    let mut renderer = ViewportRenderer::new(&device, FORMAT, 4);

    let first = render_with(
        &mut renderer,
        &device,
        &queue,
        &full_frame(&first_view, &scene, first_at),
    );
    let idle = render_with(
        &mut renderer,
        &device,
        &queue,
        &full_frame(&first_view, &scene, first_at),
    );
    let after_idle = renderer.work();
    let turned = render_with(
        &mut renderer,
        &device,
        &queue,
        &full_frame(&turned_view, &scene, turned_at),
    );
    let after_turning = renderer.work();
    let fresh = render(&device, &queue, &turned_view, &scene, turned_at);

    assert_eq!(
        after_idle,
        Work {
            uploads: 1,
            sorts: 1
        }
    );
    assert_eq!(
        after_turning,
        Work {
            uploads: 1,
            sorts: 2
        }
    );
    assert_eq!(idle.pixels, first.pixels);
    assert_eq!(idle.pick, first.pick);
    assert!(differing_pixels(&turned, &fresh) <= 2);
    assert_eq!(
        turned.pick.hits.first().map(|hit| hit.id),
        fresh.pick.hits.first().map(|hit| hit.id)
    );
    assert_eq!(turned.pick.hits[0].id, PickId::from_index(2).unwrap());
}

#[test]
fn only_a_changed_batch_is_uploaded_again_and_a_dropped_one_stops_drawing() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let view = looking_down(100.0, f64::from(SIZE), f64::from(SIZE));
    let base = Arc::new(Batch {
        lines: line_and_marker().lines().cloned().collect(),
        ..Batch::default()
    });
    let marker = |x: f64| {
        Arc::new(Batch {
            markers: vec![Marker {
                position: Point3::new(x, 20.0, 0.0),
                color: Color::from_rgb8(255, 255, 255),
                diameter: 9.0,
                layer: Layer::Model,
                pick: PickId::from_index(1),
            }],
            ..Batch::default()
        })
    };
    let with = |overlay: Option<Arc<Batch>>| Scene {
        batches: std::iter::once(Arc::clone(&base)).chain(overlay).collect(),
        ..Scene::default()
    };
    let left = view.project(Point3::new(-10.0, 20.0, 0.0)).unwrap();
    let right = view.project(Point3::new(10.0, 20.0, 0.0)).unwrap();
    let mut renderer = ViewportRenderer::new(&device, FORMAT, 4);
    let mut draw = |scene: &Scene| {
        render_with(
            &mut renderer,
            &device,
            &queue,
            &full_frame(&view, scene, left),
        )
    };

    let first = draw(&with(Some(marker(-10.0))));
    let moved = draw(&with(Some(marker(10.0))));
    let dropped = draw(&with(None));
    let uploaded = renderer.uploaded();

    assert!(pixel(&first, left)[0] > 200);
    assert!(pixel(&moved, left)[0] < 60 && pixel(&moved, right)[0] > 200);
    assert!(pixel(&dropped, right)[0] < 60);
    assert_eq!(renderer.work().uploads, 3);
    assert_eq!(uploaded.len(), 1);
    assert!(Arc::ptr_eq(uploaded[0].as_ref().unwrap(), &base));
}

#[test]
fn moving_far_from_where_the_scene_was_uploaded_uploads_it_again_as_exactly_as_ever() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let far = Point3::new(4.0e6, -3.0e6, 2.0e5);
    let lines: Vec<Line> = line_and_marker()
        .lines()
        .map(|line| Line {
            start: line.start + (far - Point3::ZERO),
            end: line.end + (far - Point3::ZERO),
            ..line.clone()
        })
        .collect();
    let scene = Scene::from(Batch {
        lines,
        ..Batch::default()
    });
    let view_at = |target: Point3| {
        View::new(
            Viewpoint::looking_from(Vector3::Z, target, 100.0).unwrap(),
            f64::from(SIZE),
            f64::from(SIZE),
        )
    };
    let start = view_at(Point3::ZERO);
    let nearby = view_at(Point3::new(150.0, 0.0, 0.0));
    let arrived = view_at(far);
    let on_line = arrived
        .project(far + Vector3::new(-10.0, 0.0, 0.0))
        .unwrap();
    let mut renderer = ViewportRenderer::new(&device, FORMAT, 4);
    let draw = |renderer: &mut ViewportRenderer, view: &View| {
        render_with(
            renderer,
            &device,
            &queue,
            &full_frame(view, &scene, on_line),
        );
        renderer.anchor()
    };

    let first_anchor = draw(&mut renderer, &start);
    let nearby_anchor = draw(&mut renderer, &nearby);
    let cached = render_with(
        &mut renderer,
        &device,
        &queue,
        &full_frame(&arrived, &scene, on_line),
    );
    let fresh = render(&device, &queue, &arrived, &scene, on_line);

    assert_eq!(nearby_anchor, first_anchor);
    assert_eq!(renderer.anchor(), Some(arrived.eye()));
    assert_eq!(renderer.work().uploads, 2);
    assert_eq!(cached.pixels, fresh.pixels);
    assert!(pixel(&cached, on_line)[0] > 200);
    assert_eq!(cached.pick, fresh.pick);
}

#[test]
fn translucent_fills_are_ordered_again_when_the_view_turns_over() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let red = Fill {
        color: Color::from_rgba8(255, 0, 0, 128),
        ..square_fill(5.0, 20.0, Layer::Model, 0)
    };
    let green = Fill {
        color: Color::from_rgba8(0, 255, 0, 128),
        ..square_fill(-5.0, 20.0, Layer::Model, 1)
    };
    let scene = Scene::from(Batch {
        fills: vec![red, green],
        ..Batch::default()
    });
    let from = |direction: Vector3| {
        View::new(
            Viewpoint::looking_from(direction, Point3::ZERO, 100.0).unwrap(),
            f64::from(SIZE),
            f64::from(SIZE),
        )
    };
    let above = from(Vector3::new(0.0, -0.1, 1.0));
    let below = from(Vector3::new(0.0, -0.1, -1.0));
    let middle = DVec2::splat(f64::from(SIZE) / 2.0);
    let mut renderer = ViewportRenderer::new(&device, FORMAT, 4);

    let from_above = render_with(
        &mut renderer,
        &device,
        &queue,
        &full_frame(&above, &scene, middle),
    );
    let above_draws = renderer.fill_draws().to_vec();
    let from_below = render_with(
        &mut renderer,
        &device,
        &queue,
        &full_frame(&below, &scene, middle),
    );
    let fresh_below = render(&device, &queue, &below, &scene, middle);

    assert!(pixel(&from_above, middle)[0] > pixel(&from_above, middle)[1]);
    assert!(pixel(&from_below, middle)[1] > pixel(&from_below, middle)[0]);
    assert_eq!(from_below.pixels, fresh_below.pixels);
    assert_eq!(
        renderer.work(),
        Work {
            uploads: 1,
            sorts: 2
        }
    );
    assert_eq!(above_draws, vec![(0, 6..12), (0, 0..6)]);
    assert_eq!(renderer.fill_draws(), &[(0, 0..12)][..]);
}

#[test]
fn an_allocation_the_device_refuses_comes_back_as_an_error_instead_of_a_lost_encoder() {
    let Some((device, queue)) = gpu() else {
        return;
    };

    let (buffer, refused) = gpu::scoped(&device, || {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("too large"),
            size: device.limits().max_buffer_size.saturating_add(1),
            usage: wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    });
    let (_, fitting) = gpu::scoped(&device, || {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("small"),
            size: 64,
            usage: wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    });
    drop(buffer);
    queue.submit([]);

    assert!(refused.is_some());
    assert!(fitting.is_none());
}

#[test]
fn viewport_targets_the_device_refuses_are_reported_once_and_the_frame_is_still_cleared() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let mut renderer = ViewportRenderer::new(&device, FORMAT, 4);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen target"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut draw = |side: u32| {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let faults = renderer.draw(
            &device,
            &queue,
            &mut encoder,
            &SurfaceTarget {
                view: &target_view,
                width: side,
                height: side,
            },
            None,
        );
        queue.submit([encoder.finish()]);
        faults
    };
    let too_large = device.limits().max_texture_dimension_2d + 1;

    let refused = draw(too_large);
    let repeated = draw(too_large);
    let fitting = draw(SIZE);

    assert!(refused.targets);
    assert!(!repeated.any());
    assert!(!fitting.any());
}
