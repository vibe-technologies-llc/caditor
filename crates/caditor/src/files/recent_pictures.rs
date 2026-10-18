use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

use caditor_file::read_preview;

use super::{Event, Files};
use crate::cloud::Thumbnail;

pub const AT_ONCE: usize = 3;
pub const RECHECK_AFTER: Duration = Duration::from_secs(30);

type Stamp = Option<SystemTime>;

#[derive(Debug)]
pub enum Outcome {
    Unchanged,
    Read {
        stamp: Stamp,
        picture: Option<Arc<Thumbnail>>,
    },
    Failed,
}

#[derive(Debug)]
struct Entry {
    stamp: Stamp,
    picture: Option<Arc<Thumbnail>>,
    checked: Instant,
}

#[derive(Debug, Default)]
pub struct RecentPictures {
    entries: BTreeMap<PathBuf, Entry>,
    reading: BTreeSet<PathBuf>,
    outdated: BTreeSet<PathBuf>,
}

impl RecentPictures {
    pub fn picture(&self, path: &Path) -> Option<&Arc<Thumbnail>> {
        self.entries.get(path)?.picture.as_ref()
    }

    pub fn wants(&self, path: &Path, now: Instant) -> bool {
        !self.reading.contains(path)
            && self
                .entries
                .get(path)
                .is_none_or(|entry| now.saturating_duration_since(entry.checked) >= RECHECK_AFTER)
    }

    pub fn has_room(&self) -> bool {
        self.reading.len() < AT_ONCE
    }

    fn begin(&mut self, path: &Path, listed: &[PathBuf], now: Instant) -> Option<Option<Stamp>> {
        if !self.has_room() || !self.wants(path, now) {
            return None;
        }
        self.entries.retain(|kept, _| listed.contains(kept));
        self.reading.insert(path.to_path_buf());
        Some(self.entries.get(path).map(|entry| entry.stamp))
    }

    fn arrived(&mut self, path: PathBuf, outcome: Outcome, now: Instant) {
        self.reading.remove(&path);
        if self.outdated.remove(&path) {
            return;
        }
        match outcome {
            Outcome::Unchanged => {
                if let Some(entry) = self.entries.get_mut(&path) {
                    entry.checked = now;
                }
            }
            Outcome::Read { stamp, picture } => {
                self.entries.insert(
                    path,
                    Entry {
                        stamp,
                        picture,
                        checked: now,
                    },
                );
            }
            Outcome::Failed => {
                self.entries.insert(
                    path,
                    Entry {
                        stamp: None,
                        picture: None,
                        checked: now,
                    },
                );
            }
        }
    }

    pub fn forget(&mut self, path: &Path) {
        self.entries.remove(path);
        if self.reading.contains(path) {
            self.outdated.insert(path.to_path_buf());
        }
    }
}

pub fn read(path: &Path, known: Option<Stamp>) -> Outcome {
    let stamp = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok();
    if stamp.is_some() && known == Some(stamp) {
        return Outcome::Unchanged;
    }
    let picture = match read_preview(path) {
        Ok(Some(preview)) => Thumbnail::from_bytes(preview.png()).map(Arc::new),
        Ok(None) => None,
        Err(error) => {
            log::debug!("no picture of {} could be read: {error}", path.display());
            None
        }
    };
    Outcome::Read { stamp, picture }
}

impl Files {
    pub fn recent_picture(&self, path: &Path) -> Option<&Arc<Thumbnail>> {
        self.recent_pictures.picture(path)
    }

    pub fn wants_recent_picture(&self, path: &Path) -> bool {
        self.recent_pictures.wants(path, Instant::now())
    }

    pub fn has_room_for_recent_picture(&self) -> bool {
        self.recent_pictures.has_room()
    }

    pub(super) fn load_recent_picture(&mut self, path: PathBuf) {
        let listed = self.recent.paths().to_vec();
        let Some(known) = self.recent_pictures.begin(&path, &listed, Instant::now()) else {
            return;
        };
        let failed = path.clone();
        self.spawn_own(
            "recent picture",
            move || Event::RecentPicture {
                outcome: read(&path, known),
                path,
            },
            move || Event::RecentPicture {
                path: failed,
                outcome: Outcome::Failed,
            },
        );
    }

