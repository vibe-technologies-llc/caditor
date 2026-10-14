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
            .spawn(poll);
        if let Err(error) = started {
            log::warn!("could not watch the system's animation setting ({error})");
        }
    });
}

fn poll() {
    let mut failing = false;
    loop {
        match portal::animations_enabled() {
            Ok(enabled) => {
                REDUCED.store(!enabled, Ordering::Relaxed);
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
