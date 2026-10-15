use std::{io::Cursor, path::Path};

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};

use crate::{
    read::{open_file, read_open},
    reason::ReadFailure,
};

pub const MAX_ENVIRONMENT_PIXELS: u64 = 8192 * 4096;
const MAX_DECODER_BYTES: u64 = 1 << 30;
const MILLION: u64 = 1_000_000;

#[derive(Debug, Clone, PartialEq)]
pub struct EnvironmentPixels {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<f32>,
}

#[derive(Debug, thiserror::Error)]
pub enum EnvironmentReadError {
    #[error("{0}")]
    Reading(ReadFailure),
    #[error(
        "it is not a Radiance HDR picture (a .hdr file), the form caditor reads for an environment"
    )]
    NotRadiance,
    #[error("the Radiance picture is damaged or uses a form caditor cannot decode ({0})")]
    Undecodable(image::ImageError),
    #[error(
        "the picture is {width} by {height} pixels, more than the {} million pixels caditor reads for an environment; shrink it first",
        MAX_ENVIRONMENT_PIXELS / MILLION
    )]
    TooLarge { width: u32, height: u32 },
    #[error("there is not enough memory to decode it")]
    OutOfMemory,
}

pub fn read_environment_file(path: &Path) -> Result<EnvironmentPixels, EnvironmentReadError> {
    let reading = |error: std::io::Error| EnvironmentReadError::Reading(ReadFailure::of(&error));
    let file = open_file(path).map_err(reading)?;
    let bytes = read_open(&file).map_err(reading)?;
    decode_environment(&bytes)
}

pub fn decode_environment(bytes: &[u8]) -> Result<EnvironmentPixels, EnvironmentReadError> {
    if image::guess_format(bytes).ok() != Some(ImageFormat::Hdr) {
        return Err(EnvironmentReadError::NotRadiance);
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Hdr);
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_DECODER_BYTES);
    reader.limits(limits);
    let decoder = reader
        .into_decoder()
        .map_err(EnvironmentReadError::Undecodable)?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > MAX_ENVIRONMENT_PIXELS {
        return Err(EnvironmentReadError::TooLarge { width, height });
    }
    let decoded = DynamicImage::from_decoder(decoder).map_err(|error| match error {
        image::ImageError::Limits(_) => EnvironmentReadError::OutOfMemory,
        error => EnvironmentReadError::Undecodable(error),
    })?;
    let rgb = decoded.into_rgb32f();
    Ok(EnvironmentPixels {
        width: rgb.width(),
        height: rgb.height(),
        rgb: rgb.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use image::{Rgb, codecs::hdr::HdrEncoder};

    use super::*;

    fn radiance(width: u32, height: u32, paint: impl Fn(u32, u32) -> [f32; 3]) -> Vec<u8> {
        let texels: Vec<Rgb<f32>> = (0..height)
            .flat_map(|y| {
                let paint = &paint;
                (0..width).map(move |x| Rgb(paint(x, y)))
            })
            .collect();
        let mut bytes = Vec::new();
        HdrEncoder::new(&mut bytes)
            .encode(&texels, width as usize, height as usize)
            .unwrap();
        bytes
    }

    #[test]
    fn a_radiance_picture_decodes_to_linear_values_above_one() {
        let bytes = radiance(
            64,
            32,
            |x, _| if x < 32 { [8.0, 0.5, 0.25] } else { [0.0; 3] },
        );

        let pixels = decode_environment(&bytes).unwrap();

        assert_eq!((pixels.width, pixels.height), (64, 32));
        assert_eq!(pixels.rgb.len(), 64 * 32 * 3);
        for (found, expected) in pixels.rgb.iter().zip([8.0, 0.5, 0.25]) {
            assert!((found - expected).abs() < expected * 0.02, "{found}");
        }
        assert!(pixels.rgb.get(32 * 3).copied().unwrap() < 1e-3);
    }

    #[test]
    fn a_file_is_read_from_disk_and_a_missing_one_says_so() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("sky.hdr");
        std::fs::write(&path, radiance(16, 8, |_, _| [1.0; 3])).unwrap();

        let read = read_environment_file(&path).unwrap();
        let missing = read_environment_file(&folder.path().join("none.hdr")).unwrap_err();

        assert_eq!((read.width, read.height), (16, 8));
        assert!(matches!(
            missing,
            EnvironmentReadError::Reading(ReadFailure::NotFound)
        ));
    }

    #[test]
    fn other_pictures_and_damaged_ones_are_refused_in_words() {
        let mut png = Vec::new();
        DynamicImage::new_rgb8(4, 4)
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
        let mut damaged = radiance(16, 8, |_, _| [1.0; 3]);
        damaged.truncate(damaged.len() / 2);

        assert!(matches!(
            decode_environment(&png),
            Err(EnvironmentReadError::NotRadiance)
        ));
        assert!(matches!(
            decode_environment(b"not a picture"),
            Err(EnvironmentReadError::NotRadiance)
        ));
        assert!(matches!(
            decode_environment(&damaged),
            Err(EnvironmentReadError::Undecodable(_))
        ));
        assert!(
            EnvironmentReadError::NotRadiance
                .to_string()
                .contains("Radiance")
        );
    }

    #[test]
    fn a_picture_too_large_to_hold_is_refused_before_its_pixels_are_read() {
        let header = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 8192 +X 16384\n";

        let refused = decode_environment(header).unwrap_err();

        assert!(
            matches!(
                refused,
                EnvironmentReadError::TooLarge {
                    width: 16384,
                    height: 8192
                }
            ),
            "{refused}"
        );
    }
}
