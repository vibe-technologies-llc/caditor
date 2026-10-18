use std::{
    fs::File,
    io::{self, Read},
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use parking_lot::{Condvar, Mutex};

use crate::{binary, read::open_file, reason::ReadFailure};

pub const MAX_PREVIEW_BYTES: usize = 512 << 10;
pub const PREVIEW_PATIENCE: Duration = Duration::from_millis(1500);
pub(crate) const PREVIEW_PREFIX: u64 = MAX_PREVIEW_BYTES as u64 + (64 << 10);
pub(crate) const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview(Arc<[u8]>);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PreviewError {
    #[error("the picture is not a PNG")]
    NotPng,
    #[error("the picture takes {size} bytes, more than the {MAX_PREVIEW_BYTES} a model keeps")]
    TooLarge { size: usize },
}

impl Preview {
    pub fn from_png(bytes: Vec<u8>) -> Result<Self, PreviewError> {
        if !bytes.starts_with(&PNG_SIGNATURE) {
            return Err(PreviewError::NotPng);
        }
        if bytes.len() > MAX_PREVIEW_BYTES {
            return Err(PreviewError::TooLarge { size: bytes.len() });
        }
        Ok(Self(bytes.into()))
    }

    pub fn png(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Debug)]
enum Drawn {
    Waiting,
    Ready(Preview),
    Declined,
    Closed,
}

#[derive(Debug)]
struct Shared {
    state: Mutex<Drawn>,
    changed: Condvar,
}

#[derive(Debug, Clone)]
pub struct PreviewSlot(Arc<Shared>);

impl PartialEq for PreviewSlot {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for PreviewSlot {}

#[derive(Debug)]
pub struct PreviewDrawing(Arc<Shared>);

impl PreviewSlot {
    pub fn new() -> (Self, PreviewDrawing) {
        let shared = Arc::new(Shared {
            state: Mutex::new(Drawn::Waiting),
            changed: Condvar::new(),
        });
        (Self(Arc::clone(&shared)), PreviewDrawing(shared))
    }

    pub(crate) fn wait(&self) -> Option<Preview> {
        self.wait_at_most(PREVIEW_PATIENCE)
    }

    pub fn wait_at_most(&self, patience: Duration) -> Option<Preview> {
        let deadline = Instant::now() + patience;
        let mut state = self.0.state.lock();
        while matches!(*state, Drawn::Waiting) {
            if self.0.changed.wait_until(&mut state, deadline).timed_out() {
                break;
            }
        }
        match std::mem::replace(&mut *state, Drawn::Closed) {
            Drawn::Ready(preview) => Some(preview),
            Drawn::Waiting | Drawn::Declined | Drawn::Closed => None,
        }
    }
}

impl PreviewDrawing {
    pub fn is_wanted(&self) -> bool {
        matches!(*self.0.state.lock(), Drawn::Waiting)
    }

    pub fn fill(self, preview: Preview) {
        self.settle(Drawn::Ready(preview));
    }

    pub fn decline(self) {
        self.settle(Drawn::Declined);
    }

    fn settle(&self, outcome: Drawn) {
        let mut state = self.0.state.lock();
        if matches!(*state, Drawn::Waiting) {
            *state = outcome;
            self.0.changed.notify_all();
        }
    }
}

impl Drop for PreviewDrawing {
    fn drop(&mut self) {
        self.settle(Drawn::Declined);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PreviewReadError {
    #[error("{0}")]
    Reading(ReadFailure),
}

pub fn read_preview(path: &Path) -> Result<Option<Preview>, PreviewReadError> {
    let reading = |error: io::Error| PreviewReadError::Reading(ReadFailure::of(&error));
    let file = open_file(path).map_err(reading)?;
    let mut prefix = Vec::new();
    File::take(file, PREVIEW_PREFIX)
        .read_to_end(&mut prefix)
        .map_err(reading)?;
    Ok(binary::preview_of_prefix(&prefix))
}
