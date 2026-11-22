#[cfg(test)]
use std::cell::Cell;
use std::path::Path;
#[cfg(any(test, feature = "test-support"))]
use std::{collections::BTreeMap, path::PathBuf};

#[cfg(any(test, feature = "test-support"))]
use parking_lot::Mutex;

#[cfg(any(test, feature = "test-support"))]
static WHOLE_READS: Mutex<BTreeMap<PathBuf, usize>> = Mutex::new(BTreeMap::new());

#[cfg(test)]
thread_local! {
    static UNITS: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn add(units: usize) {
    UNITS.with(|tally| tally.set(tally.get().saturating_add(units)));
}

#[cfg(not(test))]
pub(crate) fn add(_units: usize) {}

#[cfg(test)]
pub(crate) fn measure<T>(work: impl FnOnce() -> T) -> (T, usize) {
    let before = UNITS.with(Cell::get);
    let result = work();
    (result, UNITS.with(Cell::get) - before)
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn read_whole(path: &Path) {
    let mut reads = WHOLE_READS.lock();

    let count = reads.entry(path.to_path_buf()).or_default();

    *count = count.saturating_add(1);
}

#[cfg(not(any(test, feature = "test-support")))]
pub(crate) fn read_whole(_path: &Path) {}

#[cfg(any(test, feature = "test-support"))]
pub fn disk_check_reads(path: &Path) -> usize {
    WHOLE_READS.lock().get(path).copied().unwrap_or_default()
}
