use std::time::{Duration, SystemTime};

use caditor_document::Document;

use crate::{
    binary::{self, History, Retention},
    journal::{JournalEntry, JournalHead, Logged, Mirror, decode_journal, encode_journal, replay},
    load::{LoadError, Loaded},
    recent::RecentFiles,
    settings::Settings,
};

pub fn reseal(bytes: &[u8]) -> Vec<u8> {
    binary::reseal(bytes)
}

pub fn history(bytes: &[u8]) -> History {
    binary::history(bytes)
}

pub fn save_over(
    document: &Document,
    previous: Option<&[u8]>,
    seconds: u64,
) -> Result<Vec<u8>, impl std::error::Error> {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(seconds);
    binary::encode_keeping(document, previous, (now, Retention::Standard), None)
        .map(|encoded| encoded.bytes)
}

pub fn load_version(bytes: &[u8], index: usize) -> Result<Loaded, LoadError> {
    binary::load_version(bytes, index)
}

pub fn recover(journal: &[u8]) -> Option<usize> {
    let contents = decode_journal(journal).ok()?;
    let replayed = replay(contents.base, contents.entries);
    Some(replayed.entries.len())
}

pub struct Journal {
    base: Document,
    mirror: Mirror,
    entries: Vec<Logged>,
}

impl Journal {
    pub fn new(base: Document) -> Self {
        Self {
            mirror: Mirror::new(base.clone()),
            base,
            entries: Vec::new(),
        }
    }

    pub fn log(&mut self, entry: JournalEntry) {
        let logged = self.mirror.log(entry);
        self.entries.push(logged);
    }

    pub fn logged(&self) -> usize {
        self.entries.len()
    }

    pub fn encoded(&self) -> Option<Vec<u8>> {
        let head = JournalHead {
            file: None,
            on_disk: None,
            loaded_with_problems: false,
            folded: 0,
        };
        encode_journal(&head, &self.base, &self.entries).ok()
    }
}

pub struct Replayed {
    pub document: Document,
    pub replayed: usize,
    pub stopped_early: bool,
    pub issues: Vec<String>,
    pub unreadable_entries: usize,
}

pub fn replay_journal(journal: &[u8]) -> Option<Replayed> {
    let contents = decode_journal(journal).ok()?;
    let replayed = replay(contents.base, contents.entries);
    Some(Replayed {
        document: replayed.editor.document().clone(),
        replayed: replayed.entries.len(),
        stopped_early: replayed.stopped_early,
        issues: contents.issues,
        unreadable_entries: contents.unreadable_entries,
    })
}

pub fn settings(bytes: &[u8]) -> Option<Settings> {
    Settings::parse(bytes).ok()
}

pub fn stored_settings(settings: &Settings) -> Option<Vec<u8>> {
    settings.stored().ok()
}

pub fn recent_files(bytes: &[u8]) -> RecentFiles {
    RecentFiles::parse(bytes)
}

pub fn stored_recent_files(recent: &RecentFiles) -> Option<Vec<u8>> {
    recent.stored().ok()
}
