use std::{fs, io, path::Path};

#[cfg(unix)]
pub fn create_dir_all(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    fs::DirBuilder::new()
        .recursive(true)
        .mode(OWNER_ONLY)
        .create(path)
}

#[cfg(not(unix))]
pub fn create_dir_all(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)
}

#[cfg(unix)]
pub fn restrict(folder: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    match fs::metadata(folder) {
        Ok(found) if found.is_dir() && found.permissions().mode() & 0o777 != OWNER_ONLY => {
            fs::set_permissions(folder, fs::Permissions::from_mode(OWNER_ONLY))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(not(unix))]
pub fn restrict(_folder: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
const OWNER_ONLY: u32 = 0o700;

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    mod owner_only {
        use std::os::unix::fs::PermissionsExt;

        use tempfile::TempDir;

        use super::super::*;

        fn mode(path: &Path) -> u32 {
            fs::metadata(path).unwrap().permissions().mode() & 0o777
        }

        #[test]
        fn every_folder_made_is_for_its_owner_alone() {
            let dir = TempDir::new().unwrap();
            let deep = dir.path().join("cloud").join("downloads").join("abc");

            create_dir_all(&deep).unwrap();

            assert_eq!(mode(&deep), 0o700);
            assert_eq!(mode(deep.parent().unwrap()), 0o700);
            assert_eq!(mode(&dir.path().join("cloud")), 0o700);
        }

        #[test]
        fn making_a_folder_that_exists_is_not_an_error() {
            let dir = TempDir::new().unwrap();

            create_dir_all(dir.path()).unwrap();
            create_dir_all(&dir.path().join("a")).unwrap();
            create_dir_all(&dir.path().join("a")).unwrap();
        }

        #[test]
        fn restricting_closes_a_folder_an_older_version_left_open() {
            let dir = TempDir::new().unwrap();
            let folder = dir.path().join("cloud");
            fs::create_dir(&folder).unwrap();
            fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();

            restrict(&folder).unwrap();

            assert_eq!(mode(&folder), 0o700);
        }

        #[test]
        fn restricting_a_missing_folder_does_nothing() {
            let dir = TempDir::new().unwrap();

            restrict(&dir.path().join("cloud")).unwrap();

            assert!(!dir.path().join("cloud").exists());
        }
    }
}
