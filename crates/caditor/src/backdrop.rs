use std::{
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

use caditor_file::{EnvironmentReadError, Settings, read_environment_file};
use caditor_render::{Environment, EnvironmentError};

use crate::model::Waker;

const IMAGE_KEY: &str = "rendering.backdrop_image";

#[derive(Debug, thiserror::Error)]
pub enum BackdropFailure {
    #[error("{0}")]
    Reading(#[from] EnvironmentReadError),
    #[error("{0}")]
    Preparing(#[from] EnvironmentError),
    #[error("reading it stopped unexpectedly")]
    Crashed,
    #[error("a thread to read it could not be started ({0})")]
    NoThread(std::io::Error),
}

type Prepared = (PathBuf, Result<Arc<Environment>, BackdropFailure>);

#[derive(Debug, Clone, Default)]
pub enum Status {
    #[default]
    Plain,
    Loading(PathBuf),
    Ready(PathBuf, Arc<Environment>),
    Failed(PathBuf, Arc<BackdropFailure>),
}

impl Status {
    fn path(&self) -> Option<&Path> {
        match self {
            Self::Plain => None,
            Self::Loading(path) | Self::Ready(path, _) | Self::Failed(path, _) => Some(path),
        }
    }
}

pub struct Backdrop {
    status: Status,
    sender: Sender<Prepared>,
    receiver: Receiver<Prepared>,
}

impl Default for Backdrop {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            status: Status::Plain,
            sender,
            receiver,
        }
    }
}

pub fn read(settings: &Settings) -> Option<PathBuf> {
    settings
        .text(IMAGE_KEY)
        .filter(|text| !text.is_empty())
        .map(PathBuf::from)
}

pub fn write(path: Option<&Path>, settings: &mut Settings) {
    match path.and_then(Path::to_str) {
        Some(text) => settings.set_text(IMAGE_KEY, text),
        None => settings.remove(IMAGE_KEY),
    }
}

fn prepare(path: &Path) -> Result<Arc<Environment>, BackdropFailure> {
    let pixels = read_environment_file(path)?;
    let environment = Environment::new(pixels.width, pixels.height, pixels.rgb)?;
    Ok(Arc::new(environment))
}

impl Backdrop {
    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn environment(&self) -> Option<&Arc<Environment>> {
        match &self.status {
            Status::Ready(_, environment) => Some(environment),
            Status::Plain | Status::Loading(_) | Status::Failed(..) => None,
        }
    }

    pub fn reload(&mut self) {
        self.status = Status::Plain;
    }

    pub fn follow(&mut self, wanted: Option<&Path>, make_waker: &dyn Fn() -> Waker) {
        if self.status.path() == wanted {
            return;
        }
        let Some(path) = wanted else {
            self.status = Status::Plain;
            return;
        };
        self.status = Status::Loading(path.to_path_buf());
        let sender = self.sender.clone();
        let wake = make_waker();
        let reading = path.to_path_buf();
        let work = move || {
            let result = panic::catch_unwind(AssertUnwindSafe(|| prepare(&reading)))
                .unwrap_or_else(|_| {
                    log::error!("preparing the environment picture panicked");
                    Err(BackdropFailure::Crashed)
                });
            if sender.send((reading, result)).is_ok() {
                wake();
            }
        };
        if let Err(error) = thread::Builder::new()
            .name("environment".to_owned())
            .spawn(work)
        {
            log::error!("could not start a thread to read the environment picture: {error}");
            self.status = Status::Failed(
                path.to_path_buf(),
                Arc::new(BackdropFailure::NoThread(error)),
            );
        }
    }

    pub fn poll(&mut self) -> Option<(PathBuf, Arc<BackdropFailure>)> {
        let mut failed = None;
        while let Ok((path, result)) = self.receiver.try_recv() {
            let current = matches!(&self.status, Status::Loading(loading) if *loading == path);
            if !current {
                continue;
            }
            self.status = match result {
                Ok(environment) => Status::Ready(path, environment),
                Err(failure) => {
                    let failure = Arc::new(failure);
                    log::warn!(
                        "the environment picture {} cannot be used: {failure}",
                        path.display()
                    );
                    failed = Some((path.clone(), Arc::clone(&failure)));
                    Status::Failed(path, failure)
                }
            };
        }
        failed
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn no_wake() -> Waker {
        Box::new(|| {})
    }

    fn sky_file(folder: &Path, name: &str, mantissa: [u8; 3]) -> PathBuf {
        let [red, green, blue] = mantissa;
        let mut bytes = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 16 +X 32\n".to_vec();
        for _ in 0..32 * 16 {
            bytes.extend([red, green, blue, 131]);
        }
        let path = folder.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn settled(backdrop: &mut Backdrop) -> Option<(PathBuf, Arc<BackdropFailure>)> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut failed = None;
        while matches!(backdrop.status(), Status::Loading(_)) && Instant::now() < deadline {
            failed = failed.or(backdrop.poll());
            std::thread::sleep(Duration::from_millis(1));
        }
        failed
    }

    #[test]
    fn a_picture_is_read_off_the_calling_thread_and_then_lights_the_view() {
        let folder = tempfile::tempdir().unwrap();
        let path = sky_file(folder.path(), "sky.hdr", [128, 64, 32]);
        let mut backdrop = Backdrop::default();

        backdrop.follow(Some(&path), &no_wake);
        let loading = matches!(backdrop.status(), Status::Loading(_));
        let failed = settled(&mut backdrop);

        assert!(loading);
        assert!(failed.is_none());
        let environment = backdrop.environment().expect("the picture is ready");
        assert_eq!((environment.width(), environment.height()), (32, 16));
        let radiance = environment.radiance_at(0, 3, 3).unwrap();
        assert!((radiance[0] - 4.0).abs() < 0.1, "{radiance:?}");
    }

    #[test]
    fn a_picture_that_cannot_be_used_falls_back_to_the_plain_sky_saying_why() {
        let folder = tempfile::tempdir().unwrap();
        let missing = folder.path().join("gone.hdr");
        let not_a_picture = folder.path().join("notes.hdr");
        std::fs::write(&not_a_picture, b"just text").unwrap();
        let mut backdrop = Backdrop::default();

        backdrop.follow(Some(&missing), &no_wake);
        let first = settled(&mut backdrop).expect("a missing file fails");
        let first_state = backdrop.environment().is_none();
        backdrop.follow(Some(&not_a_picture), &no_wake);
        let second = settled(&mut backdrop).expect("a text file fails");

        assert_eq!(first.0, missing);
        assert!(
            first.1.to_string().contains("no longer exists"),
            "{}",
            first.1
        );
        assert!(first_state);
        assert!(second.1.to_string().contains("Radiance"), "{}", second.1);
        assert!(matches!(backdrop.status(), Status::Failed(..)));
        assert!(backdrop.environment().is_none());
    }

    #[test]
    fn choosing_another_picture_or_none_ignores_the_one_still_loading() {
        let folder = tempfile::tempdir().unwrap();
        let first = sky_file(folder.path(), "first.hdr", [64; 3]);
        let second = sky_file(folder.path(), "second.hdr", [96; 3]);
        let mut backdrop = Backdrop::default();

        backdrop.follow(Some(&first), &no_wake);
        backdrop.follow(Some(&second), &no_wake);
        settled(&mut backdrop);
        let shown = backdrop
            .environment()
            .unwrap()
            .radiance_at(0, 1, 1)
            .unwrap();
        backdrop.follow(None, &no_wake);

        assert!((shown[0] - 3.0).abs() < 0.1, "{shown:?}");
        assert!(matches!(backdrop.status(), Status::Plain));
        assert!(backdrop.environment().is_none());
    }

    #[test]
    fn the_picture_is_remembered_in_the_settings_and_forgotten_with_none() {
        let mut settings = Settings::default();
        let path = Path::new("/skies/studio.hdr");

        write(Some(path), &mut settings);
        let remembered = read(&settings);
        write(None, &mut settings);

        assert_eq!(remembered.as_deref(), Some(path));
        assert_eq!(read(&settings), None);
        assert_eq!(read(&Settings::default()), None);
    }
}
