use std::sync::Arc;

use ahash::AHashMap;
use glam::DVec2;
use parking_lot::Mutex;

use crate::{
    camera::View,
    scene::{PickHit, PickId, PickResult},
};

pub const PICK_RADIUS_POINTS: f64 = 7.5;
pub const ID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
pub const DEPTH_VALUE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
const TEXEL_BYTES: u32 = 4;
const MAX_PICK_RADIUS: u32 = 63;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PickWindow {
    radius: u32,
    pixels_per_point: f64,
}

impl PickWindow {
    pub fn for_scale(pixels_per_point: f32) -> Self {
        let pixels_per_point = f64::from(pixels_per_point);
        let pixels_per_point = if pixels_per_point.is_finite() && pixels_per_point > 0.0 {
            pixels_per_point
        } else {
            1.0
        };
        let radius = (PICK_RADIUS_POINTS * pixels_per_point)
            .floor()
            .clamp(0.0, f64::from(MAX_PICK_RADIUS)) as u32;
        Self {
            radius,
            pixels_per_point,
        }
    }

    pub fn side(self) -> u32 {
        self.radius * 2 + 1
    }

    fn row_pitch(self) -> u32 {
        (self.side() * TEXEL_BYTES).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
    }

    fn plane_bytes(self) -> u64 {
        u64::from(self.row_pitch()) * u64::from(self.side())
    }

    fn extent(self) -> wgpu::Extent3d {
        wgpu::Extent3d {
            width: self.side(),
            height: self.side(),
            depth_or_array_layers: 1,
        }
    }

    fn same_size(self, other: Self) -> bool {
        self.radius == other.radius
    }
}

type MapOutcome = Arc<Mutex<Option<Result<(), wgpu::BufferAsyncError>>>>;

enum Stage {
    Encoded,
    Abandoned,
    Mapping(MapOutcome),
}

struct InFlight {
    view: View,
    cursor: DVec2,
    stage: Stage,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PickPoll {
    Pending,
    Ready(PickResult),
    Failed,
}

pub struct PickTargets {
    pub ids: wgpu::TextureView,
    pub depths: wgpu::TextureView,
    pub depth_buffer: wgpu::TextureView,
}

struct WindowResources {
    window: PickWindow,
    id_texture: wgpu::Texture,
    depth_texture: wgpu::Texture,
    targets: PickTargets,
    readback: wgpu::Buffer,
}

impl WindowResources {
    fn new(
        device: &wgpu::Device,
        depth_buffer_format: wgpu::TextureFormat,
        window: PickWindow,
    ) -> Self {
        let id_texture = pick_texture(device, "pick ids", ID_FORMAT, window, true);
        let depth_texture = pick_texture(device, "pick depths", DEPTH_VALUE_FORMAT, window, true);
        let depth_buffer = pick_texture(
            device,
            "pick depth buffer",
            depth_buffer_format,
            window,
            false,
        );
        let targets = PickTargets {
            ids: id_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            depths: depth_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            depth_buffer: depth_buffer.create_view(&wgpu::TextureViewDescriptor::default()),
        };
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pick readback"),
            size: window.plane_bytes() * 2,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            window,
            id_texture,
            depth_texture,
            targets,
            readback,
        }
    }
}

pub struct Picking {
    depth_buffer_format: wgpu::TextureFormat,
    resources: WindowResources,
    readback_failed: bool,
    in_flight: Option<InFlight>,
}

impl Picking {
    pub fn new(device: &wgpu::Device, depth_buffer_format: wgpu::TextureFormat) -> Self {
        let window = PickWindow::for_scale(1.0);
        Self {
            depth_buffer_format,
            resources: WindowResources::new(device, depth_buffer_format, window),
            readback_failed: false,
            in_flight: None,
        }
    }

    pub fn is_pending(&self) -> bool {
        self.in_flight.is_some()
    }

    pub fn prepare(&mut self, device: &wgpu::Device, window: PickWindow) -> Option<PickWindow> {
        if self.is_pending() {
            return None;
        }
        if self.resources.window.same_size(window) && !self.readback_failed {
            self.resources.window = window;
        } else {
            self.resources = WindowResources::new(device, self.depth_buffer_format, window);
            self.readback_failed = false;
        }
        Some(window)
    }

    pub fn prepared(&self) -> Option<&PickTargets> {
        (!self.is_pending()).then_some(&self.resources.targets)
    }

