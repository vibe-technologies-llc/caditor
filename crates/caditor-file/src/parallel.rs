#[cfg(test)]
mod tests;

use std::{
    num::NonZeroUsize,
    panic,
    sync::atomic::{AtomicUsize, Ordering},
    thread,
};

use caditor_kernel::{check_interrupt, current_interrupt, interruptible};

pub(crate) fn in_parallel<T: Sync, R: Send>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = thread::available_parallelism()
        .map_or(1, NonZeroUsize::get)
        .min(items.len());
    if threads <= 1 {
        return items.iter().map(&work).collect();
    }
    let next = AtomicUsize::new(0);
    let interrupt = current_interrupt();
    let mut results: Vec<Option<R>> = items.iter().map(|_| None).collect();
    thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                let (next, work, interrupt) = (&next, &work, interrupt.clone());
                scope.spawn(move || {
                    let serve = || {
                        let mut done = Vec::new();
                        loop {
                            let at = next.fetch_add(1, Ordering::Relaxed);
                            let Some(item) = items.get(at) else {
                                break;
                            };
                            if check_interrupt().is_err() {
                                break;
                            }
                            done.push((at, work(item)));
                        }
                        done
                    };
                    match interrupt {
                        Some(interrupt) => interruptible(interrupt, serve),
                        None => serve(),
                    }
                })
            })
            .collect();
        for worker in workers {
            let done = worker
                .join()
                .unwrap_or_else(|panic| panic::resume_unwind(panic));
            for (at, result) in done {
                if let Some(slot) = results.get_mut(at) {
                    *slot = Some(result);
                }
            }
        }
    });
    results.into_iter().map_while(|result| result).collect()
}

pub(crate) fn each_in_parallel<T: Sync, R: Send>(
    items: &[T],
    work: impl Fn(&T) -> R + Sync,
) -> Vec<R> {
    let mut done = in_parallel(items, &work);
    for item in items.get(done.len()..).unwrap_or_default() {
        done.push(work(item));
    }
    done
}
