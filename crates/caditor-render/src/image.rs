use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver, RecvError, Sender, TryRecvError},
    },
    thread,
    time::Duration,
};

use parking_lot::Mutex;

use crate::{
    SurfaceSize,
    camera::View,
    gpu::{self, DeviceLoss, Wake},
    scene::Scene,
    viewport::{ImagePlan, ViewportRenderer},
};

pub const MAX_IMAGE_SIDE: u32 = 8192;
pub const TILE_SIDE: u32 = 2048;
pub const IMAGE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub const READBACK_BUFFERS: usize = 3;
const MAPPING_POLL_INTERVAL: Duration = Duration::from_millis(1);
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

#[cfg(test)]
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
    #[error("drawing the image stopped before it was finished")]
    Abandoned,
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

type MapOutcome = Arc<Mutex<Option<Result<(), wgpu::BufferAsyncError>>>>;

struct DrawnTile {
    tile: Tile,
    buffer: wgpu::Buffer,
    mapped: MapOutcome,
}

type TileMessage = Result<DrawnTile, ImageError>;

pub struct ImageGpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub loss: DeviceLoss,
}

pub struct ImageTiles {
    gpu: ImageGpu,
    renderer: ViewportRenderer,
    plan: ImagePlan,
    remaining: std::vec::IntoIter<Tile>,
    spare: Vec<wgpu::Buffer>,
    made: usize,
    buffer_bytes: u64,
    returned: Receiver<wgpu::Buffer>,
    drawn: Option<Sender<TileMessage>>,
}

pub fn start(
    gpu: ImageGpu,
    renderer: impl FnOnce() -> ViewportRenderer,
    request: &ImageRequest<'_>,
    tile_side: u32,
    wake: Wake,
) -> Result<(ImageTiles, ImageBands), ImageError> {
    check_size(request.size)?;
    if gpu.loss.is_lost() {
        return Err(ImageError::DeviceLost);
    }
    let tile_side = tile_side.max(1);
    let (prepared, error) = gpu::scoped(&gpu.device, || {
        let mut renderer = renderer();
        renderer
            .prepare_image(&gpu.device, &gpu.queue, request, tile_side)
            .map(|plan| (renderer, plan))
    });
    if let Some(error) = error {
        return Err(refusal(&error));
    }
    let (renderer, plan) = prepared.ok_or(ImageError::Refused)?;

    let tiles = tiles(request.size, tile_side);
    let buffer_bytes = tiles.first().map_or(0, |tile| tile.readback_bytes());
    let (drawn_sender, drawn) = mpsc::channel();
    let (returned_sender, returned) = mpsc::channel();
    let bands = ImageBands {
        device: gpu.device.clone(),
        loss: gpu.loss.clone(),
        size: request.size,
        order: plan.order(),
        link: Some(Link {
            drawn,
            returned: returned_sender,
        }),
        inline: None,
        wake,
        band: Vec::new(),
        next_row: 0,
    };
    let producer = ImageTiles {
        gpu,
        renderer,
        plan,
        remaining: tiles.into_iter(),
        spare: Vec::new(),
        made: 0,
        buffer_bytes,
        returned,
        drawn: Some(drawn_sender),
    };
    Ok((producer, bands))
}

pub fn drawn_inline(
    gpu: ImageGpu,
    renderer: impl FnOnce() -> ViewportRenderer,
    request: &ImageRequest<'_>,
    tile_side: u32,
) -> Result<ImageBands, ImageError> {
    let (tiles, mut bands) = start(gpu, renderer, request, tile_side, Arc::new(|| {}))?;
    bands.inline = Some(Box::new(tiles));
    Ok(bands)
}

fn refusal(error: &wgpu::Error) -> ImageError {
    match error {
        wgpu::Error::OutOfMemory { .. } => {
            log::warn!("the exported image did not fit in graphics memory: {error}");
            ImageError::OutOfMemory
        }
        _ => {
            log::warn!("the graphics device refused to draw the exported image: {error}");
            ImageError::Refused
        }
    }
}

