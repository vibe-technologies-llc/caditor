use std::{
    collections::{BTreeMap, BTreeSet},
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

use caditor_document::{Document, FeatureKind, Picture, PictureDigest};
use caditor_file::{PictureReadError, RgbaPixels, decode_picture};
use caditor_render::PictureTexture;

use crate::model::Waker;

#[derive(Debug, thiserror::Error)]
pub enum PictureFailure {
    #[error(transparent)]
    Undecodable(#[from] PictureReadError),
    #[error("the picture could not be prepared for showing")]
    Unshowable,
}

type Decoded = (PictureDigest, Result<Arc<PictureTexture>, PictureFailure>);

#[derive(Debug)]
enum Entry {
    Decoding,
    Ready(Arc<PictureTexture>),
    Failed(PictureFailure),
}

pub struct ImageTextures {
    entries: BTreeMap<PictureDigest, Entry>,
    sender: Sender<Decoded>,
    receiver: Receiver<Decoded>,
    generation: u64,
    wanted_for: Option<u64>,
}

impl Default for ImageTextures {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            entries: BTreeMap::new(),
            sender,
            receiver,
            generation: 0,
            wanted_for: None,
        }
    }
}

pub fn texture_of(pixels: RgbaPixels) -> Result<Arc<PictureTexture>, PictureFailure> {
    PictureTexture::new(pixels.width, pixels.height, pixels.rgba)
        .map(Arc::new)
        .ok_or(PictureFailure::Unshowable)
}

fn decoded(picture: &Picture) -> Result<Arc<PictureTexture>, PictureFailure> {
    decode_picture(picture.bytes())
        .map_err(PictureFailure::from)
        .and_then(texture_of)
}

impl ImageTextures {
    pub fn decoded_for(document: &Document) -> Self {
        let mut textures = Self::default();
        for feature in document.features() {
            let FeatureKind::ReferenceImage(image) = &feature.kind else {
                continue;
            };
            let digest = image.picture.digest();
            if textures.entries.contains_key(&digest) {
                continue;
            }
            let entry = match decoded(&image.picture) {
                Ok(texture) => Entry::Ready(texture),
                Err(reason) => {
                    log::warn!(
                        "the reference image “{}” is left out: {reason}",
                        feature.name
                    );
                    Entry::Failed(reason)
                }
            };
            textures.entries.insert(digest, entry);
        }
        textures.generation += 1;
        textures
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn texture(&self, digest: PictureDigest) -> Option<&Arc<PictureTexture>> {
        match self.entries.get(&digest) {
            Some(Entry::Ready(texture)) => Some(texture),
            Some(Entry::Decoding | Entry::Failed(_)) | None => None,
        }
    }

    pub fn failure(&self, digest: PictureDigest) -> Option<&PictureFailure> {
        match self.entries.get(&digest) {
            Some(Entry::Failed(reason)) => Some(reason),
            Some(Entry::Decoding | Entry::Ready(_)) | None => None,
        }
    }

    pub fn is_decoding(&self, digest: PictureDigest) -> bool {
        matches!(self.entries.get(&digest), Some(Entry::Decoding))
    }

    #[cfg(test)]
    pub fn pending(&self) -> bool {
        self.entries
            .values()
            .any(|entry| matches!(entry, Entry::Decoding))
    }

    pub fn keep(&mut self, digest: PictureDigest, texture: Arc<PictureTexture>) {
        self.entries.insert(digest, Entry::Ready(texture));
        self.generation += 1;
    }

    pub fn want(&mut self, document: &Document, revision: u64, make_waker: &dyn Fn() -> Waker) {
        if self.wanted_for == Some(revision) {
            return;
        }
        self.wanted_for = Some(revision);
        let pictures: BTreeMap<PictureDigest, Arc<Picture>> = document
            .features()
            .filter_map(|feature| match &feature.kind {
                FeatureKind::ReferenceImage(image) => {
                    Some((image.picture.digest(), Arc::clone(&image.picture)))
                }
                _ => None,
            })
            .collect();
        let wanted: BTreeSet<PictureDigest> = pictures.keys().copied().collect();
        let before = self.entries.len();
        self.entries
            .retain(|digest, entry| wanted.contains(digest) || matches!(entry, Entry::Decoding));
        if self.entries.len() != before {
            self.generation += 1;
        }
        for (digest, picture) in pictures {
            if self.entries.contains_key(&digest) {
                continue;
            }
            self.entries.insert(digest, Entry::Decoding);
            let sender = self.sender.clone();
            let wake = make_waker();
            let work = move || {
                let result = panic::catch_unwind(AssertUnwindSafe(|| decoded(&picture)))
                    .unwrap_or_else(|_| {
                        log::error!("decoding a reference image panicked");
                        Err(PictureFailure::Unshowable)
                    });
                if sender.send((digest, result)).is_ok() {
                    wake();
                }
            };
            if let Err(error) = thread::Builder::new()
                .name("picture".to_owned())
                .spawn(work)
            {
                log::error!("could not start a thread to decode a reference image: {error}");
                self.entries
                    .insert(digest, Entry::Failed(PictureFailure::Unshowable));
            }
        }
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok((digest, result)) = self.receiver.try_recv() {
            if !self.entries.contains_key(&digest) {
                continue;
            }
            self.entries.insert(
                digest,
                match result {
                    Ok(texture) => Entry::Ready(texture),
                    Err(reason) => Entry::Failed(reason),
                },
            );
            changed = true;
        }
        if changed {
            self.generation += 1;
        }
        changed
    }
}
