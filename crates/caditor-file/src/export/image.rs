use std::{
    fs::File,
    io::{self, BufWriter, Write},
    path::Path,
};

use caditor_document::CancelToken;
use png::{BitDepth, ColorType, Compression, Encoder, EncodingError, SrgbRenderingIntent};

use crate::{reason::WriteFailure, save::replace_atomically};

pub const PNG_EXTENSION: &str = "png";
const TEXEL_BYTES: u64 = 4;
const COMPRESSED_CHUNK_BYTES: usize = 1 << 18;

pub trait PixelRows {
    type Error;

    fn width(&self) -> u32;

    fn height(&self) -> u32;

    fn next_rows(&mut self) -> Option<Result<&[u8], Self::Error>>;
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
        found: u64,
    },
    #[error("the image could not be encoded as PNG")]
    Encoding(#[source] EncodingError),
    #[error("{0}")]
    Writing(WriteFailure),
}

#[derive(Debug, thiserror::Error)]
pub enum PngExportError<E> {
    #[error("{0}")]
    Pixels(E),
    #[error(transparent)]
    Export(#[from] ImageExportError),
}

enum Halt<E> {
    Stopped(PngExportError<E>),
    Io(io::Error),
}

impl<E> Halt<E> {
    fn export(error: ImageExportError) -> Self {
        Self::Stopped(PngExportError::Export(error))
    }

    fn encoding(error: EncodingError) -> Self {
        match error {
            EncodingError::IoError(error) => Self::Io(error),
            error => Self::export(ImageExportError::Encoding(error)),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("writing the image stopped")]
struct Stopped;

pub fn export_png<R: PixelRows + ?Sized>(
    path: &Path,
    rows: &mut R,
    cancel: &CancelToken,
) -> Result<(), PngExportError<R::Error>> {
    if cancel.is_cancelled() {
        return Err(ImageExportError::Cancelled.into());
    }
    let mut stopped = None;
    let written = replace_atomically(path, |file| {
        write_png(file, rows, cancel).map_err(|halt| match halt {
            Halt::Io(error) => error,
            Halt::Stopped(error) => {
                stopped = Some(error);
                io::Error::other(Stopped)
            }
        })
    });
    match (stopped, written) {
        (Some(error), _) => Err(error),
        (None, Ok(())) => Ok(()),
        (None, Err(error)) => Err(ImageExportError::Writing(WriteFailure::of(&error)).into()),
    }
}

fn write_png<R: PixelRows + ?Sized>(
    file: &mut File,
    rows: &mut R,
    cancel: &CancelToken,
) -> Result<(), Halt<R::Error>> {
    let (width, height) = (rows.width(), rows.height());
    let expected = u64::from(width) * u64::from(height) * TEXEL_BYTES;
    let miscounted = |found| {
        Halt::export(ImageExportError::PixelCount {
            width,
            height,
            expected,
            found,
        })
    };

    let mut encoder = Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    encoder.set_compression(Compression::Fast);
    encoder.set_source_srgb(SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header().map_err(Halt::encoding)?;
    let mut stream = writer
        .stream_writer_with_size(COMPRESSED_CHUNK_BYTES)
        .map_err(Halt::encoding)?;

    let mut found = 0u64;
    while let Some(next) = rows.next_rows() {
        let pixels = next.map_err(|error| Halt::Stopped(PngExportError::Pixels(error)))?;
        found = found.saturating_add(pixels.len() as u64);
        if found > expected {
            return Err(miscounted(found));
        }
        stream.write_all(pixels).map_err(Halt::Io)?;
        if cancel.is_cancelled() {
            return Err(Halt::export(ImageExportError::Cancelled));
        }
    }
    if found != expected {
        return Err(miscounted(found));
    }
    stream.finish().map_err(Halt::encoding)?;
    writer.finish().map_err(Halt::encoding)
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, convert::Infallible, fs};

    use tempfile::TempDir;

    use super::*;

    struct Bands {
        width: u32,
        height: u32,
        bands: VecDeque<Result<Vec<u8>, &'static str>>,
        current: Vec<u8>,
    }

    impl Bands {
        fn of(width: u32, height: u32, rows_per_band: &[u32]) -> Self {
            let pixels = gradient(width, height);
            let mut start = 0;
            let bands = rows_per_band
                .iter()
                .map(|rows| {
                    let end = (start + (rows * width * 4) as usize).min(pixels.len());
                    let band = pixels[start..end].to_vec();
                    start = end;
                    Ok(band)
                })
                .collect();
            Self {
                width,
                height,
                bands,
                current: Vec::new(),
            }
        }
    }

    impl PixelRows for Bands {
        type Error = &'static str;

        fn width(&self) -> u32 {
            self.width
        }

        fn height(&self) -> u32 {
            self.height
        }

        fn next_rows(&mut self) -> Option<Result<&[u8], Self::Error>> {
            match self.bands.pop_front()? {
                Ok(band) => {
                    self.current = band;
                    Some(Ok(&self.current))
                }
                Err(error) => Some(Err(error)),
            }
        }
    }

    struct Whole<'a> {
        width: u32,
        height: u32,
        pixels: &'a [u8],
    }

    impl PixelRows for Whole<'_> {
        type Error = Infallible;

        fn width(&self) -> u32 {
            self.width
        }

        fn height(&self) -> u32 {
            self.height
        }

        fn next_rows(&mut self) -> Option<Result<&[u8], Self::Error>> {
            let pixels = std::mem::take(&mut self.pixels);
            (!pixels.is_empty()).then_some(Ok(pixels))
        }
    }

