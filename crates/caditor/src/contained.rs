use std::{
    io,
    panic::{self, AssertUnwindSafe},
    sync::Arc,
    thread,
};

use parking_lot::Mutex;

#[derive(Debug)]
pub enum Unfinished {
    Panicked,
    NoThread(io::Error),
}

pub fn spawn<T: Send + 'static>(
    name: &'static str,
    work: impl FnOnce() -> T + Send + 'static,
    failed: impl FnOnce(Unfinished) -> T + Send + 'static,
    finished: impl FnOnce(T) + Send + 'static,
) {
    let slot = Arc::new(Mutex::new(Some((failed, finished))));
    let worker_slot = Arc::clone(&slot);

    let spawned = thread::Builder::new().name(name.to_owned()).spawn(move || {
        let outcome = panic::catch_unwind(AssertUnwindSafe(work));
        let reply = worker_slot.lock().take();

        if let Some((failed, finished)) = reply {
            let result = outcome.unwrap_or_else(|_| {
                log::error!("the {name} thread panicked");
                failed(Unfinished::Panicked)
            });

            deliver(name, finished, result);
        }
    });

    if let Err(error) = spawned {
        log::error!("could not start the {name} thread: {error}");

        let reply = slot.lock().take();

        if let Some((failed, finished)) = reply {
            deliver(name, finished, failed(Unfinished::NoThread(error)));
        }
    }
}

fn deliver<T>(name: &str, finished: impl FnOnce(T), result: T) {
    if panic::catch_unwind(AssertUnwindSafe(|| finished(result))).is_err() {
        log::error!("handing back the result of the {name} thread panicked");
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, time::Duration};

    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    enum Outcome {
        Worked(u32),
        Panicked,
        NoThread,
    }

    fn outcome_of(work: impl FnOnce() -> Outcome + Send + 'static) -> Outcome {
        let (sender, receiver) = mpsc::channel();

        spawn(
            "test",
            work,
            |unfinished| match unfinished {
                Unfinished::Panicked => Outcome::Panicked,
                Unfinished::NoThread(_) => Outcome::NoThread,
            },
            move |outcome| {
                let _ = sender.send(outcome);
            },
        );
        receiver.recv_timeout(Duration::from_secs(10)).unwrap()
    }

    #[test]
    fn a_job_hands_back_what_it_worked_out() {
        assert_eq!(outcome_of(|| Outcome::Worked(7)), Outcome::Worked(7));
    }

    #[test]
    fn a_panicking_job_hands_back_its_failure() {
        assert_eq!(
            outcome_of(|| std::panic::panic_any("boom")),
            Outcome::Panicked
        );
    }
}
