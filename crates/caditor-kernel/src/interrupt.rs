use std::{cell::Cell, sync::Arc};

use thiserror::Error;

pub type Interrupt = Arc<dyn Fn() -> bool + Send + Sync>;

thread_local! {
    static CURRENT: Cell<Option<Interrupt>> = const { Cell::new(None) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("the operation was cancelled")]
pub struct Interrupted;

pub fn interruptible<T>(interrupt: Interrupt, work: impl FnOnce() -> T) -> T {
    let previous = CURRENT.with(|current| current.replace(Some(interrupt)));
    let _restore = Restore(previous);
    work()
}

pub fn check() -> Result<(), Interrupted> {
    let interrupted = CURRENT.with(|current| {
        let lent = Lent {
            slot: current,
            interrupt: current.take(),
        };
        lent.interrupt
            .as_ref()
            .is_some_and(|interrupted| interrupted())
    });
    if interrupted {
        Err(Interrupted)
    } else {
        Ok(())
    }
}

struct Lent<'a> {
    slot: &'a Cell<Option<Interrupt>>,
    interrupt: Option<Interrupt>,
}

impl Drop for Lent<'_> {
    fn drop(&mut self) {
        self.slot.set(self.interrupt.take());
    }
}

struct Restore(Option<Interrupt>);

impl Drop for Restore {
    fn drop(&mut self) {
        let previous = self.0.take();
        CURRENT.with(|current| current.replace(previous));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    #[test]
    fn checks_see_the_innermost_interrupt_until_its_work_ends() {
        assert_eq!(check(), Ok(()));
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        interruptible(Arc::new(move || flag.load(Ordering::SeqCst)), || {
            assert_eq!(check(), Ok(()));
            stop.store(true, Ordering::SeqCst);
            assert_eq!(check(), Err(Interrupted));
            interruptible(Arc::new(|| false), || assert_eq!(check(), Ok(())));
            assert_eq!(check(), Err(Interrupted));
        });
        assert_eq!(check(), Ok(()));
    }

    #[test]
    fn an_interrupt_may_itself_check_without_seeing_itself() {
        let nested = interruptible(Arc::new(|| check().is_err()), || {
            (check(), interruptible(Arc::new(|| true), check))
        });
        assert_eq!(nested, (Ok(()), Err(Interrupted)));
        assert_eq!(check(), Ok(()));
    }

    #[test]
    fn an_interrupt_that_panics_is_still_in_place_afterwards() {
        let polled = Arc::new(AtomicBool::new(false));
        let seen = Arc::clone(&polled);
        let interrupt: Interrupt = Arc::new(move || {
            assert!(seen.swap(true, Ordering::SeqCst), "the first poll panics");
            true
        });
        interruptible(interrupt, || {
            assert!(std::panic::catch_unwind(check).is_err());
            assert_eq!(check(), Err(Interrupted));
        });
    }
}