    pub(super) fn recent_picture_arrived(&mut self, path: PathBuf, outcome: Outcome) {
        self.recent_pictures.arrived(path, outcome, Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use std::{fs::File, time::Duration};

    use caditor_document::Document;
    use caditor_file::{PreviewSlot, SaveOptions, save_with};
    use tempfile::TempDir;

    use super::*;
    use crate::cloud::sample_png;

    fn model_with_picture(path: &Path, side: u32) {
        let (slot, drawing) = PreviewSlot::new();
        drawing.fill(caditor_file::Preview::from_png(sample_png(side)).unwrap());
        let options = SaveOptions {
            preview: Some(&slot),
            ..SaveOptions::default()
        };
        save_with(&Document::default(), path, &options).unwrap();
    }

    fn picture_size(outcome: &Outcome) -> Option<[usize; 2]> {
        match outcome {
            Outcome::Read {
                picture: Some(picture),
                ..
            } => Some(picture.size()),
            _ => None,
        }
    }

    #[test]
    fn a_picture_is_read_once_per_modification_time() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.caditor");
        model_with_picture(&path, 8);

        let first = read(&path, None);
        let Outcome::Read { stamp, .. } = &first else {
            panic!("{first:?}");
        };
        let stamp = *stamp;
        assert_eq!(picture_size(&first), Some([8, 8]));

        assert!(matches!(read(&path, Some(stamp)), Outcome::Unchanged));

        model_with_picture(&path, 12);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::now() + Duration::from_secs(120))
            .unwrap();

        assert_eq!(picture_size(&read(&path, Some(stamp))), Some([12, 12]));
    }

    #[test]
    fn a_model_without_a_picture_or_a_file_that_is_gone_reads_as_none() {
        let dir = TempDir::new().unwrap();
        let plain = dir.path().join("plain.caditor");
        caditor_file::save(&Document::default(), &plain, false).unwrap();

        assert!(matches!(
            read(&plain, None),
            Outcome::Read { picture: None, .. }
        ));
        assert!(matches!(
            read(&dir.path().join("gone.caditor"), None),
            Outcome::Read {
                stamp: None,
                picture: None
            }
        ));
    }

    #[test]
    fn at_most_a_few_are_read_at_once_and_each_path_only_once() {
        let now = Instant::now();
        let listed: Vec<PathBuf> = (0..5)
            .map(|n| PathBuf::from(format!("{n}.caditor")))
            .collect();
        let mut pictures = RecentPictures::default();

        let started: Vec<bool> = listed
            .iter()
            .map(|path| pictures.begin(path, &listed, now).is_some())
            .collect();

        assert_eq!(started, [true, true, true, false, false]);
        assert!(!pictures.has_room());
        assert!(pictures.begin(&listed[0], &listed, now).is_none());

        pictures.arrived(listed[0].clone(), Outcome::Failed, now);

        assert!(pictures.has_room());
        assert!(pictures.begin(&listed[3], &listed, now).is_some());
        assert!(!pictures.wants(&listed[0], now));
    }

    #[test]
    fn a_checked_path_is_asked_again_only_after_a_while_and_with_its_stamp() {
        let now = Instant::now();
        let path = PathBuf::from("a.caditor");
        let listed = [path.clone()];
        let stamp = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(5));
        let mut pictures = RecentPictures::default();
        assert_eq!(pictures.begin(&path, &listed, now), Some(None));
        pictures.arrived(
            path.clone(),
            Outcome::Read {
                stamp,
                picture: None,
            },
            now,
        );

        assert!(!pictures.wants(&path, now + RECHECK_AFTER - Duration::from_secs(1)));
        assert!(pictures.wants(&path, now + RECHECK_AFTER));
        assert_eq!(
            pictures.begin(&path, &listed, now + RECHECK_AFTER),
            Some(Some(stamp))
        );
    }

    #[test]
    fn a_path_saved_while_it_was_being_read_is_read_again_and_not_listed_ones_are_dropped() {
        let now = Instant::now();
        let path = PathBuf::from("a.caditor");
        let other = PathBuf::from("b.caditor");
        let mut pictures = RecentPictures::default();
        pictures.begin(&other, &[other.clone(), path.clone()], now);
        pictures.arrived(other.clone(), Outcome::Failed, now);
        pictures.begin(&path, &[other.clone(), path.clone()], now);

        pictures.forget(&path);
        pictures.arrived(
            path.clone(),
            Outcome::Read {
                stamp: None,
                picture: None,
            },
            now,
        );

        assert!(pictures.wants(&path, now));
        assert!(pictures.picture(&path).is_none());

        pictures.begin(&path, std::slice::from_ref(&path), now);
        assert!(!pictures.entries.contains_key(&other));
    }
}
