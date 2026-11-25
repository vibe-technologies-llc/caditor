use std::{ops::Range, sync::Arc};

use parking_lot::Mutex;

#[derive(Debug, Default)]
pub(crate) struct Spans {
    capacity: u64,
    free: Vec<Range<u64>>,
}

impl Spans {
    pub(crate) fn new(capacity: u64) -> Self {
        let mut spans = Self::default();

        spans.grow(capacity);
        spans
    }

    pub(crate) fn capacity(&self) -> u64 {
        self.capacity
    }

    pub(crate) fn longest_free(&self) -> u64 {
        self.free
            .iter()
            .map(|span| span.end - span.start)
            .max()
            .unwrap_or(0)
    }

    pub(crate) fn take(&mut self, length: u64) -> Option<Range<u64>> {
        if length == 0 {
            return Some(0..0);
        }

        let at = self
            .free
            .iter()
            .position(|span| span.end - span.start >= length)?;
        let span = self.free.get_mut(at)?;
        let taken = span.start..span.start + length;

        span.start += length;
        if span.is_empty() {
            self.free.remove(at);
        }
        Some(taken)
    }

    pub(crate) fn give_back(&mut self, span: Range<u64>) {
        if span.is_empty() || span.end > self.capacity {
            return;
        }

        let at = self.free.partition_point(|free| free.start < span.start);
        let before = at.checked_sub(1).filter(|before| {
            self.free
                .get(*before)
                .is_some_and(|free| free.end == span.start)
        });
        let after = self
            .free
            .get(at)
            .is_some_and(|free| free.start == span.end)
            .then_some(at);

        match (before, after) {
            (Some(before), Some(after)) => {
                let end = self.free.get(after).map_or(span.end, |free| free.end);

                if let Some(free) = self.free.get_mut(before) {
                    free.end = end;
                }
                self.free.remove(after);
            }
            (Some(before), None) => {
                if let Some(free) = self.free.get_mut(before) {
                    free.end = span.end;
                }
            }
            (None, Some(after)) => {
                if let Some(free) = self.free.get_mut(after) {
                    free.start = span.start;
                }
            }
            (None, None) => self.free.insert(at, span),
        }
    }

    pub(crate) fn grow(&mut self, capacity: u64) {
        if capacity > self.capacity {
            let added = self.capacity..capacity;

            self.capacity = capacity;
            self.give_back(added);
        }
    }

    pub(crate) fn is_unused(&self) -> bool {
        match self.free.as_slice() {
            [] => self.capacity == 0,
            [only] => *only == (0..self.capacity),
            _ => false,
        }
    }
}

pub(crate) type SharedSpans = Arc<Mutex<Spans>>;

pub(crate) struct Held {
    owner: SharedSpans,
    span: Range<u64>,
}

impl Held {
    pub(crate) fn take(owner: &SharedSpans, length: u64) -> Option<Self> {
        let span = owner.lock().take(length)?;
        Some(Self {
            owner: Arc::clone(owner),
            span,
        })
    }

    pub(crate) fn start(&self) -> u64 {
        self.span.start
    }

    pub(crate) fn len(&self) -> u64 {
        self.span.end - self.span.start
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        self.owner.lock().give_back(self.span.clone());
    }
}

pub(crate) struct Slab<T, const N: usize> {
    contents: T,
    spans: [SharedSpans; N],
}

impl<T, const N: usize> Slab<T, N> {
    fn take(slab: &Arc<Self>, lengths: [u64; N]) -> Option<SlabSpan<T, N>> {
        let mut held = Vec::with_capacity(N);

        for (owner, length) in slab.spans.iter().zip(lengths) {
            held.push(Held::take(owner, length)?);
        }
        Some(SlabSpan {
            slab: Arc::clone(slab),
            held: held.try_into().ok()?,
        })
    }

    fn capacities(&self) -> [u64; N] {
        std::array::from_fn(|kind| {
            self.spans
                .get(kind)
                .map_or(0, |spans| spans.lock().capacity())
        })
    }

    fn is_unused(&self) -> bool {
        self.spans.iter().all(|spans| spans.lock().is_unused())
    }
}

pub(crate) struct SlabSpan<T, const N: usize> {
    slab: Arc<Slab<T, N>>,
    held: [Held; N],
}

impl<T, const N: usize> SlabSpan<T, N> {
    pub(crate) fn contents(&self) -> &T {
        &self.slab.contents
    }

