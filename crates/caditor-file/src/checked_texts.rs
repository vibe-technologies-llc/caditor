#[cfg(test)]
mod tests;

use std::{
    collections::{BTreeSet, VecDeque},
    io,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use parking_lot::Mutex;
use xxhash_rust::xxh3::xxh3_64;

use crate::{lock::locked_update, read::read_file, reason::UpdateFailure, save::write_atomically};

const CHECKED_FILE: &str = "checked-imports";
const LOCK_FILE: &str = "checked-imports.lock";
const MAGIC: &[u8; 8] = b"CDCHKD01";
const DIGEST_BYTES: usize = blake3::OUT_LEN;
const CHECKSUM_BYTES: usize = 8;
pub(crate) const MAX_CHECKED: usize = 100_000;

type Digest = [u8; DIGEST_BYTES];

#[derive(Debug, Default)]
pub(crate) struct CheckedTexts {
    order: VecDeque<Digest>,
    held: BTreeSet<Digest>,
    unsaved: bool,
    dir: Option<PathBuf>,
}

impl CheckedTexts {
    pub fn kept_in(dir: &Path) -> Self {
        let mut checked = Self {
            dir: Some(dir.to_path_buf()),
            ..Self::default()
        };
        checked.absorb(stored_in(dir));
        checked
    }

    pub fn contains(&self, digest: &blake3::Hash) -> bool {
        self.held.contains(digest.as_bytes())
    }

    pub fn mark(&mut self, digest: blake3::Hash) {
        if self.add(*digest.as_bytes()) {
            self.unsaved = true;
        }
    }

    pub fn persist(&mut self) -> Result<(), UpdateFailure> {
        let Some(dir) = self.dir.clone().filter(|_| self.unsaved) else {
            return Ok(());
        };
        locked_update(&dir, LOCK_FILE, || {
            let mut merged = Self::default();
            merged.absorb(stored_in(&dir));
            merged.absorb(self.order.iter().copied());
            write_atomically(&dir.join(CHECKED_FILE), &merged.encoded())
                .map_err(|error| UpdateFailure::writing(&error))?;
            self.absorb(merged.order);
            Ok(())
        })?;
        self.unsaved = false;
        Ok(())
    }

    fn absorb(&mut self, digests: impl IntoIterator<Item = Digest>) {
        for digest in digests {
            self.add(digest);
        }
    }

    fn add(&mut self, digest: Digest) -> bool {
        if !self.held.insert(digest) {
            return false;
        }
        self.order.push_back(digest);
        while self.order.len() > MAX_CHECKED {
            if let Some(oldest) = self.order.pop_front() {
                self.held.remove(&oldest);
            }
        }
        true
    }

    fn encoded(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MAGIC.len() + self.order.len() * DIGEST_BYTES + 8);
        bytes.extend_from_slice(MAGIC);
        for digest in &self.order {
            bytes.extend_from_slice(digest);
        }
        let checksum = xxh3_64(&bytes);
        bytes.extend_from_slice(&checksum.to_le_bytes());
        bytes
    }
}

fn decoded(bytes: &[u8]) -> Option<Vec<Digest>> {
    let (body, checksum) = bytes.split_at_checked(bytes.len().checked_sub(CHECKSUM_BYTES)?)?;
    let stored = u64::from_le_bytes(checksum.try_into().ok()?);
    if xxh3_64(body) != stored {
        return None;
    }
    let digests = body.strip_prefix(MAGIC.as_slice())?;
    let (whole, rest) = digests.as_chunks::<DIGEST_BYTES>();
    rest.is_empty().then(|| whole.to_vec())
}

fn stored_in(dir: &Path) -> Vec<Digest> {
    let path = dir.join(CHECKED_FILE);
    match read_file(&path) {
        Ok(bytes) => decoded(&bytes).unwrap_or_else(|| {
            log::warn!(
                "{} was unreadable, so every import is checked again",
                path.display()
            );
            Vec::new()
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            log::warn!("could not read {}: {error}", path.display());
            Vec::new()
        }
    }
}

static CHECKED: LazyLock<Mutex<CheckedTexts>> = LazyLock::new(Mutex::default);

pub fn remember_checked_imports(state_dir: &Path) {
    *CHECKED.lock() = CheckedTexts::kept_in(state_dir);
}

pub(crate) fn is_checked(digest: &blake3::Hash) -> bool {
    CHECKED.lock().contains(digest)
}

pub(crate) fn mark_checked(digest: blake3::Hash) {
    CHECKED.lock().mark(digest);
}

pub(crate) fn persist_checked() {
    if let Err(error) = CHECKED.lock().persist() {
        log::warn!("the imports checked on this computer could not be kept: {error}");
    }
}