impl ImageTiles {
    pub fn is_finished(&self) -> bool {
        self.drawn.is_none()
    }

    #[cfg(test)]
    pub fn buffers_made(&self) -> usize {
        self.made
    }

    pub fn advance(&mut self) {
        while self.drawn.is_some() {
            if self.remaining.as_slice().is_empty() {
                self.drawn = None;
                return;
            }
            let Some(buffer) = self.free_buffer() else {
                return;
            };
            self.draw_next(buffer);
        }
    }

    fn free_buffer(&mut self) -> Option<Option<wgpu::Buffer>> {
        loop {
            match self.returned.try_recv() {
                Ok(buffer) => self.spare.push(buffer),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.drawn = None;
                    return None;
                }
            }
        }
        match self.spare.pop() {
            Some(buffer) => Some(Some(buffer)),
            None => (self.made < READBACK_BUFFERS).then_some(None),
        }
    }

    fn draw_next(&mut self, reused: Option<wgpu::Buffer>) {
        let Some(tile) = self.remaining.next() else {
            return;
        };
        if self.gpu.loss.is_lost() {
            self.fail(ImageError::DeviceLost);
            return;
        }
        let created = reused.is_none();
        let gpu = &self.gpu;
        let renderer = &mut self.renderer;
        let plan = &self.plan;
        let buffer_bytes = self.buffer_bytes;
        let (buffer, error) = gpu::scoped(&gpu.device, || {
            let buffer = reused.unwrap_or_else(|| {
                gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("image readback"),
                    size: buffer_bytes,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                })
            });
            renderer.draw_tile(&gpu.device, &gpu.queue, plan, tile, &buffer);
            buffer
        });
        if created {
            self.made += 1;
        }
        if let Some(error) = error {
            self.fail(refusal(&error));
            return;
        }

        let mapped = MapOutcome::default();
        let sink = Arc::clone(&mapped);
        buffer.map_async(wgpu::MapMode::Read, .., move |result| {
            *sink.lock() = Some(result);
        });
        let sent = self.drawn.as_ref().map(|drawn| {
            drawn.send(Ok(DrawnTile {
                tile,
                buffer,
                mapped,
            }))
        });
        if !matches!(sent, Some(Ok(()))) {
            self.drawn = None;
        }
    }

    fn fail(&mut self, error: ImageError) {
        if let Some(drawn) = self.drawn.take() {
            let _ = drawn.send(Err(error));
        }
    }
}

struct Link {
    drawn: Receiver<TileMessage>,
    returned: Sender<wgpu::Buffer>,
}

pub struct ImageBands {
    device: wgpu::Device,
    loss: DeviceLoss,
    size: SurfaceSize,
    order: ChannelOrder,
    link: Option<Link>,
    inline: Option<Box<ImageTiles>>,
    wake: Wake,
    band: Vec<u8>,
    next_row: u32,
}

impl ImageBands {
    pub fn size(&self) -> SurfaceSize {
        self.size
    }

    pub fn next_band(&mut self) -> Option<Result<&[u8], ImageError>> {
        if self.next_row >= self.size.height {
            return None;
        }
        match self.fill_band() {
            Ok(bytes) => Some(self.band.get(..bytes).ok_or(ImageError::Readback)),
            Err(error) => {
                self.next_row = self.size.height;
                self.hang_up();
                Some(Err(error))
            }
        }
    }

