use std::{borrow::Cow, fmt, sync::Arc};

use caditor_zstd::{Level, ZstdError, compress, decompress};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StepTextError {
    #[error("the kept STEP text could not be unpacked: {0}")]
    Unpacked(#[from] ZstdError),
    #[error("the kept STEP text unpacked to bytes that are not UTF-8")]
    NotUtf8,
}

enum Kept {
    Packed { frame: Box<[u8]>, length: usize },
    Plain(Box<str>),
}

#[derive(Clone)]
pub struct StepText(Arc<Kept>);

impl StepText {
    pub fn new(text: &str) -> Self {
        let kept = match compress(text.as_bytes(), Level::FAST) {
            Ok(frame) if frame.len() < text.len() => Kept::Packed {
                frame: frame.into_boxed_slice(),
                length: text.len(),
            },
            Ok(_) | Err(_) => Kept::Plain(text.into()),
        };
        Self(Arc::new(kept))
    }

    pub fn text(&self) -> Result<Cow<'_, str>, StepTextError> {
        match &*self.0 {
            Kept::Plain(text) => Ok(Cow::Borrowed(text)),
            Kept::Packed { frame, length } => String::from_utf8(decompress(frame, *length)?)
                .map(Cow::Owned)
                .map_err(|_| StepTextError::NotUtf8),
        }
    }

    pub fn len(&self) -> usize {
        match &*self.0 {
            Kept::Plain(text) => text.len(),
            Kept::Packed { length, .. } => *length,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn held_bytes(&self) -> usize {
        match &*self.0 {
            Kept::Plain(text) => text.len(),
            Kept::Packed { frame, .. } => frame.len(),
        }
    }

    pub fn address(&self) -> usize {
        Arc::as_ptr(&self.0).addr()
    }
}

impl PartialEq for StepText {
    fn eq(&self, other: &Self) -> bool {
        if Arc::ptr_eq(&self.0, &other.0) {
            return true;
        }
        match (&*self.0, &*other.0) {
            (
                Kept::Packed { frame, length },
                Kept::Packed {
                    frame: other,
                    length: same,
                },
            ) => length == same && frame == other,
            (Kept::Plain(text), Kept::Plain(other)) => text == other,
            (Kept::Packed { .. }, Kept::Plain(_)) | (Kept::Plain(_), Kept::Packed { .. }) => {
                self.len() == other.len()
                    && matches!((self.text(), other.text()), (Ok(text), Ok(other)) if text == other)
            }
        }
    }
}

impl fmt::Debug for StepText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StepText")
            .field("length", &self.len())
            .field("held", &self.held_bytes())
            .finish()
    }
}

impl From<&str> for StepText {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}

impl From<String> for StepText {
    fn from(text: String) -> Self {
        Self::new(&text)
    }
}

impl From<&String> for StepText {
    fn from(text: &String) -> Self {
        Self::new(text)
    }
}

impl From<Arc<str>> for StepText {
    fn from(text: Arc<str>) -> Self {
        Self::new(&text)
    }
}
