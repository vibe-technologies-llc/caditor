use std::time::{Duration, SystemTime};

use caditor_document::Document;

use crate::{
    binary::{self, History},
    journal::{decode_journal, replay},
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

pub fn save_over(document: &Document, previous: Option<&[u8]>, seconds: u64) -> Option<Vec<u8>> {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(seconds);
    binary::encode_over(document, previous, now, None)
        .ok()
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