    pub(crate) fn start(&self, kind: usize) -> u64 {
        self.held.get(kind).map_or(0, Held::start)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SlabSizes<const N: usize> {
    pub first: [u64; N],
    pub most: [u64; N],
}

impl<const N: usize> SlabSizes<N> {
    pub(crate) fn packs(&self, lengths: [u64; N], share: u64) -> bool {
        lengths
            .iter()
            .zip(self.most)
            .all(|(length, most)| *length <= most / share.max(1))
    }

    fn next(&self, last: Option<[u64; N]>, lengths: [u64; N]) -> [u64; N] {
        std::array::from_fn(|kind| {
            let first = self.first.get(kind).copied().unwrap_or(0);
            let most = self.most.get(kind).copied().unwrap_or(0);
            let length = lengths.get(kind).copied().unwrap_or(0);
            let doubled = last
                .and_then(|last| last.get(kind).copied())
                .map_or(first, |last| last.saturating_mul(2));

            doubled.clamp(first.min(most), most).max(length)
        })
    }
}

pub(crate) struct Slabs<T, const N: usize> {
    sizes: SlabSizes<N>,
    slabs: Vec<Arc<Slab<T, N>>>,
}

impl<T, const N: usize> Slabs<T, N> {
    pub(crate) fn new(sizes: SlabSizes<N>) -> Self {
        Self {
            sizes,
            slabs: Vec::new(),
        }
    }

    pub(crate) fn sizes(&self) -> SlabSizes<N> {
        self.sizes
    }

    pub(crate) fn place(
        &mut self,
        lengths: [u64; N],
        make: impl FnOnce([u64; N]) -> T,
    ) -> Option<SlabSpan<T, N>> {
        if let Some(span) = self.slabs.iter().find_map(|slab| Slab::take(slab, lengths)) {
            return Some(span);
        }

        let last = self.slabs.last().map(|slab| slab.capacities());
        let capacities = self.sizes.next(last, lengths);
        let slab = Arc::new(Slab {
            contents: make(capacities),
            spans: capacities.map(|capacity| Arc::new(Mutex::new(Spans::new(capacity)))),
        });
        let span = Slab::take(&slab, lengths);

        self.slabs.push(slab);
        span
    }

    pub(crate) fn release_unused(&mut self) {
        self.slabs
            .retain(|slab| Arc::strong_count(slab) > 1 || !slab.is_unused());
    }

    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.slabs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_are_taken_first_fit_and_given_back_merged_with_their_neighbours() {
        let mut spans = Spans::new(10);

        let first = spans.take(3).unwrap();
        let second = spans.take(3).unwrap();
        let third = spans.take(3).unwrap();
        let too_long = spans.take(2);

        assert_eq!(
            [first.clone(), second.clone(), third.clone()],
            [0..3, 3..6, 6..9]
        );
        assert_eq!(too_long, None);
        assert!(!spans.is_unused());

        spans.give_back(first);
        spans.give_back(third);

        assert_eq!(spans.free, vec![0..3, 6..10]);
        assert_eq!(spans.take(4), Some(6..10));

        spans.give_back(6..10);
        spans.give_back(second);

        assert_eq!(spans.free, vec![0..10]);
        assert!(spans.is_unused());

        spans.grow(16);

        assert_eq!(spans.free, vec![0..16]);
        assert_eq!(spans.take(0), Some(0..0));
    }

    #[test]
    fn a_held_span_goes_back_when_dropped() {
        let owner: SharedSpans = Arc::new(Mutex::new(Spans::new(8)));

        let held = Held::take(&owner, 5).unwrap();
        let refused = Held::take(&owner, 4);

        assert_eq!((held.start(), held.len()), (0, 5));
        assert!(refused.is_none());

        drop(held);

        assert!(owner.lock().is_unused());
    }

    #[test]
    fn slabs_double_up_to_their_largest_size_and_an_unused_one_is_released() {
        let mut slabs: Slabs<[u64; 2], 2> = Slabs::new(SlabSizes {
            first: [4, 8],
            most: [16, 16],
        });

        let small = slabs.place([3, 3], |capacities| capacities).unwrap();
        let next = slabs.place([3, 3], |capacities| capacities).unwrap();
        let large = slabs.place([20, 1], |capacities| capacities).unwrap();

        assert_eq!(*small.contents(), [4, 8]);
        assert_eq!(*next.contents(), [8, 16]);
        assert_eq!(*large.contents(), [20, 16]);
        assert_eq!((next.start(0), next.start(1)), (0, 0));
        assert_eq!(slabs.count(), 3);

        drop(next);
        slabs.release_unused();

        assert_eq!(slabs.count(), 2);

        let again = slabs.place([1, 1], |capacities| capacities).unwrap();

        assert_eq!(*again.contents(), [4, 8]);
        assert_eq!((again.start(0), again.start(1)), (3, 3));
        assert!(
            SlabSizes::<2> {
                first: [4, 8],
                most: [16, 16],
            }
            .packs([4, 4], 4)
        );
    }
}
