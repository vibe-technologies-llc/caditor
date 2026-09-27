use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) const JOURNAL_EXTENSION: &str = "journal";
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
    let name = file.file_name()?;
    let mut journal = OsString::from(".");
    journal.push(name);
    journal.push(".");
    journal.push(JOURNAL_EXTENSION);
    Some(file.with_file_name(journal))
}

pub(crate) fn fallback_journal(file: &Path, recovery_dir: &Path) -> PathBuf {
    let hash = file
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .fold(FNV_OFFSET, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
        });
    recovery_dir.join(format!("file-{hash:016x}.{JOURNAL_EXTENSION}"))
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