    fn fill_band(&mut self) -> Result<usize, ImageError> {
        let row_bytes = self.size.width as usize * TEXEL_BYTES as usize;
        let mut covered = 0;
        let mut rows = 0;
        while covered < self.size.width {
            let drawn = self.next_tile()?;
            let tile = drawn.tile;
            if tile.y != self.next_row || tile.x != covered {
                return Err(ImageError::Readback);
            }
            self.wait_until_mapped(&drawn.mapped)?;

            rows = tile.height as usize;
            let needed = rows * row_bytes;
            if self.band.len() < needed {
                self.band.resize(needed, 0);
            }
            let copied = copy_tile(&drawn, &mut self.band, row_bytes);
            drawn.buffer.unmap();
            if let Some(link) = &self.link {
                let _ = link.returned.send(drawn.buffer);
            }
            (self.wake)();
            copied?;
            covered += tile.width;
        }

        let bytes = rows * row_bytes;
        let band = self.band.get_mut(..bytes).ok_or(ImageError::Readback)?;
        for texel in band.as_chunks_mut::<{ TEXEL_BYTES as usize }>().0 {
            straighten(texel, self.order);
        }
        self.next_row = self.next_row.saturating_add(rows as u32);
        Ok(bytes)
    }

    fn next_tile(&mut self) -> Result<DrawnTile, ImageError> {
        if let Some(tiles) = self.inline.as_mut() {
            tiles.advance();
        }
        let link = self.link.as_ref().ok_or(ImageError::Abandoned)?;
        match link.drawn.recv() {
            Ok(message) => message,
            Err(RecvError) => Err(ImageError::Abandoned),
        }
    }

    fn wait_until_mapped(&self, mapped: &MapOutcome) -> Result<(), ImageError> {
        loop {
            if let Some(outcome) = mapped.lock().take() {
                return outcome.map_err(|error| {
                    log::warn!("reading back an exported image failed: {error}");
                    ImageError::Readback
                });
            }
            if self.loss.is_lost() {
                return Err(ImageError::DeviceLost);
            }
            if let Err(error) = self.device.poll(wgpu::PollType::Poll) {
                log::warn!("could not poll the graphics device for an exported image: {error}");
                return Err(ImageError::Readback);
            }
            if mapped.lock().is_none() {
                thread::sleep(MAPPING_POLL_INTERVAL);
            }
        }
    }

    fn hang_up(&mut self) {
        self.inline = None;
        if self.link.take().is_some() {
            (self.wake)();
        }
    }

    #[cfg(test)]
    pub fn buffers_made(&self) -> Option<usize> {
        self.inline.as_ref().map(|tiles| tiles.buffers_made())
    }

    #[cfg(test)]
    pub fn into_image(mut self) -> Result<Image, ImageError> {
        let mut pixels = Vec::new();
        while let Some(band) = self.next_band() {
            pixels.extend_from_slice(band?);
        }
        Ok(Image {
            width: self.size.width,
            height: self.size.height,
            pixels,
        })
    }
}

impl Drop for ImageBands {
    fn drop(&mut self) {
        self.hang_up();
    }
}

fn copy_tile(drawn: &DrawnTile, band: &mut [u8], row_bytes: usize) -> Result<(), ImageError> {
    let tile = drawn.tile;
    let mapped = drawn
        .buffer
        .get_mapped_range(..)
        .map_err(|_| ImageError::Readback)?;
    let pitch = tile.row_pitch() as usize;
    let tile_bytes = tile.width as usize * TEXEL_BYTES as usize;
    let left = tile.x as usize * TEXEL_BYTES as usize;
    for row in 0..tile.height as usize {
        let source = mapped
            .get(row * pitch..row * pitch + tile_bytes)
            .ok_or(ImageError::Readback)?;
        let start = row * row_bytes + left;
        band.get_mut(start..start + tile_bytes)
            .ok_or(ImageError::Readback)?
            .copy_from_slice(source);
    }
    Ok(())
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
    fn tiles_come_in_bands_from_the_top_left_to_the_right() {
        let tiles = tiles(size(500, 300), 200);

        let starts: Vec<(u32, u32)> = tiles.iter().map(|tile| (tile.x, tile.y)).collect();

        assert_eq!(
            starts,
            [(0, 0), (200, 0), (400, 0), (0, 200), (200, 200), (400, 200)]
        );
        assert!(
            tiles
                .iter()
                .all(|tile| tile.readback_bytes() <= tiles[0].readback_bytes())
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
