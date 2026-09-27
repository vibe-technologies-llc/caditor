use std::sync::Arc;

use ahash::AHashMap;
use glam::DVec2;
use parking_lot::Mutex;

use crate::{
    camera::View,
    scene::{PickHit, PickId, PickResult},
};

pub const PICK_WINDOW: u32 = 15;
pub const ID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
pub const DEPTH_VALUE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
const TEXEL_BYTES: u32 = 4;
const ROW_PITCH: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
const PLANE_BYTES: u64 = ROW_PITCH as u64 * PICK_WINDOW as u64;
const CENTER: i64 = PICK_WINDOW as i64 / 2;

type MapOutcome = Arc<Mutex<Option<Result<(), wgpu::BufferAsyncError>>>>;

enum Stage {
    Encoded,
    Mapping(MapOutcome),
}

struct InFlight {
    view: View,
    cursor: DVec2,
    stage: Stage,
}

pub struct PickTargets {
    pub ids: wgpu::TextureView,
    pub depths: wgpu::TextureView,
    pub depth_buffer: wgpu::TextureView,
}

pub struct Picking {
    id_texture: wgpu::Texture,
    depth_texture: wgpu::Texture,
    targets: PickTargets,
    readback: wgpu::Buffer,
    in_flight: Option<InFlight>,
}

impl Picking {
    pub fn new(device: &wgpu::Device, depth_buffer_format: wgpu::TextureFormat) -> Self {
        let id_texture = pick_texture(device, "pick ids", ID_FORMAT, true);
        let depth_texture = pick_texture(device, "pick depths", DEPTH_VALUE_FORMAT, true);
        let depth_buffer = pick_texture(device, "pick depth buffer", depth_buffer_format, false);
        let targets = PickTargets {
            ids: id_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            depths: depth_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            depth_buffer: depth_buffer.create_view(&wgpu::TextureViewDescriptor::default()),
        };
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pick readback"),
            size: PLANE_BYTES * 2,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            id_texture,
            depth_texture,
            targets,
            readback,
            in_flight: None,
        }
    }

    pub fn is_idle(&self) -> bool {
        self.in_flight.is_none()
    }

    pub fn is_pending(&self) -> bool {
        self.in_flight.is_some()
    }

    pub fn targets(&self) -> &PickTargets {
        &self.targets
    }

    pub fn encode_readback(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: View,
        cursor: DVec2,
    ) {
        for (texture, offset) in [(&self.id_texture, 0), (&self.depth_texture, PLANE_BYTES)] {
            encoder.copy_texture_to_buffer(
                texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &self.readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset,
                        bytes_per_row: Some(ROW_PITCH),
                        rows_per_image: Some(PICK_WINDOW),
                    },
                },
                window_extent(),
            );
        }
        self.in_flight = Some(InFlight {
            view,
            cursor,
            stage: Stage::Encoded,
        });
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
        self.readback
            .map_async(wgpu::MapMode::Read, .., move |result| {
                *sink.lock() = Some(result);
            });
        in_flight.stage = Stage::Mapping(outcome);
    }

    pub fn poll(&mut self, device: &wgpu::Device) -> Option<PickResult> {
        let Stage::Mapping(outcome) = &self.in_flight.as_ref()?.stage else {
            return None;
        };
        if let Err(error) = device.poll(wgpu::PollType::Poll) {
            log::warn!("could not poll the graphics device for picking: {error}");
        }
        let outcome = outcome.lock().take()?;
        let in_flight = self.in_flight.take()?;
        if let Err(error) = outcome {
            log::warn!("reading back the pick buffer failed: {error}");
            return None;
        }

        let hits = match self.readback.get_mapped_range(..) {
            Ok(bytes) => decode_hits(&bytes, &in_flight.view, in_flight.cursor),
            Err(error) => {
                log::warn!("could not read the pick buffer: {error}");
                Vec::new()
            }
        };
        self.readback.unmap();
        Some(PickResult {
            cursor: in_flight.cursor,
            hits,
        })
    }
}