    fn gradient(width: u32, height: u32) -> Vec<u8> {
        (0..height)
            .flat_map(|y| (0..width).flat_map(move |x| [x as u8, y as u8, 200, (x + y) as u8]))
            .collect()
    }

    fn decoded(path: &Path) -> (png::OutputInfo, Vec<u8>, bool) {
        let mut reader = png::Decoder::new(std::io::BufReader::new(File::open(path).unwrap()))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let frame = reader.next_frame(&mut pixels).unwrap();
        pixels.truncate(frame.buffer_size());
        let srgb = reader.info().srgb.is_some();
        (frame, pixels, srgb)
    }

    fn names(dir: &TempDir) -> Vec<std::ffi::OsString> {
        fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect()
    }

    #[test]
    fn bands_stream_into_a_png_that_decodes_back_to_the_same_srgb_pixels() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("view.png");
        let mut bands = Bands::of(37, 11, &[4, 4, 3]);

        export_png(&path, &mut bands, &CancelToken::never()).unwrap();
        let (frame, pixels, srgb) = decoded(&path);

        assert_eq!((frame.width, frame.height), (37, 11));
        assert_eq!(frame.color_type, ColorType::Rgba);
        assert_eq!(frame.bit_depth, BitDepth::Eight);
        assert_eq!(pixels, gradient(37, 11));
        assert!(srgb);
    }

    #[test]
    fn pixels_that_do_not_match_the_size_are_refused_and_write_nothing() {
        let dir = TempDir::new().unwrap();
        let pixels = gradient(4, 4);

        let short = export_png(
            &dir.path().join("short.png"),
            &mut Whole {
                width: 5,
                height: 4,
                pixels: &pixels,
            },
            &CancelToken::never(),
        );
        let long = export_png(
            &dir.path().join("long.png"),
            &mut Whole {
                width: 3,
                height: 4,
                pixels: &pixels,
            },
            &CancelToken::never(),
        );

        assert!(matches!(
            short,
            Err(PngExportError::Export(ImageExportError::PixelCount {
                expected: 80,
                found: 64,
                ..
            }))
        ));
        assert!(matches!(
            long,
            Err(PngExportError::Export(ImageExportError::PixelCount {
                expected: 48,
                found: 64,
                ..
            }))
        ));
        assert!(names(&dir).is_empty());
    }

    #[test]
    fn rows_that_fail_part_way_keep_the_old_file_and_say_why() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("view.png");
        let mut bands = Bands::of(8, 6, &[2, 2]);
        bands.bands.push_back(Err("the device was lost"));
        fs::write(&path, b"old").unwrap();

        let failed = export_png(&path, &mut bands, &CancelToken::never());

        assert!(matches!(
            failed,
            Err(PngExportError::Pixels("the device was lost"))
        ));
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert_eq!(names(&dir), vec![std::ffi::OsString::from("view.png")]);
    }

    #[test]
    fn an_exported_png_replaces_the_file_whole_and_a_cancelled_one_writes_nothing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("view.png");
        let pixels = gradient(8, 6);
        let image = |pixels| Whole {
            width: 8,
            height: 6,
            pixels,
        };
        fs::write(&path, b"old").unwrap();

        export_png(&path, &mut image(&pixels), &CancelToken::never()).unwrap();
        let cancelled = export_png(
            &dir.path().join("cancelled.png"),
            &mut image(&pixels),
            &CancelToken::new(|| true),
        );

        assert_eq!(decoded(&path).1, pixels);
        assert!(matches!(
            cancelled,
            Err(PngExportError::Export(ImageExportError::Cancelled))
        ));
        assert_eq!(names(&dir), vec![std::ffi::OsString::from("view.png")]);
    }

    #[test]
    fn a_failed_write_says_why_in_words() {
        let dir = TempDir::new().unwrap();
        let pixels = gradient(2, 2);

        let failed = export_png(
            &dir.path().join("missing").join("view.png"),
            &mut Whole {
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
