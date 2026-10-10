use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) const JOURNAL_EXTENSION: &str = "journal";
pub(crate) const MARKER_EXTENSION: &str = "location";
use crate::os;

const UNREADABLE_EXTENSION: &str = "unreadable";
const UNREADABLE_ATTEMPTS: u32 = 100;
pub(crate) const SET_ASIDE_KEPT_SECONDS: u64 = 30 * 24 * 60 * 60;
const APPLICATION: &str = "caditor";
const RECOVERY: &str = "recovery";
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;

pub fn state_dir() -> Option<PathBuf> {
    os::state_base().map(|base| base.join(APPLICATION))
}

pub fn recovery_dir(state_dir: &Path) -> PathBuf {
    state_dir.join(RECOVERY)
}

pub(crate) fn adjacent_journal(file: &Path) -> Option<PathBuf> {
    let name = file
        .file_name()
        .filter(|name| name.len() + ADJACENT_JOURNAL_ROOM <= MAX_NAME_BYTES)?;
    let mut journal = OsString::from(".");
    journal.push(name);
    journal.push(".");
    journal.push(JOURNAL_EXTENSION);
    Some(file.with_file_name(journal))
}

pub(crate) const MAX_NAME_BYTES: usize = 255;
const ADJACENT_JOURNAL_ROOM: usize = 40;

pub(crate) fn fitting(name: &OsStr, limit: usize) -> OsString {
    let bytes = name.as_encoded_bytes();
    if bytes.len() <= limit {
        return name.to_os_string();
    }
    let hash = format!("~{:016x}", path_hash(Path::new(name)));
    let room = limit.saturating_sub(hash.len());
    let mut shortened = os::name_prefix(name, room);
    shortened.push(hash);
    shortened
}

pub(crate) fn path_hash(path: &Path) -> u64 {
    path.as_os_str()
        .as_encoded_bytes()
        .iter()
        .fold(FNV_OFFSET, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
        })
}

pub(crate) fn fallback_journal(file: &Path, recovery_dir: &Path) -> PathBuf {
    let hash = path_hash(&os::journal_identity(file));
    fallback_named(hash, recovery_dir)
}

pub(crate) fn fallback_named(hash: u64, recovery_dir: &Path) -> PathBuf {
    recovery_dir.join(format!("file-{hash:016x}.{JOURNAL_EXTENSION}"))
}

fn fallback_journals(file: &Path, recovery_dir: &Path) -> Vec<PathBuf> {
    distinct_hashes(&os::journal_identity(file), file)
        .into_iter()
        .map(|hash| fallback_named(hash, recovery_dir))
        .collect()
}

fn distinct_hashes(identity: &Path, spelled: &Path) -> Vec<u64> {
    let current = path_hash(identity);
    let as_spelled = path_hash(spelled);
    if current == as_spelled {
        vec![current]
    } else {
        vec![current, as_spelled]
    }
}

pub(crate) fn journal_marker(journal: &Path, recovery_dir: &Path) -> PathBuf {
    let hash = path_hash(&os::journal_identity(journal));
    marker_named(hash, recovery_dir)
}

fn marker_named(hash: u64, recovery_dir: &Path) -> PathBuf {
    recovery_dir.join(format!("adjacent-{hash:016x}.{MARKER_EXTENSION}"))
}

pub(crate) fn journal_markers(journal: &Path, recovery_dir: &Path) -> Vec<PathBuf> {
    distinct_hashes(&os::journal_identity(journal), journal)
        .into_iter()
        .map(|hash| marker_named(hash, recovery_dir))
        .collect()
}

#[cfg(any(windows, test))]
pub(crate) fn lowercase_spelling(path: &Path) -> PathBuf {
    match path.to_str() {
        Some(text) => PathBuf::from(
            text.chars()
                .flat_map(char::to_lowercase)
                .collect::<String>(),
        ),
        None => path.to_path_buf(),
    }
}

pub(crate) fn untitled_journal(recovery_dir: &Path) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    recovery_dir.join(format!(
        "untitled-{nanos}-{}.{JOURNAL_EXTENSION}",
        std::process::id()
    ))
}

pub(crate) fn journals_for(file: &Path, recovery_dir: Option<&Path>) -> Vec<PathBuf> {
    adjacent_journal(file)
        .into_iter()
        .chain(
            recovery_dir
                .into_iter()
                .flat_map(|dir| fallback_journals(file, dir)),
        )
        .collect()
}

pub(crate) fn journal_destinations(file: &Path, recovery_dir: Option<&Path>) -> Vec<PathBuf> {
    adjacent_journal(file)
        .into_iter()
        .chain(recovery_dir.map(|dir| fallback_journal(file, dir)))
        .collect()
}

pub(crate) fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

pub(crate) fn set_aside_at(name: &OsStr) -> Option<u64> {
    let name = name.to_str()?;
    let stamped = name.strip_suffix(UNREADABLE_EXTENSION)?.strip_suffix('.')?;
    let (journal, stamp) = stamped.rsplit_once('.')?;
    journal
        .rsplit_once('.')
        .filter(|(_, extension)| *extension == JOURNAL_EXTENSION)?;
    let seconds = stamp.split_once('-').map_or(stamp, |(seconds, _)| seconds);
    seconds.parse().ok()
}

pub(crate) fn unreadable_journal(journal: &Path) -> PathBuf {
    let seconds = now_seconds();
    let base = journal.file_name().unwrap_or_default();
    let named = |suffix: String| {
        let mut name = base.to_os_string();
        name.push(format!(".{seconds}{suffix}.{UNREADABLE_EXTENSION}"));
        journal.with_file_name(name)
    };
    std::iter::once(String::new())
        .chain((1..UNREADABLE_ATTEMPTS).map(|attempt| format!("-{attempt}")))
        .map(named)
        .find(|candidate| !candidate.exists())
        .unwrap_or_else(|| named(format!("-{UNREADABLE_ATTEMPTS}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spellings_with_one_identity_share_a_fallback_journal_and_the_as_spelled_one_stays_findable()
    {
        let identity = Path::new(r"c:\models\m.caditor");
        let spelled = Path::new(r"C:\Models\M.caditor");
        let recovery = Path::new("recovery");

        let hashes = distinct_hashes(identity, spelled);
        let same = distinct_hashes(spelled, spelled);

        assert_eq!(hashes, vec![path_hash(identity), path_hash(spelled)]);
        assert_eq!(same, vec![path_hash(spelled)]);
        assert_eq!(
            fallback_named(path_hash(spelled), recovery),
            recovery.join(format!("file-{:016x}.journal", path_hash(spelled)))
        );
        assert_eq!(
            marker_named(path_hash(spelled), recovery),
            recovery.join(format!("adjacent-{:016x}.location", path_hash(spelled)))
        );
    }

    #[test]
    fn lowercase_spelling_folds_every_character_and_leaves_the_rest() {
        assert_eq!(
            lowercase_spelling(Path::new(r"C:\Models\ÄBC.caditor")),
            PathBuf::from(r"c:\models\äbc.caditor")
        );
    }

    #[cfg(unix)]
    #[test]
    fn on_unix_the_lookup_names_are_the_primary_names_alone() {
        let file = Path::new("/models/m.caditor");
        let recovery = Path::new("/recovery");

        assert_eq!(
            fallback_journals(file, recovery),
            vec![fallback_journal(file, recovery)]
        );
        assert_eq!(
            journal_markers(file, recovery),
            vec![journal_marker(file, recovery)]
        );
        assert_eq!(
            journals_for(file, Some(recovery)),
            journal_destinations(file, Some(recovery))
        );
    }
}
