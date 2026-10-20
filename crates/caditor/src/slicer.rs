use std::{
    ffi::OsString,
    fmt, fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    thread,
    time::{Duration, SystemTime},
};

use caditor_file::Settings;

pub const PROGRAM_KEY: &str = "slicer.program";
pub const FOLDER: &str = "slicer";
const KEPT_FOR: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slicer {
    OrcaSlicer,
}

impl Slicer {
    pub const SUPPORTED: Self = Self::OrcaSlicer;

    pub fn name(self) -> &'static str {
        match self {
            Self::OrcaSlicer => "OrcaSlicer",
        }
    }

    fn programs(self) -> &'static [&'static str] {
        match self {
            Self::OrcaSlicer => os::ORCA_PROGRAMS,
        }
    }

    #[cfg(unix)]
    fn flatpak(self) -> &'static str {
        match self {
            Self::OrcaSlicer => "io.github.softfever.OrcaSlicer",
        }
    }

    #[cfg(unix)]
    fn appimage_prefixes(self) -> &'static [&'static str] {
        match self {
            Self::OrcaSlicer => &["orcaslicer", "orca-slicer", "orca_slicer"],
        }
    }

    #[cfg(windows)]
    fn install_folder(self) -> &'static str {
        match self {
            Self::OrcaSlicer => "OrcaSlicer",
        }
    }
}

impl fmt::Display for Slicer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    Program(PathBuf),
    #[cfg(unix)]
    Flatpak {
        flatpak: PathBuf,
        app: &'static str,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installation {
    pub slicer: Slicer,
    pub launch: Launch,
}

impl Installation {
    pub fn program(&self) -> &Path {
        match &self.launch {
            Launch::Program(program) => program,
            #[cfg(unix)]
            Launch::Flatpak { flatpak, .. } => flatpak,
        }
    }

