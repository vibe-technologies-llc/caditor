#[cfg(test)]
use std::cell::Cell;

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
