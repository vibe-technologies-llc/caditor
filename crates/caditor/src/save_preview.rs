use std::{
    panic::{self, AssertUnwindSafe},
    thread,
};

use caditor_document::CancelToken;
use caditor_file::{PngExportError, Preview, PreviewDrawing, PreviewError, PreviewSlot, png_bytes};
use caditor_render::{ImageError, SurfaceSize};

use crate::image_export::ReadPixels;

pub const PREVIEW_SIZE: SurfaceSize = SurfaceSize {
    width: 256,
    height: 192,
};

#[derive(Debug, thiserror::Error)]
pub enum PreviewFailure {
    #[error("the picture could not be encoded: {0}")]
    Encoding(#[from] PngExportError<ImageError>),
    #[error("the picture cannot be kept: {0}")]
    Unfit(#[from] PreviewError),
    #[error("drawing the picture panicked")]
    Panicked,
}

struct Waiting {
    revision: u64,
    drawing: PreviewDrawing,
}

#[derive(Default)]
pub struct SavePreviews {
    enabled: bool,
    waiting: Option<Waiting>,
}

impl SavePreviews {
    pub fn enable(&mut self) {
        self.enabled = true;
    }

    pub fn request(&mut self, revision: u64, has_bodies: bool) -> Option<PreviewSlot> {
        self.waiting = None;
        if !self.enabled || !has_bodies {
            return None;
        }
        let (slot, drawing) = PreviewSlot::new();
        self.waiting = Some(Waiting { revision, drawing });
        Some(slot)
    }

    pub fn take(&mut self, revision: u64) -> Option<PreviewDrawing> {
        let waiting = self.waiting.take()?;
        (waiting.revision == revision && waiting.drawing.is_wanted()).then_some(waiting.drawing)
    }

    #[cfg(test)]
    pub fn is_waiting(&self) -> bool {
        self.waiting.is_some()
    }
}

pub fn finish(drawing: PreviewDrawing, rows: ReadPixels) {
    let spawned = thread::Builder::new()
        .name("preview".to_owned())
        .spawn(move || {
            let kept = panic::catch_unwind(AssertUnwindSafe(|| keep(rows)))
                .unwrap_or(Err(PreviewFailure::Panicked));
            match kept {
                Ok(preview) => drawing.fill(preview),
                Err(error) => {
                    log::info!("the model is saved without a picture: {error}");
                    drawing.decline();
                }
            }
        });
    if let Err(error) = spawned {
        log::warn!("the model is saved without a picture, no thread could start: {error}");
    }
}

fn keep(mut rows: ReadPixels) -> Result<Preview, PreviewFailure> {
    let png = png_bytes(&mut *rows, &CancelToken::never())?;
    Ok(Preview::from_png(png)?)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use caditor_file::PixelRows;

    use super::*;

    const PATIENCE: Duration = Duration::from_secs(10);

    struct Flat {
        given: bool,
        row: Vec<u8>,
    }

    impl PixelRows for Flat {
        type Error = ImageError;

        fn width(&self) -> u32 {
            PREVIEW_SIZE.width
        }

        fn height(&self) -> u32 {
            PREVIEW_SIZE.height
        }

        fn next_rows(&mut self) -> Option<Result<&[u8], ImageError>> {
            if std::mem::replace(&mut self.given, true) {
                return None;
            }
            Some(Ok(&self.row))
        }
    }

    fn flat() -> ReadPixels {
        let texels = (PREVIEW_SIZE.width * PREVIEW_SIZE.height) as usize;
        Box::new(Flat {
            given: false,
            row: [40, 90, 160, 255].repeat(texels),
        })
    }

    fn enabled() -> SavePreviews {
        let mut previews = SavePreviews::default();
        previews.enable();
        previews
    }

    #[test]
    fn nothing_is_asked_for_unless_enabled_and_the_model_has_bodies() {
        let mut disabled = SavePreviews::default();
        let mut previews = enabled();

        assert!(disabled.request(1, true).is_none());
        assert!(!disabled.is_waiting());
        assert!(previews.request(1, false).is_none());
        assert!(!previews.is_waiting());
    }

    #[test]
    fn the_picture_is_drawn_only_for_the_revision_that_was_saved() {
        let mut previews = enabled();

        let slot = previews.request(4, true).unwrap();
        let stale = previews.take(5);

        assert!(stale.is_none());
        assert_eq!(slot.wait_at_most(PATIENCE), None);

        let slot = previews.request(4, true).unwrap();
        let drawing = previews.take(4).unwrap();
        finish(drawing, flat());

        let preview = slot.wait_at_most(PATIENCE).unwrap();
        assert!(preview.png().starts_with(&[0x89, b'P', b'N', b'G']));
        assert!(previews.take(4).is_none());
    }

    #[test]
    fn a_newer_save_replaces_the_picture_still_waited_for() {
        let mut previews = enabled();
        let first = previews.request(1, true).unwrap();

        let second = previews.request(2, true).unwrap();

        assert_eq!(first.wait_at_most(PATIENCE), None);
        assert!(previews.take(2).is_some());
        drop(second);
    }

    #[test]
    fn a_picture_that_cannot_be_encoded_declines_instead_of_hanging_the_save() {
        let mut previews = enabled();
        let slot = previews.request(1, true).unwrap();
        let drawing = previews.take(1).unwrap();

        finish(
            drawing,
            Box::new(Flat {
                given: false,
                row: vec![0; 3],
            }),
        );

        assert_eq!(slot.wait_at_most(PATIENCE), None);
    }
}
