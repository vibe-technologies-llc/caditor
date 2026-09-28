use std::{cell::RefCell, sync::Arc};

use thiserror::Error;

pub type Interrupt = Arc<dyn Fn() -> bool + Send + Sync>;

thread_local! {
    static CURRENT: RefCell<Option<Interrupt>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("the operation was cancelled")]
pub struct Interrupted;

pub fn interruptible<T>(interrupt: Interrupt, work: impl FnOnce() -> T) -> T {
    let previous = CURRENT.with(|current| current.replace(Some(interrupt)));
    let _restore = Restore(previous);
    work()
}

pub(crate) fn check() -> Result<(), Interrupted> {
    let interrupt = CURRENT.with(|current| current.borrow().clone());
    match interrupt {
        Some(interrupted) if interrupted() => Err(Interrupted),
        Some(_) | None => Ok(()),
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
}
