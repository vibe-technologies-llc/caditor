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

pub const UNSHOWABLE: &str = "the picture could not be prepared for showing";

type Decoded = (PictureDigest, Result<Arc<PictureTexture>, String>);

#[derive(Debug, Clone)]
enum Entry {
    Decoding,
    Ready(Arc<PictureTexture>),
    Failed(String),
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

pub fn texture_of(pixels: RgbaPixels) -> Result<Arc<PictureTexture>, String> {
    PictureTexture::new(pixels.width, pixels.height, pixels.rgba)
        .map(Arc::new)
        .ok_or_else(|| UNSHOWABLE.to_owned())
}

fn decoded(picture: &Picture) -> Result<Arc<PictureTexture>, String> {
    decode_picture(picture.bytes())
        .map_err(|error: PictureReadError| error.to_string())
        .and_then(texture_of)
}

impl ImageTextures {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn texture(&self, digest: PictureDigest) -> Option<&Arc<PictureTexture>> {
        match self.entries.get(&digest) {
            Some(Entry::Ready(texture)) => Some(texture),
            Some(Entry::Decoding | Entry::Failed(_)) | None => None,
        }
    }

    pub fn failure(&self, digest: PictureDigest) -> Option<&str> {
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
                        Err(UNSHOWABLE.to_owned())
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
                    .insert(digest, Entry::Failed(UNSHOWABLE.to_owned()));
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
