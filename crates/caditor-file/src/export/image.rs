use std::path::Path;

use caditor_document::CancelToken;
use png::{BitDepth, ColorType, Compression, Encoder, EncodingError, SrgbRenderingIntent};

use crate::{reason::WriteFailure, save::write_atomically};

pub const PNG_EXTENSION: &str = "png";
const TEXEL_BYTES: u64 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbaImage<'a> {
    pub width: u32,
    pub height: u32,
    pub pixels: &'a [u8],
}

#[derive(Debug, thiserror::Error)]
pub enum ImageExportError {
    #[error("the image export was cancelled")]
    Cancelled,
    #[error("an image of {width} × {height} pixels needs {expected} bytes but holds {found}")]
    PixelCount {
        width: u32,
        height: u32,
        expected: u64,
        found: usize,
    },
    #[error("the image could not be encoded as PNG")]
    Encoding(#[source] EncodingError),
    #[error("{0}")]
    Writing(WriteFailure),
}

fn encode_png(image: &RgbaImage<'_>) -> Result<Vec<u8>, ImageExportError> {
    let expected = u64::from(image.width) * u64::from(image.height) * TEXEL_BYTES;
    if u64::try_from(image.pixels.len()).ok() != Some(expected) {
        return Err(ImageExportError::PixelCount {
            width: image.width,
            height: image.height,
            expected,
            found: image.pixels.len(),
        });
    }
    let mut encoded = Vec::new();
    let mut encoder = Encoder::new(&mut encoded, image.width, image.height);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    encoder.set_compression(Compression::Fast);
    encoder.set_source_srgb(SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header().map_err(ImageExportError::Encoding)?;
    writer
        .write_image_data(image.pixels)
        .map_err(ImageExportError::Encoding)?;
    writer.finish().map_err(ImageExportError::Encoding)?;
    Ok(encoded)
}

pub fn export_png(
    path: &Path,
    image: &RgbaImage<'_>,
    cancel: &CancelToken,
) -> Result<(), ImageExportError> {
    if cancel.is_cancelled() {
        return Err(ImageExportError::Cancelled);
    }
    let encoded = encode_png(image)?;
    if cancel.is_cancelled() {
        return Err(ImageExportError::Cancelled);
    }
    write_atomically(path, &encoded)
        .map_err(|error| ImageExportError::Writing(WriteFailure::of(&error)))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn gradient(width: u32, height: u32) -> Vec<u8> {
        (0..height)
            .flat_map(|y| (0..width).flat_map(move |x| [x as u8, y as u8, 200, (x + y) as u8]))
            .collect()
    }

    #[test]
    fn a_png_decodes_back_to_the_same_srgb_pixels_with_their_alpha() {
        let pixels = gradient(37, 11);
        let image = RgbaImage {
            width: 37,
            height: 11,
            pixels: &pixels,
        };

        let encoded = encode_png(&image).unwrap();
        let mut reader = png::Decoder::new(std::io::Cursor::new(encoded))
            .read_info()
            .unwrap();
        let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
        let frame = reader.next_frame(&mut decoded).unwrap();

        assert_eq!((frame.width, frame.height), (37, 11));
        assert_eq!(frame.color_type, ColorType::Rgba);
        assert_eq!(frame.bit_depth, BitDepth::Eight);
        assert_eq!(&decoded[..frame.buffer_size()], pixels.as_slice());
        assert!(reader.info().srgb.is_some());
    }

    #[test]
    fn pixels_that_do_not_match_the_size_are_refused() {
        let pixels = gradient(4, 4);

        let refused = encode_png(&RgbaImage {
            width: 5,
            height: 4,
            pixels: &pixels,
        });

        assert!(matches!(
            refused,
            Err(ImageExportError::PixelCount {
                expected: 80,
                found: 64,
                ..
            })
        ));
    }

    #[test]
    fn an_exported_png_replaces_the_file_whole_and_a_cancelled_one_writes_nothing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("view.png");
        let pixels = gradient(8, 6);
        let image = RgbaImage {
            width: 8,
            height: 6,
            pixels: &pixels,
        };
        fs::write(&path, b"old").unwrap();

        export_png(&path, &image, &CancelToken::never()).unwrap();
        let cancelled = export_png(
            &dir.path().join("cancelled.png"),
            &image,
            &CancelToken::new(|| true),
        );

        assert_eq!(fs::read(&path).unwrap(), encode_png(&image).unwrap());
        assert!(matches!(cancelled, Err(ImageExportError::Cancelled)));
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("view.png")]);
    }

    #[test]
    fn a_failed_write_says_why_in_words() {
        let dir = TempDir::new().unwrap();
        let pixels = gradient(2, 2);

        let failed = export_png(
            &dir.path().join("missing").join("view.png"),
            &RgbaImage {
                width: 2,
                height: 2,
                pixels: &pixels,
            },
            &CancelToken::never(),
        )
        .unwrap_err();

        assert_eq!(failed.to_string(), "its folder no longer exists");
    }
}
