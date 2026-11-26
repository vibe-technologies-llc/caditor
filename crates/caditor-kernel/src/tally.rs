#[cfg(test)]
use std::cell::Cell;

#[cfg(test)]
thread_local! {
    static UNITS: Cell<usize> = const { Cell::new(0) };
    static MESHED_FACES: Cell<usize> = const { Cell::new(0) };
    static MESHED_LOOPS: Cell<usize> = const { Cell::new(0) };
    static JUDGED_FRAGMENTS: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn add(units: usize) {
    UNITS.with(|tally| tally.set(tally.get().saturating_add(units)));
}

#[cfg(not(test))]
pub(crate) fn add(_units: usize) {}

#[cfg(test)]
pub(crate) fn add_meshed_faces(faces: usize) {
    add(faces);
    MESHED_FACES.with(|tally| tally.set(tally.get().saturating_add(faces)));
}

#[cfg(not(test))]
pub(crate) fn add_meshed_faces(_faces: usize) {}

#[cfg(test)]
pub(crate) fn add_meshed_loops(loops: usize) {
    add(loops);
    MESHED_LOOPS.with(|tally| tally.set(tally.get().saturating_add(loops)));
}

#[cfg(not(test))]
pub(crate) fn add_meshed_loops(_loops: usize) {}

#[cfg(test)]
pub(crate) fn add_judged_fragment() {
    add(1);
    JUDGED_FRAGMENTS.with(|tally| tally.set(tally.get().saturating_add(1)));
}

#[cfg(not(test))]
pub(crate) fn add_judged_fragment() {}

#[cfg(test)]
pub(crate) fn measure<T>(work: impl FnOnce() -> T) -> (T, usize) {
    let before = UNITS.with(Cell::get);
    let result = work();
    (result, UNITS.with(Cell::get) - before)
}

#[cfg(test)]
pub(crate) fn measure_meshed_faces<T>(work: impl FnOnce() -> T) -> (T, usize) {
    let before = MESHED_FACES.with(Cell::get);
    let result = work();
    (result, MESHED_FACES.with(Cell::get) - before)
}

#[cfg(test)]
pub(crate) fn measure_meshed_loops<T>(work: impl FnOnce() -> T) -> (T, usize) {
    let before = MESHED_LOOPS.with(Cell::get);
    let result = work();
    (result, MESHED_LOOPS.with(Cell::get) - before)
}

#[cfg(test)]
pub(crate) fn measure_judged_fragments<T>(work: impl FnOnce() -> T) -> (T, usize) {
    let before = JUDGED_FRAGMENTS.with(Cell::get);
    let result = work();
    (result, JUDGED_FRAGMENTS.with(Cell::get) - before)
}
