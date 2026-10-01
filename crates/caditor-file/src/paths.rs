use std::{
    ffi::{OsStr, OsString},
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) const JOURNAL_EXTENSION: &str = "journal";
pub(crate) const MARKER_EXTENSION: &str = "location";
const UNREADABLE_EXTENSION: &str = "unreadable";
const UNREADABLE_ATTEMPTS: u32 = 100;
pub(crate) const SET_ASIDE_KEPT_SECONDS: u64 = 30 * 24 * 60 * 60;
const APPLICATION: &str = "caditor";
const RECOVERY: &str = "recovery";
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;

pub fn state_dir() -> Option<PathBuf> {
    let from_xdg = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute());
    let from_home = || {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .map(|home| home.join(".local").join("state"))
    };
    from_xdg
        .or_else(from_home)
        .map(|base| base.join(APPLICATION))
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
    let cut = match name.to_str() {
        Some(text) => text.floor_char_boundary(room),
        None => room,
    };
    let mut shortened = OsString::from_vec(bytes.get(..cut).unwrap_or_default().to_vec());
    shortened.push(hash);
    shortened
}

fn path_hash(path: &Path) -> u64 {
    path.as_os_str()
        .as_encoded_bytes()
        .iter()
        .fold(FNV_OFFSET, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
        })
}

pub(crate) fn fallback_journal(file: &Path, recovery_dir: &Path) -> PathBuf {
    let hash = path_hash(file);
    recovery_dir.join(format!("file-{hash:016x}.{JOURNAL_EXTENSION}"))
}

pub(crate) fn journal_marker(journal: &Path, recovery_dir: &Path) -> PathBuf {
    let hash = path_hash(journal);
    recovery_dir.join(format!("adjacent-{hash:016x}.{MARKER_EXTENSION}"))
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