    fn command(&self, file: &Path) -> Command {
        match &self.launch {
            Launch::Program(program) => {
                let mut command = Command::new(program);
                command.arg(file);
                command
            }
            #[cfg(unix)]
            Launch::Flatpak { flatpak, app } => {
                let mut command = Command::new(flatpak);
                command
                    .args(["run", "--file-forwarding", app, "@@"])
                    .arg(file)
                    .arg("@@");
                command
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FindError {
    #[error(
        "{slicer} was not found. Install it, or name its program in Preferences › General › \
         Exports"
    )]
    NotFound { slicer: Slicer },
    #[error(
        "The {slicer} program “{}” named in Preferences › General › Exports does not exist. \
         Name it again there, or empty the field to look for {slicer} where it is usually \
         installed",
        path.display()
    )]
    ChosenMissing { slicer: Slicer, path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PrepareError {
    #[error(transparent)]
    Find(#[from] FindError),
    #[error("caditor has no folder of its own to keep the file for {slicer} in")]
    NoFolder { slicer: Slicer },
    #[error("caditor could not make its folder for the file for {slicer}: {kind}")]
    Folder { slicer: Slicer, kind: io::ErrorKind },
    #[error("caditor ran into an internal error while looking for {slicer}")]
    Crashed { slicer: Slicer },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LaunchError {
    #[error(
        "caditor could not start {slicer} from “{}”, since it is not allowed to run as a \
         program. Allow it to run in its file properties",
        program.display()
    )]
    NotRunnable { slicer: Slicer, program: PathBuf },
    #[error("caditor could not start {slicer} from “{}”: {kind}", program.display())]
    Unavailable {
        slicer: Slicer,
        program: PathBuf,
        kind: io::ErrorKind,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub installation: Installation,
    pub folder: PathBuf,
}

pub type Finder =
    Arc<dyn Fn(Slicer, Option<&Path>) -> Result<Installation, FindError> + Send + Sync>;
pub type Launcher = Arc<dyn Fn(&Installation, &Path) -> Result<(), LaunchError> + Send + Sync>;

pub fn chosen_program(settings: &Settings) -> Option<PathBuf> {
    settings
        .text(PROGRAM_KEY)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(PathBuf::from)
}

pub fn write_chosen_program(program: Option<&Path>, settings: &mut Settings) {
    match program.and_then(Path::to_str).map(str::trim) {
        Some(text) if !text.is_empty() => settings.set_text(PROGRAM_KEY, text),
        _ => settings.remove(PROGRAM_KEY),
    }
}

pub fn find(slicer: Slicer, chosen: Option<&Path>) -> Result<Installation, FindError> {
    if let Some(path) = chosen {
        return if path.is_file() {
            Ok(Installation {
                slicer,
                launch: Launch::Program(path.to_path_buf()),
            })
        } else {
            Err(FindError::ChosenMissing {
                slicer,
                path: path.to_path_buf(),
            })
        };
    }
    on_path(slicer.programs(), std::env::var_os("PATH"))
        .map(Launch::Program)
        .or_else(|| os::installed(slicer))
        .map(|launch| Installation { slicer, launch })
        .ok_or(FindError::NotFound { slicer })
}

fn on_path(programs: &[&str], path: Option<OsString>) -> Option<PathBuf> {
    let path = path?;
    std::env::split_paths(&path)
        .filter(|folder| folder.is_absolute())
        .flat_map(|folder| programs.iter().map(move |program| folder.join(program)))
        .find(|candidate| os::is_program(candidate))
}

pub fn prepare(
    folder: Option<&Path>,
    installation: Result<Installation, FindError>,
) -> Result<Prepared, PrepareError> {
    let installation = installation?;
    let slicer = installation.slicer;
    let folder = folder.ok_or(PrepareError::NoFolder { slicer })?;
    make_folder(folder).map_err(|kind| PrepareError::Folder { slicer, kind })?;
    Ok(Prepared {
        installation,
        folder: folder.to_path_buf(),
    })
}

pub fn make_folder(folder: &Path) -> Result<(), io::ErrorKind> {
    fs::create_dir_all(folder).map_err(|error| error.kind())?;
    remove_old_files(folder, SystemTime::now());
    Ok(())
}

fn remove_old_files(folder: &Path, now: SystemTime) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .ok()
            .filter(fs::Metadata::is_file)
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > KEPT_FOR);
        if old && let Err(error) = fs::remove_file(entry.path()) {
            log::debug!("could not remove an old file for the slicer: {error}");
        }
    }
}

pub fn launch(installation: &Installation, file: &Path) -> Result<(), LaunchError> {
    let slicer = installation.slicer;
    let failed = |error: io::Error| match error.kind() {
        io::ErrorKind::PermissionDenied => LaunchError::NotRunnable {
            slicer,
            program: installation.program().to_path_buf(),
        },
        kind => LaunchError::Unavailable {
            slicer,
            program: installation.program().to_path_buf(),
            kind,
        },
    };
    let mut child = installation
        .command(file)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(failed)?;
    let reaper = thread::Builder::new()
        .name("slicer".to_owned())
        .spawn(move || {
            if let Err(error) = child.wait() {
                log::debug!("the slicer could not be waited for: {error}");
            }
        });
    if let Err(error) = reaper {
        log::debug!("the slicer is left to finish by itself: {error}");
    }
    Ok(())
}

