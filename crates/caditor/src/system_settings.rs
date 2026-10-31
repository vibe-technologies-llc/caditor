#[cfg(test)]
use std::cell::Cell;
use std::{
    sync::{
        Once,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use crate::portal::{self, DesktopSettings};

const POLL: Duration = Duration::from_secs(5);

static REDUCED: AtomicBool = AtomicBool::new(false);
static CONTRAST: AtomicBool = AtomicBool::new(false);
static WATCHING: Once = Once::new();

#[cfg(test)]
thread_local! {
    static PRETENDED_CONTRAST: Cell<bool> = const { Cell::new(false) };
}

pub fn reduces_motion() -> bool {
    REDUCED.load(Ordering::Relaxed)
}

#[cfg(not(test))]
pub fn more_contrast() -> bool {
    CONTRAST.load(Ordering::Relaxed)
}

#[cfg(test)]
pub fn more_contrast() -> bool {
    PRETENDED_CONTRAST.with(Cell::get)
}

pub fn watch() {
    if cfg!(test) {
        return;
    }
    WATCHING.call_once(|| {
        let started = thread::Builder::new()
            .name("system settings".to_owned())
            .spawn(follow);
        if let Err(error) = started {
            log::warn!("could not watch the system's accessibility settings ({error})");
        }
    });
}

fn store(settings: DesktopSettings) {
    REDUCED.store(settings.reduces_motion, Ordering::Relaxed);
    CONTRAST.store(settings.more_contrast, Ordering::Relaxed);
}

fn follow() {
    match portal::watch_desktop(store) {
        Ok(()) => log::debug!("the desktop stopped announcing accessibility setting changes"),
        Err(error) => {
            log::debug!("the system's accessibility settings cannot be watched ({error})");
        }
    }
    poll();
}

fn poll() {
    let mut failing = false;
    loop {
        match portal::desktop_settings() {
            Ok(settings) => {
                store(settings);
                failing = false;
            }
            Err(error) => {
                store(DesktopSettings::default());
                if !failing {
                    log::debug!("the system's accessibility settings are unknown ({error})");
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

#[cfg(test)]
pub fn pretend_contrast(more: bool) {
    PRETENDED_CONTRAST.with(|pretended| pretended.set(more));
}