    pub fn encode_readback(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: View,
        cursor: DVec2,
    ) {
        let resources = &self.resources;
        let window = resources.window;
        for (texture, offset) in [
            (&resources.id_texture, 0),
            (&resources.depth_texture, window.plane_bytes()),
        ] {
            encoder.copy_texture_to_buffer(
                texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &resources.readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset,
                        bytes_per_row: Some(window.row_pitch()),
                        rows_per_image: Some(window.side()),
                    },
                },
                window.extent(),
            );
        }
        self.in_flight = Some(InFlight {
            view,
            cursor,
            stage: Stage::Encoded,
        });
    }

    #[cfg(test)]
    pub fn destroy_readback(&self) {
        self.resources.readback.destroy();
    }

    pub fn abandon_unsubmitted(&mut self) {
        if let Some(in_flight) = self.in_flight.as_mut()
            && matches!(in_flight.stage, Stage::Encoded)
        {
            in_flight.stage = Stage::Abandoned;
        }
    }

    pub fn after_submit(&mut self) {
        let Some(in_flight) = self.in_flight.as_mut() else {
            return;
        };
        if !matches!(in_flight.stage, Stage::Encoded) {
            return;
        }
        let outcome = MapOutcome::default();
        let sink = Arc::clone(&outcome);
        self.resources
            .readback
            .map_async(wgpu::MapMode::Read, .., move |result| {
                *sink.lock() = Some(result);
            });
        in_flight.stage = Stage::Mapping(outcome);
    }

    pub fn poll(&mut self, device: &wgpu::Device) -> PickPoll {
        let outcome = match &self.in_flight {
            Some(InFlight {
                stage: Stage::Mapping(outcome),
                ..
            }) => outcome,
            Some(InFlight {
                stage: Stage::Abandoned,
                ..
            }) => {
                self.in_flight = None;
                return PickPoll::Failed;
            }
            Some(InFlight {
                stage: Stage::Encoded,
                ..
            })
            | None => return PickPoll::Pending,
        };
        if let Err(error) = device.poll(wgpu::PollType::Poll) {
            log::warn!("could not poll the graphics device for picking: {error}");
        }
        let Some(outcome) = outcome.lock().take() else {
            return PickPoll::Pending;
        };
        let Some(in_flight) = self.in_flight.take() else {
            return PickPoll::Pending;
        };
        self.read(outcome, &in_flight)
    }

    fn read(
        &mut self,
        outcome: Result<(), wgpu::BufferAsyncError>,
        in_flight: &InFlight,
    ) -> PickPoll {
        if let Err(error) = outcome {
            log::warn!("reading back the pick buffer failed: {error}");
            self.readback_failed = true;
            return PickPoll::Failed;
        }
        let readback = &self.resources.readback;
        let hits = match readback.get_mapped_range(..) {
            Ok(bytes) => decode_hits(
                &bytes,
                self.resources.window,
                &in_flight.view,
                in_flight.cursor,
            ),
            Err(error) => {
                log::warn!("could not read the pick buffer: {error}");
                readback.unmap();
                self.readback_failed = true;
                return PickPoll::Failed;
            }
        };
        readback.unmap();
        PickPoll::Ready(PickResult {
            cursor: in_flight.cursor,
            hits,
        })
    }
}

