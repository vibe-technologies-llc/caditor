use std::{
    error::Error,
    io::{self, Cursor},
    path::Path,
    sync::Arc,
};

use caditor_document::{
    MAX_PICTURE_BYTES, MAX_PICTURE_SIDE, Picture, PictureError, PictureFormat, PixelSize,
    picture_format,
};
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};
use jxl_oxide::{AllocTracker, JxlImage, JxlThreadPool};

use crate::{
    read::{open_file, read_open},
    reason::ReadFailure,
};

const RGBA: usize = 4;
const MAX_DECODER_BYTES: u64 = 1 << 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaPixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl RgbaPixels {
    pub fn size(&self) -> PixelSize {
        PixelSize {
            width: self.width,
            height: self.height,
        }
    }
}

#[derive(Debug)]
pub struct ReadPicture {
    pub picture: Arc<Picture>,
    pub pixels: RgbaPixels,
}

#[derive(Debug, thiserror::Error)]
pub enum PictureReadError {
    #[error("{0}")]
    Reading(ReadFailure),
    #[error("{0}")]
    Refused(#[from] PictureError),
    #[error("the {} picture is damaged or uses a form caditor cannot decode ({source})", .format.name())]
    Undecodable {
        format: PictureFormat,
        source: image::ImageError,
    },
    #[error("the JPEG XL picture is damaged or uses a form caditor cannot decode ({0})")]
    UndecodableJpegXl(Box<dyn Error + Send + Sync>),
    #[error("the {} picture holds {channels} colour channels, which caditor cannot show", .format.name())]
    UnusualChannels {
        format: PictureFormat,
        channels: u32,
    },
    #[error("there is not enough memory to decode it")]
    OutOfMemory,
}

pub fn read_picture_file(path: &Path) -> Result<ReadPicture, PictureReadError> {
    let reading = |error: io::Error| PictureReadError::Reading(ReadFailure::of(&error));
    let file = open_file(path).map_err(reading)?;
    let length = file.metadata().map_err(reading)?.len();
    if usize::try_from(length).map_or(true, |length| length > MAX_PICTURE_BYTES) {
        return Err(PictureError::TooLarge.into());
    }
    let bytes = read_open(&file).map_err(reading)?;
    read_picture(bytes)
}

pub fn read_picture(bytes: Vec<u8>) -> Result<ReadPicture, PictureReadError> {
    let pixels = decode_picture(&bytes)?;
    let picture = Picture::new(bytes, pixels.size())?;
    Ok(ReadPicture {
        picture: Arc::new(picture),
        pixels,
    })
}

pub fn decode_picture(bytes: &[u8]) -> Result<RgbaPixels, PictureReadError> {
    let format = picture_format(bytes)?;
    let pixels = match image_format(format) {
        Some(known) => decode_with_image(bytes, format, known)?,
        None => decode_jpeg_xl(bytes)?,
    };
    PixelSize::checked(pixels.width, pixels.height)?;
    Ok(pixels)
}

fn image_format(format: PictureFormat) -> Option<ImageFormat> {
    Some(match format {
        PictureFormat::Png => ImageFormat::Png,
        PictureFormat::Jpeg => ImageFormat::Jpeg,
        PictureFormat::WebP => ImageFormat::WebP,
        PictureFormat::Gif => ImageFormat::Gif,
        PictureFormat::Bmp => ImageFormat::Bmp,
        PictureFormat::Tiff => ImageFormat::Tiff,
        PictureFormat::Qoi => ImageFormat::Qoi,
        PictureFormat::Tga => ImageFormat::Tga,
        PictureFormat::JpegXl => return None,
    })
}

fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_PICTURE_SIDE);
    limits.max_image_height = Some(MAX_PICTURE_SIDE);
    limits.max_alloc = Some(MAX_DECODER_BYTES);
    limits
}

fn reader(bytes: &[u8], format: ImageFormat) -> ImageReader<Cursor<&[u8]>> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits());
    reader
}

fn decode_with_image(
    bytes: &[u8],
    format: PictureFormat,
    known: ImageFormat,
) -> Result<RgbaPixels, PictureReadError> {
    let undecodable = |source| PictureReadError::Undecodable { format, source };
    let mut header = ImageReader::with_format(Cursor::new(bytes), known);
    header.no_limits();
    let (width, height) = header.into_dimensions().map_err(undecodable)?;
    PixelSize::checked(width, height)?;
    let mut decoder = reader(bytes, known).into_decoder().map_err(undecodable)?;
    let orientation = decoder.orientation().map_err(undecodable)?;
    let mut decoded = DynamicImage::from_decoder(decoder).map_err(|error| match error {
        image::ImageError::Limits(_) => PictureReadError::OutOfMemory,
        error => undecodable(error),
    })?;
    decoded.apply_orientation(orientation);
    let rgba = decoded.into_rgba8();
    Ok(RgbaPixels {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

fn decode_jpeg_xl(bytes: &[u8]) -> Result<RgbaPixels, PictureReadError> {
    let tracker = usize::try_from(MAX_DECODER_BYTES).map_or_else(
        |_| AllocTracker::with_limit(usize::MAX),
        AllocTracker::with_limit,
    );
    let image = JxlImage::builder()
        .pool(JxlThreadPool::none())
        .alloc_tracker(tracker)
        .read(Cursor::new(bytes))
        .map_err(PictureReadError::UndecodableJpegXl)?;
    PixelSize::checked(image.width(), image.height())?;
    let render = image
        .render_frame(0)
        .map_err(PictureReadError::UndecodableJpegXl)?;
    let mut stream = render.stream();
    let width = stream.width();
    let height = stream.height();
    let channels = stream.channels();
    PixelSize::checked(width, height)?;
    let pixel_count = usize::try_from(u64::from(width) * u64::from(height))
        .map_err(|_| PictureReadError::OutOfMemory)?;
    let samples = pixel_count
        .checked_mul(usize::try_from(channels).map_err(|_| PictureReadError::OutOfMemory)?)
        .ok_or(PictureReadError::OutOfMemory)?;
    let mut read = Vec::new();
    read.try_reserve_exact(samples)
        .map_err(|_| PictureReadError::OutOfMemory)?;
    read.resize(samples, 0_u8);
    stream.write_to_buffer(&mut read);
    let rgba = widened(&read, channels).ok_or(PictureReadError::UnusualChannels {
        format: PictureFormat::JpegXl,
        channels,
    })?;
    Ok(RgbaPixels {
        width,
        height,
        rgba,
    })
}

fn widened(samples: &[u8], channels: u32) -> Option<Vec<u8>> {
    let stride = usize::try_from(channels).ok()?;
    if stride == 0 {
        return None;
    }
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(samples.len() / stride * RGBA).ok()?;
    for pixel in samples.chunks_exact(stride) {
        let sample = |at: usize| pixel.get(at).copied();
        let opaque = u8::MAX;
        let texel = match stride {
            1 => [sample(0)?, sample(0)?, sample(0)?, opaque],
            2 => [sample(0)?, sample(0)?, sample(0)?, sample(1)?],
            3 => [sample(0)?, sample(1)?, sample(2)?, opaque],
            4 => [sample(0)?, sample(1)?, sample(2)?, sample(3)?],
            _ => return None,
        };
        rgba.extend_from_slice(&texel);
    }
    Some(rgba)
}
