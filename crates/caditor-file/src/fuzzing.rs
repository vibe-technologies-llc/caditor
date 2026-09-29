use crate::{
    binary::{self, History},
    journal::{decode_journal, replay},
    load::{LoadError, Loaded},
};

pub fn reseal(bytes: &[u8]) -> Vec<u8> {
    binary::reseal(bytes)
}

pub fn history(bytes: &[u8]) -> History {
    binary::history(bytes)
}

pub fn load_version(bytes: &[u8], index: usize) -> Result<Loaded, LoadError> {
    binary::load_version(bytes, index)
}

pub fn recover(journal: &[u8]) -> Option<usize> {
    let contents = decode_journal(journal).ok()?;
    let replayed = replay(contents.base, contents.entries);
    Some(replayed.entries.len())
}
