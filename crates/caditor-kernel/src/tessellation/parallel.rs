use std::{
    num::NonZeroUsize,
    panic,
    sync::atomic::{AtomicUsize, Ordering},
    thread,
};

use crate::interrupt;

pub(crate) const FACES_PER_THREAD: usize = 8;

pub(crate) fn available_threads() -> usize {
    thread::available_parallelism().map_or(1, NonZeroUsize::get)
}

pub(crate) fn each_in_order<T: Send>(
    count: usize,
    most_threads: usize,
    least_each: usize,
    make: impl Fn(usize) -> T + Sync,
) -> Vec<T> {
    let threads = most_threads.min(count.div_ceil(least_each.max(1)));
    if threads <= 1 {
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
        let helpers: Vec<_> = (1..threads)
            .filter_map(|_| {
                let interrupt = interrupt::current();
                thread::Builder::new()
                    .name("tessellation".to_owned())
                    .spawn_scoped(scope, move || interrupt::within(interrupt, take))
                    .ok()
            })
            .collect();
        let mut made = take();
        for helper in helpers {
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