#[cfg(unix)]
mod os {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
    };

    use super::{Launch, Slicer, on_path};

    pub const ORCA_PROGRAMS: &[&str] = &["orca-slicer", "OrcaSlicer", "orcaslicer"];
    const RUNNABLE: u32 = 0o111;
    const APPIMAGE: &str = ".appimage";
    const SYSTEM_FLATPAKS: &str = "/var/lib/flatpak/exports/bin";
    const USER_FLATPAKS: &[&str] = &[".local", "share", "flatpak", "exports", "bin"];
    const APPIMAGE_FOLDERS: &[&[&str]] = &[
        &["Applications"],
        &["AppImages"],
        &[".local", "bin"],
        &["Downloads"],
    ];
    const SYSTEM_APPIMAGE_FOLDER: &str = "/opt";

    pub fn is_program(path: &Path) -> bool {
        fs::metadata(path).is_ok_and(|metadata| {
            metadata.is_file() && metadata.permissions().mode() & RUNNABLE != 0
        })
    }

    pub fn installed(slicer: Slicer) -> Option<Launch> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|home| home.is_absolute());
        flatpak(slicer, home.as_deref()).or_else(|| appimage(slicer, home.as_deref()))
    }

    fn under(home: &Path, parts: &[&str]) -> PathBuf {
        parts
            .iter()
            .fold(home.to_path_buf(), |path, part| path.join(part))
    }

    fn flatpak(slicer: Slicer, home: Option<&Path>) -> Option<Launch> {
        let app = slicer.flatpak();
        let exported = std::iter::once(PathBuf::from(SYSTEM_FLATPAKS))
            .chain(home.map(|home| under(home, USER_FLATPAKS)))
            .any(|folder| folder.join(app).is_file());
        if !exported {
            return None;
        }
        on_path(&["flatpak"], std::env::var_os("PATH"))
            .map(|flatpak| Launch::Flatpak { flatpak, app })
    }

    fn appimage(slicer: Slicer, home: Option<&Path>) -> Option<Launch> {
        let folders: Vec<PathBuf> = home
            .into_iter()
            .flat_map(|home| APPIMAGE_FOLDERS.iter().map(|parts| under(home, parts)))
            .chain(std::iter::once(PathBuf::from(SYSTEM_APPIMAGE_FOLDER)))
            .collect();
        newest_appimage(slicer, &folders).map(Launch::Program)
    }

    pub(super) fn newest_appimage(slicer: Slicer, folders: &[PathBuf]) -> Option<PathBuf> {
        folders
            .iter()
            .filter_map(|folder| fs::read_dir(folder).ok())
            .flat_map(|entries| entries.flatten())
            .filter(|entry| is_appimage_of(slicer, &entry.file_name().to_string_lossy()))
            .map(|entry| entry.path())
            .filter(|path| is_program(path))
            .max_by_key(|path| {
                fs::metadata(path)
                    .and_then(|metadata| metadata.modified())
                    .ok()
            })
    }

    pub(super) fn is_appimage_of(slicer: Slicer, name: &str) -> bool {
        let name = name.to_lowercase();
        name.ends_with(APPIMAGE)
            && slicer
                .appimage_prefixes()
                .iter()
                .any(|prefix| name.starts_with(prefix))
    }
}

#[cfg(windows)]
mod os {
    use std::path::{Path, PathBuf};

    use super::{Launch, Slicer};

    pub const ORCA_PROGRAMS: &[&str] = &["orca-slicer.exe"];
    const PROGRAM_FOLDERS: &[&str] = &["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"];
    const USER_PROGRAMS: &str = "Programs";

    pub fn is_program(path: &Path) -> bool {
        path.is_file()
    }