fn pick_texture(
    device: &wgpu::Device,
    label: &'static str,
    format: wgpu::TextureFormat,
    copied: bool,
) -> wgpu::Texture {
    let mut usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
    if copied {
        usage |= wgpu::TextureUsages::COPY_SRC;
    }
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: window_extent(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn window_extent() -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: PICK_WINDOW,
        height: PICK_WINDOW,
        depth_or_array_layers: 1,
    }
}

pub fn window_center(cursor: DVec2) -> DVec2 {
    cursor.floor() + 0.5
}

pub fn pick_transform(cursor: DVec2, viewport: DVec2) -> [f32; 4] {
    let center = window_center(cursor);
    let ndc = DVec2::new(
        center.x / viewport.x * 2.0 - 1.0,
        1.0 - center.y / viewport.y * 2.0,
    );
    let scale = viewport / f64::from(PICK_WINDOW);
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

fn decode_hits(bytes: &[u8], view: &View, cursor: DVec2) -> Vec<PickHit> {
    let center = window_center(cursor);
    let mut nearest: AHashMap<PickId, PickHit> = AHashMap::new();
    for row in 0..PICK_WINDOW {
        for column in 0..PICK_WINDOW {
            let offset = u64::from(row * ROW_PITCH + column * TEXEL_BYTES);
            let Some(id) =
                texel(bytes, offset).and_then(|raw| PickId::from_raw(u32::from_le_bytes(raw)))
            else {
                continue;
            };
            let Some(depth) = texel(bytes, PLANE_BYTES + offset).map(f32::from_le_bytes) else {
                continue;
            };
            let delta = DVec2::new(
                (i64::from(column) - CENTER) as f64,
                (i64::from(row) - CENTER) as f64,
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
                offset_px: delta.length() as f32,
                position: ray.at(f64::from(depth) / facing),
            };
            nearest
                .entry(id)
                .and_modify(|best| {
                    if hit.offset_px < best.offset_px {
                        *best = hit;
                    }
                })
                .or_insert(hit);
        }
    }
    let mut hits: Vec<PickHit> = nearest.into_values().collect();
    hits.sort_by(|a, b| a.offset_px.total_cmp(&b.offset_px).then(a.id.cmp(&b.id)));
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
        let [sx, sy, ox, oy] = pick_transform(cursor, viewport);
        let center = window_center(cursor);
        let ndc_x = center.x / viewport.x * 2.0 - 1.0;
        let ndc_y = 1.0 - center.y / viewport.y * 2.0;

        assert!((ndc_x as f32 * sx + ox).abs() < 1e-5);
        assert!((ndc_y as f32 * sy + oy).abs() < 1e-5);
        assert!((sx - 400.0 / PICK_WINDOW as f32).abs() < 1e-5);
    }

    #[test]
    fn decodes_the_nearest_pixel_of_each_id_with_its_world_position() {
        let view = view();
        let cursor = DVec2::new(200.0, 150.0);
        let mut bytes = vec![0u8; (PLANE_BYTES * 2) as usize];
        let texel_at = |row: u32, column: u32| u64::from(row * ROW_PITCH + column * TEXEL_BYTES);

        write(&mut bytes, texel_at(7, 7), 3u32.to_le_bytes());
        write(
            &mut bytes,
            PLANE_BYTES + texel_at(7, 7),
            100.0f32.to_le_bytes(),
        );
        write(&mut bytes, texel_at(7, 10), 5u32.to_le_bytes());
        write(
            &mut bytes,
            PLANE_BYTES + texel_at(7, 10),
            100.0f32.to_le_bytes(),
        );
        write(&mut bytes, texel_at(0, 0), 5u32.to_le_bytes());

        let hits = decode_hits(&bytes, &view, cursor);

        assert_eq!(hits.len(), 2);
        let first = hits[0];
        assert_eq!(first.id, PickId::from_raw(3).unwrap());
        assert_eq!(first.offset_px, 0.0);
        let expected = view.ray_through(window_center(cursor)).unwrap();
        let expected = expected.at(100.0 / expected.direction().dot(view.forward()));
        assert!(first.position.distance(expected) < 1e-4);
        assert!(first.position.z.abs() < 1e-4);
        assert_eq!(hits[1].id, PickId::from_raw(5).unwrap());
        assert_eq!(hits[1].offset_px, 3.0);
    }
}