fn pick_texture(
    device: &wgpu::Device,
    label: &'static str,
    format: wgpu::TextureFormat,
    window: PickWindow,
    copied: bool,
) -> wgpu::Texture {
    let mut usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
    if copied {
        usage |= wgpu::TextureUsages::COPY_SRC;
    }
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: window.extent(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

pub fn window_center(cursor: DVec2) -> DVec2 {
    cursor.floor() + 0.5
}

pub fn pick_transform(cursor: DVec2, viewport: DVec2, window: PickWindow) -> [f32; 4] {
    let center = window_center(cursor);
    let ndc = DVec2::new(
        center.x / viewport.x * 2.0 - 1.0,
        1.0 - center.y / viewport.y * 2.0,
    );
    let scale = viewport / f64::from(window.side());
    [
        scale.x as f32,
        scale.y as f32,
        (-ndc.x * scale.x) as f32,
        (-ndc.y * scale.y) as f32,
    ]
}

fn texel(bytes: &[u8], offset: u64) -> Option<[u8; 4]> {
    let start = usize::try_from(offset).ok()?;
    bytes
        .get(start..start + TEXEL_BYTES as usize)?
        .try_into()
        .ok()
}

fn decode_hits(bytes: &[u8], window: PickWindow, view: &View, cursor: DVec2) -> Vec<PickHit> {
    let center = window_center(cursor);
    let radius = i64::from(window.radius);
    let mut nearest: AHashMap<PickId, PickHit> = AHashMap::new();
    for row in 0..window.side() {
        for column in 0..window.side() {
            let offset = u64::from(row * window.row_pitch() + column * TEXEL_BYTES);
            let Some(id) =
                texel(bytes, offset).and_then(|raw| PickId::from_raw(u32::from_le_bytes(raw)))
            else {
                continue;
            };
            let Some(depth) = texel(bytes, window.plane_bytes() + offset).map(f32::from_le_bytes)
            else {
                continue;
            };
            let delta = DVec2::new(
                (i64::from(column) - radius) as f64,
                (i64::from(row) - radius) as f64,
            );
            let Some(ray) = view.ray_through(center + delta) else {
                continue;
            };
            let facing = ray.direction().dot(view.forward());
            if facing <= 0.0 {
                continue;
            }
            let hit = PickHit {
                id,
                offset_points: (delta.length() / window.pixels_per_point) as f32,
                position: ray.at(f64::from(depth) / facing),
            };
            nearest
                .entry(id)
                .and_modify(|best| {
                    if hit.offset_points < best.offset_points {
                        *best = hit;
                    }
                })
                .or_insert(hit);
        }
    }
    let mut hits: Vec<PickHit> = nearest.into_values().collect();
    hits.sort_by(|a, b| {
        a.offset_points
            .total_cmp(&b.offset_points)
            .then(a.id.cmp(&b.id))
    });
    hits
}

#[cfg(test)]
mod tests {
    use caditor_geometry::{Point3, Vector3};

    use super::*;
    use crate::camera::Viewpoint;

    fn view() -> View {
        let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 100.0).unwrap();
        View::new(viewpoint, 400.0, 300.0)
    }

    fn write(bytes: &mut [u8], offset: u64, value: [u8; 4]) {
        let start = offset as usize;
        bytes[start..start + 4].copy_from_slice(&value);
    }

    #[test]
    fn transform_maps_the_cursor_pixel_to_the_window_center() {
        let viewport = DVec2::new(400.0, 300.0);
        let cursor = DVec2::new(100.7, 50.2);
        let window = PickWindow::for_scale(1.0);

        let [sx, sy, ox, oy] = pick_transform(cursor, viewport, window);
        let center = window_center(cursor);
        let ndc_x = center.x / viewport.x * 2.0 - 1.0;
        let ndc_y = 1.0 - center.y / viewport.y * 2.0;

        assert!((ndc_x as f32 * sx + ox).abs() < 1e-5);
        assert!((ndc_y as f32 * sy + oy).abs() < 1e-5);
        assert!((sx - 400.0 / window.side() as f32).abs() < 1e-5);
    }

    #[test]
    fn the_window_reaches_the_same_distance_in_points_at_every_scale() {
        let sides = [0.75, 1.0, 1.25, 2.0, 4.0].map(|scale| PickWindow::for_scale(scale).side());

        assert_eq!(sides, [11, 15, 19, 31, 61]);
        assert_eq!(PickWindow::for_scale(f32::NAN).side(), 15);
        assert_eq!(PickWindow::for_scale(0.0).side(), 15);
        assert_eq!(
            PickWindow::for_scale(1000.0).side(),
            MAX_PICK_RADIUS * 2 + 1
        );
        assert_eq!(PickWindow::for_scale(2.0).row_pitch(), 256);
        assert_eq!(PickWindow::for_scale(20.0).row_pitch(), 512);
    }

    #[test]
    fn decodes_the_nearest_pixel_of_each_id_with_its_world_position() {
        let view = view();
        let cursor = DVec2::new(200.0, 150.0);
        let window = PickWindow::for_scale(1.0);
        let planes = window.plane_bytes();
        let texel_at =
            |row: u32, column: u32| u64::from(row * window.row_pitch() + column * TEXEL_BYTES);
        let mut bytes = vec![0u8; (planes * 2) as usize];

        write(&mut bytes, texel_at(7, 7), 3u32.to_le_bytes());
        write(&mut bytes, planes + texel_at(7, 7), 100.0f32.to_le_bytes());
        write(&mut bytes, texel_at(7, 10), 5u32.to_le_bytes());
        write(&mut bytes, planes + texel_at(7, 10), 100.0f32.to_le_bytes());
        write(&mut bytes, texel_at(0, 0), 5u32.to_le_bytes());

        let hits = decode_hits(&bytes, window, &view, cursor);
        let first = hits[0];
        let expected = view.ray_through(window_center(cursor)).unwrap();
        let expected = expected.at(100.0 / expected.direction().dot(view.forward()));

        assert_eq!(hits.len(), 2);
        assert_eq!(first.id, PickId::from_raw(3).unwrap());
        assert_eq!(first.offset_points, 0.0);
        assert!(first.position.distance(expected) < 1e-4);
        assert!(first.position.z.abs() < 1e-4);
        assert_eq!(hits[1].id, PickId::from_raw(5).unwrap());
        assert_eq!(hits[1].offset_points, 3.0);
    }

    #[test]
    fn offsets_are_in_points_at_a_doubled_scale() {
        let view = view();
        let cursor = DVec2::new(200.0, 150.0);
        let window = PickWindow::for_scale(2.0);
        let planes = window.plane_bytes();
        let texel_at =
            |row: u32, column: u32| u64::from(row * window.row_pitch() + column * TEXEL_BYTES);
        let mut bytes = vec![0u8; (planes * 2) as usize];

        write(&mut bytes, texel_at(15, 29), 4u32.to_le_bytes());
        write(
            &mut bytes,
            planes + texel_at(15, 29),
            100.0f32.to_le_bytes(),
        );

        let hits = decode_hits(&bytes, window, &view, cursor);

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].offset_points, 7.0);
    }
}