    pub fn installed(slicer: Slicer) -> Option<Launch> {
        let known = |variable: &str| {
            std::env::var_os(variable)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
        };
        PROGRAM_FOLDERS
            .iter()
            .copied()
            .filter_map(known)
            .chain(known("LOCALAPPDATA").map(|local| local.join(USER_PROGRAMS)))
            .flat_map(|folder| {
                slicer
                    .programs()
                    .iter()
                    .map(move |program| folder.join(slicer.install_folder()).join(program))
            })
            .find(|candidate| is_program(candidate))
            .map(Launch::Program)
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn program(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    #[test]
    fn a_chosen_program_is_used_as_named_and_a_missing_one_is_refused_by_its_path() {
        let dir = TempDir::new().unwrap();
        let chosen = program(dir.path(), "Orca.AppImage");
        let missing = dir.path().join("gone");

        let found = find(Slicer::OrcaSlicer, Some(&chosen));
        let refused = find(Slicer::OrcaSlicer, Some(&missing));

        assert_eq!(
            found,
            Ok(Installation {
                slicer: Slicer::OrcaSlicer,
                launch: Launch::Program(chosen),
            })
        );
        assert_eq!(
            refused,
            Err(FindError::ChosenMissing {
                slicer: Slicer::OrcaSlicer,
                path: missing,
            })
        );
    }

    #[test]
    fn the_program_is_found_in_the_first_folder_on_the_path_holding_it() {
        let dir = TempDir::new().unwrap();
        let (first, second) = (dir.path().join("first"), dir.path().join("second"));
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        let name = Slicer::OrcaSlicer.programs()[0];
        let expected = program(&second, name);

        let search = std::env::join_paths([first.as_path(), second.as_path()]).unwrap();

        assert_eq!(
            on_path(Slicer::OrcaSlicer.programs(), Some(search)),
            Some(expected)
        );
        assert_eq!(
            on_path(Slicer::OrcaSlicer.programs(), Some(first.into_os_string())),
            None
        );
        assert_eq!(on_path(Slicer::OrcaSlicer.programs(), None), None);
    }

    #[cfg(unix)]
    #[test]
    fn an_appimage_is_found_by_its_name_whatever_the_case_and_only_when_runnable() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let appimage = program(dir.path(), "OrcaSlicer_Linux_AppImage_V2.3.0.AppImage");
        let locked = dir.path().join("orca-slicer-old.AppImage");
        fs::write(&locked, b"").unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();
        program(dir.path(), "PrusaSlicer.AppImage");

        assert!(os::is_appimage_of(
            Slicer::OrcaSlicer,
            "orcaslicer.APPIMAGE"
        ));
        assert!(!os::is_appimage_of(Slicer::OrcaSlicer, "OrcaSlicer.zip"));
        assert_eq!(
            os::newest_appimage(Slicer::OrcaSlicer, &[dir.path().to_path_buf()]),
            Some(appimage)
        );
    }

    #[test]
    fn preparing_makes_the_folder_and_removes_only_files_older_than_a_week() {
        let dir = TempDir::new().unwrap();
        let folder = dir.path().join("state").join(FOLDER);
        let installation = Installation {
            slicer: Slicer::OrcaSlicer,
            launch: Launch::Program(dir.path().join("orca")),
        };

        let prepared = prepare(Some(&folder), Ok(installation.clone())).unwrap();
        let kept = folder.join("plate.3mf");
        fs::write(&kept, b"3mf").unwrap();
        remove_old_files(&folder, SystemTime::now() + KEPT_FOR / 2);

        assert_eq!(prepared.folder, folder);
        assert!(kept.exists());

        remove_old_files(&folder, SystemTime::now() + KEPT_FOR * 2);

        assert!(!kept.exists());
        assert_eq!(
            prepare(None, Ok(installation)),
            Err(PrepareError::NoFolder {
                slicer: Slicer::OrcaSlicer
            })
        );
    }

    #[test]
    fn the_chosen_program_round_trips_through_the_settings_and_blank_is_none() {
        let mut settings = Settings::default();

        write_chosen_program(Some(Path::new(" /opt/orca/orca-slicer ")), &mut settings);

        assert_eq!(
            chosen_program(&settings),
            Some(PathBuf::from("/opt/orca/orca-slicer"))
        );

        write_chosen_program(Some(Path::new("  ")), &mut settings);

        assert_eq!(chosen_program(&settings), None);
        assert_eq!(settings.text(PROGRAM_KEY), None);
    }

    #[test]
    fn a_program_that_cannot_start_is_refused_naming_it() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("orca-slicer");
        let installation = Installation {
            slicer: Slicer::OrcaSlicer,
            launch: Launch::Program(missing.clone()),
        };

        assert_eq!(
            launch(&installation, &dir.path().join("plate.3mf")),
            Err(LaunchError::Unavailable {
                slicer: Slicer::OrcaSlicer,
                program: missing,
                kind: io::ErrorKind::NotFound,
            })
        );
    }
}
