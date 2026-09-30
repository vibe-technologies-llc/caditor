use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use crate::{SurfaceSize, camera::View, scene::Scene};

pub const MAX_IMAGE_SIDE: u32 = 8192;
pub const TILE_SIDE: u32 = 2048;
pub const IMAGE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const TEXEL_BYTES: u32 = 4;
const OPAQUE: u8 = u8::MAX;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Background {
    #[default]
    Viewport,
    Transparent,
}

pub struct ImageRequest<'a> {
    pub size: SurfaceSize,
    pub view: &'a View,
    pub scene: &'a Scene,
    pub pixels_per_point: f32,
    pub background: Background,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ImageError {
    #[error("another image is still being drawn")]
    Busy,
    #[error("an image of {width} × {height} pixels is outside 1 to {largest} pixels a side")]
    OutOfRange {
        width: u32,
        height: u32,
        largest: u32,
    },
    #[error("the graphics device was lost while drawing the image")]
    DeviceLost,
    #[error("the graphics card ran out of memory for the image")]
    OutOfMemory,
    #[error("the graphics device refused to draw the image")]
    Refused,
    #[error("the image could not be read back from the graphics card")]
    Readback,
}

pub enum ImagePoll {
    Idle,
    Pending,
    Ready(ImageReadback),
    Failed(ImageError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tile {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Tile {
    pub fn row_pitch(self) -> u32 {
        (self.width * TEXEL_BYTES).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
    }

    pub fn readback_bytes(self) -> u64 {
        u64::from(self.row_pitch()) * u64::from(self.height)
    }
}

pub fn check_size(size: SurfaceSize) -> Result<(), ImageError> {
    let fits = |side: u32| (1..=MAX_IMAGE_SIDE).contains(&side);
    if fits(size.width) && fits(size.height) {
        Ok(())
    } else {
        Err(ImageError::OutOfRange {
            width: size.width,
            height: size.height,
            largest: MAX_IMAGE_SIDE,
        })
    }
}

pub fn tiles(size: SurfaceSize, side: u32) -> Vec<Tile> {
    let side = side.max(1);
    let starts = |length: u32| (0..length).step_by(side as usize);
    starts(size.height)
        .flat_map(|y| {
            starts(size.width).map(move |x| Tile {
                x,
                y,
                width: side.min(size.width - x),
                height: side.min(size.height - y),
            })
        })
        .collect()
}

pub fn tile_transform(tile: Tile, size: SurfaceSize) -> [f32; 4] {
    let full = [f64::from(size.width), f64::from(size.height)];
    let center = [
        f64::from(tile.x) + f64::from(tile.width) / 2.0,
        f64::from(tile.y) + f64::from(tile.height) / 2.0,
    ];
    let ndc = [
        center[0] / full[0] * 2.0 - 1.0,
        1.0 - center[1] / full[1] * 2.0,
    ];
    let scale = [
        full[0] / f64::from(tile.width),
        full[1] / f64::from(tile.height),
    ];
    [
        scale[0] as f32,
        scale[1] as f32,
        (-ndc[0] * scale[0]) as f32,
        (-ndc[1] * scale[1]) as f32,
    ]
}

pub struct TileReadback {
    pub tile: Tile,
    pub buffer: wgpu::Buffer,
}

pub struct ImageReadback {
    size: SurfaceSize,
    order: ChannelOrder,
    tiles: Vec<TileReadback>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelOrder {
    Rgba,
    Bgra,
}

impl ChannelOrder {
    pub fn of(format: wgpu::TextureFormat) -> Option<Self> {
        match format {
            wgpu::TextureFormat::Rgba8Unorm => Some(Self::Rgba),
            wgpu::TextureFormat::Bgra8Unorm => Some(Self::Bgra),
            _ => None,
        }
    }
}

impl ImageReadback {
    pub fn new(size: SurfaceSize, order: ChannelOrder, tiles: Vec<TileReadback>) -> Self {
        Self { size, order, tiles }
    }

    pub fn into_image(self) -> Result<Image, ImageError> {
        let width = self.size.width as usize;
        let row_bytes = width * TEXEL_BYTES as usize;
        let mut pixels = vec![0u8; row_bytes * self.size.height as usize];
        for readback in &self.tiles {
            let tile = readback.tile;
            let mapped = readback
                .buffer
                .get_mapped_range(..)
                .map_err(|_| ImageError::Readback)?;
            let pitch = tile.row_pitch() as usize;
            let tile_bytes = tile.width as usize * TEXEL_BYTES as usize;
            for row in 0..tile.height as usize {
                let source = mapped
                    .get(row * pitch..row * pitch + tile_bytes)
                    .ok_or(ImageError::Readback)?;
                let start = (tile.y as usize + row) * row_bytes + tile.x as usize * 4;
                let target = pixels
                    .get_mut(start..start + tile_bytes)
                    .ok_or(ImageError::Readback)?;
                target.copy_from_slice(source);
            }
            drop(mapped);
            readback.buffer.unmap();
        }
        for texel in pixels.as_chunks_mut::<{ TEXEL_BYTES as usize }>().0 {
            straighten(texel, self.order);
        }
        Ok(Image {
            width: self.size.width,
            height: self.size.height,
            pixels,
        })
    }
}

fn straighten(texel: &mut [u8; 4], order: ChannelOrder) {
    let [first, green, third, alpha] = *texel;
    let (red, blue) = match order {
        ChannelOrder::Rgba => (first, third),
        ChannelOrder::Bgra => (third, first),
    };
    *texel = unpremultiply([red, green, blue], alpha);
}

pub fn unpremultiply(color: [u8; 3], alpha: u8) -> [u8; 4] {
    match alpha {
        0 => [0; 4],
        OPAQUE => [color[0], color[1], color[2], OPAQUE],
        _ => {
            let [red, green, blue] = color.map(|channel| {
                let straight = (u32::from(channel) * u32::from(OPAQUE) + u32::from(alpha) / 2)
                    / u32::from(alpha);
                straight.min(u32::from(OPAQUE)) as u8
            });
            [red, green, blue, alpha]
        }
    }
}

#[derive(Default)]
struct MapProgress {
    remaining: AtomicUsize,
    failed: AtomicBool,
}

pub struct PendingImage {
    generation: u64,
    readback: ImageReadback,
    progress: Arc<MapProgress>,
}

impl PendingImage {
    pub fn map(readback: ImageReadback, generation: u64) -> Self {
        let progress = Arc::new(MapProgress {
            remaining: AtomicUsize::new(readback.tiles.len()),
            failed: AtomicBool::new(false),
        });
        for tile in &readback.tiles {
            let progress = Arc::clone(&progress);
            tile.buffer
                .map_async(wgpu::MapMode::Read, .., move |result| {
                    if let Err(error) = result {
                        log::warn!("reading back an exported image failed: {error}");
                        progress.failed.store(true, Ordering::Release);
                    }
                    progress.remaining.fetch_sub(1, Ordering::AcqRel);
                });
        }
        Self {
            generation,
            readback,
            progress,
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn is_mapped(&self) -> bool {
        self.progress.remaining.load(Ordering::Acquire) == 0
    }

    pub fn finish(self) -> ImagePoll {
        if self.progress.failed.load(Ordering::Acquire) {
            ImagePoll::Failed(ImageError::Readback)
        } else {
            ImagePoll::Ready(self.readback)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(width: u32, height: u32) -> SurfaceSize {
        SurfaceSize { width, height }
    }

    #[test]
    fn tiles_cover_the_image_once_with_smaller_tiles_at_the_edges() {
        let image = size(500, 300);

        let tiles = tiles(image, 200);

        assert_eq!(tiles.len(), 6);
        assert_eq!(
            tiles.last(),
            Some(&Tile {
                x: 400,
                y: 200,
                width: 100,
                height: 100,
            })
        );
        let covered: u32 = tiles.iter().map(|tile| tile.width * tile.height).sum();
        assert_eq!(covered, 500 * 300);
        assert!(
            tiles
                .iter()
                .all(|tile| tile.x + tile.width <= 500 && tile.y + tile.height <= 300)
        );
    }

    #[test]
    fn a_tile_transform_maps_its_edges_to_the_edges_of_its_target() {
        let image = size(400, 200);
        let tile = Tile {
            x: 300,
            y: 0,
            width: 100,
            height: 100,
        };
        let [scale_x, scale_y, offset_x, offset_y] = tile_transform(tile, image);
        let to_tile = |ndc: [f32; 2]| [ndc[0] * scale_x + offset_x, ndc[1] * scale_y + offset_y];

        let left_top = to_tile([300.0 / 400.0 * 2.0 - 1.0, 1.0]);
        let right_bottom = to_tile([1.0, 0.0]);

        assert!((left_top[0] + 1.0).abs() < 1e-6 && (left_top[1] - 1.0).abs() < 1e-6);
        assert!((right_bottom[0] - 1.0).abs() < 1e-6 && (right_bottom[1] + 1.0).abs() < 1e-6);
        assert_eq!(
            tile_transform(
                Tile {
                    x: 0,
                    y: 0,
                    width: 400,
                    height: 200,
                },
                image
            ),
            [1.0, 1.0, 0.0, 0.0]
        );
    }

    #[test]
    fn readback_rows_are_padded_to_the_copy_alignment() {
        let tile = Tile {
            x: 0,
            y: 0,
            width: 65,
            height: 3,
        };

        assert_eq!(tile.row_pitch(), 512);
        assert_eq!(tile.readback_bytes(), 1536);
    }

    #[test]
    fn sizes_outside_one_to_the_largest_side_are_refused() {
        assert_eq!(check_size(size(MAX_IMAGE_SIDE, 1)), Ok(()));
        assert_eq!(
            check_size(size(MAX_IMAGE_SIDE + 1, 10)),
            Err(ImageError::OutOfRange {
                width: MAX_IMAGE_SIDE + 1,
                height: 10,
                largest: MAX_IMAGE_SIDE,
            })
        );
        assert!(check_size(size(0, 10)).is_err());
    }

    #[test]
    fn premultiplied_colour_is_made_straight_and_empty_pixels_stay_clear() {
        assert_eq!(unpremultiply([10, 20, 30], 255), [10, 20, 30, 255]);
        assert_eq!(unpremultiply([50, 0, 100], 128), [100, 0, 199, 128]);
        assert_eq!(unpremultiply([200, 200, 200], 100), [255, 255, 255, 100]);
        assert_eq!(unpremultiply([3, 4, 5], 0), [0, 0, 0, 0]);
    }

    #[test]
    fn blue_and_red_swap_back_from_a_bgra_target() {
        let mut texel = [30, 20, 10, 255];

        straighten(&mut texel, ChannelOrder::Bgra);

        assert_eq!(texel, [10, 20, 30, 255]);
    }
}
