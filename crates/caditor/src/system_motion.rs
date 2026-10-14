use std::{
    sync::{
        Once,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use crate::portal;

const POLL: Duration = Duration::from_secs(5);

static REDUCED: AtomicBool = AtomicBool::new(false);
static WATCHING: Once = Once::new();

pub fn reduces() -> bool {
    REDUCED.load(Ordering::Relaxed)
}

pub fn watch() {
    if cfg!(test) {
        return;
    }
    WATCHING.call_once(|| {
        let started = thread::Builder::new()
            .name("system motion".to_owned())
            .spawn(follow);
        if let Err(error) = started {
            log::warn!("could not watch the system's animation setting ({error})");
        }
    });
}

fn follow() {
    match portal::watch_motion(|reduced| REDUCED.store(reduced, Ordering::Relaxed)) {
        Ok(()) => log::debug!("the desktop stopped announcing animation setting changes"),
        Err(error) => log::debug!("the system's animation setting cannot be watched ({error})"),
    }
    poll();
}

fn poll() {
    let mut failing = false;
    loop {
        match portal::reduces_motion() {
            Ok(reduced) => {
                REDUCED.store(reduced, Ordering::Relaxed);
                failing = false;
            }
            Err(error) => {
                REDUCED.store(false, Ordering::Relaxed);
                if !failing {
                    log::debug!("the system's animation setting is unknown ({error})");
                }
                failing = true;
            }
        }
        thread::sleep(POLL);
    }
}

#[cfg(test)]
pub fn pretend(reduced: bool) {
    REDUCED.store(reduced, Ordering::Relaxed);
}
