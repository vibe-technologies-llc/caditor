use std::{
    num::NonZeroUsize,
    panic,
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

use crate::interrupt;

pub(crate) const FACES_PER_THREAD: usize = 8;

static SPARE_THREADS: OnceLock<AtomicUsize> = OnceLock::new();

pub fn available_threads() -> usize {
    thread::available_parallelism().map_or(1, NonZeroUsize::get)
}

fn spare_threads() -> &'static AtomicUsize {
    SPARE_THREADS.get_or_init(|| AtomicUsize::new(available_threads().saturating_sub(1)))
}

#[derive(Debug)]
pub struct HelperThreads {
    count: usize,
}

impl HelperThreads {
    pub fn take(wanted: usize) -> Self {
        let spare = spare_threads();
        let mut left = spare.load(Ordering::Relaxed);
        loop {
            let count = wanted.min(left);
            if count == 0 {
                return Self { count };
            }
            match spare.compare_exchange_weak(
                left,
                left.saturating_sub(count),
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Self { count },
                Err(now) => left = now,
            }
        }
    }

    pub fn count(&self) -> usize {
        self.count
    }
}

impl Drop for HelperThreads {
    fn drop(&mut self) {
        if self.count > 0 {
            spare_threads().fetch_add(self.count, Ordering::AcqRel);
        }
    }
}

pub(crate) fn both<A, B: Send>(
    threads: usize,
    first: impl FnOnce() -> A,
    second: impl Fn() -> B + Sync,
) -> (A, B) {
    let helpers = HelperThreads::take(threads.saturating_sub(1).min(1));
    if helpers.count() == 0 {
        return (first(), second());
    }
    thread::scope(|scope| {
        let interrupt = interrupt::current();
        let second = &second;
        let helper = thread::Builder::new()
            .name("kernel".to_owned())
            .spawn_scoped(scope, move || interrupt::within(interrupt, second));
        let made = first();
        match helper.map(|helper| helper.join()) {
            Ok(Ok(other)) => (made, other),
            Ok(Err(payload)) => panic::resume_unwind(payload),
            Err(_) => (made, second()),
        }
    })
}

pub(crate) fn each_in_order<T: Send>(
    count: usize,
    most_threads: usize,
    least_each: usize,
    make: impl Fn(usize) -> T + Sync,
) -> Vec<T> {
    let threads = most_threads.min(count.div_ceil(least_each.max(1)));
    let helpers = HelperThreads::take(threads.saturating_sub(1));
    if helpers.count() == 0 {
        return (0..count).map(make).collect();
    }
    let next = AtomicUsize::new(0);
    let take = || {
        let mut made = Vec::new();
        loop {
            let index = next.fetch_add(1, Ordering::Relaxed);
            if index >= count {
                return made;
            }
            made.push((index, make(index)));
        }
    };
    let mut made = thread::scope(|scope| {
        let spawned: Vec<_> = (0..helpers.count())
            .filter_map(|_| {
                let interrupt = interrupt::current();
                thread::Builder::new()
                    .name("kernel".to_owned())
                    .spawn_scoped(scope, move || interrupt::within(interrupt, take))
                    .ok()
            })
            .collect();
        let mut made = take();
        for helper in spawned {
            match helper.join() {
                Ok(more) => made.extend(more),
                Err(payload) => panic::resume_unwind(payload),
            }
        }
        made
    });
    made.sort_unstable_by_key(|(index, _)| *index);
    made.into_iter().map(|(_, made)| made).collect()
}
