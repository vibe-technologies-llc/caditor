use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use thiserror::Error;

use crate::read::read_file;

pub const THEMES_FOLDER: &str = "themes";
pub const THEME_EXTENSION: &str = "json";
pub const MAX_THEME_FILE: u64 = 64 << 10;
pub const MAX_THEME_FILES: usize = 64;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeFile {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub view: Option<String>,
    #[serde(default)]
    pub colours: BTreeMap<String, String>,
}

#[derive(Debug, Error)]
pub enum ThemeFileError {
    #[error("it could not be read: {source}")]
    Unreadable { source: io::Error },
    #[error("it is {bytes} bytes, more than the {} KiB a theme may be", MAX_THEME_FILE >> 10)]
    TooLarge { bytes: u64 },
    #[error("it is not a theme caditor understands: {source}")]
    Malformed { source: serde_json::Error },
}

#[derive(Debug)]
pub struct ReadTheme {
    pub path: PathBuf,
    pub result: Result<ThemeFile, ThemeFileError>,
}

impl ReadTheme {
    pub fn key(&self) -> String {
        self.path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

pub fn themes_folder(config_dir: &Path) -> PathBuf {
    config_dir.join(THEMES_FOLDER)
}

pub fn read_themes(folder: &Path) -> Vec<ReadTheme> {
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            log::warn!("could not list the themes in {}: {error}", folder.display());
            return Vec::new();
        }
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_theme(path))
        .collect();
    paths.sort_by_cached_key(|path| path.file_name().map(OsStr::to_ascii_lowercase));
    paths.truncate(MAX_THEME_FILES);
    paths
        .into_iter()
        .map(|path| ReadTheme {
            result: read_theme(&path),
            path,
        })
        .collect()
}

fn is_theme(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(THEME_EXTENSION))
        && path.is_file()
}

pub fn read_theme(path: &Path) -> Result<ThemeFile, ThemeFileError> {
    let bytes = fs::metadata(path)
        .map_err(|source| ThemeFileError::Unreadable { source })?
        .len();
    if bytes > MAX_THEME_FILE {
        return Err(ThemeFileError::TooLarge { bytes });
    }
    let read = read_file(path).map_err(|source| ThemeFileError::Unreadable { source })?;
    parse_theme(&read)
}

pub fn parse_theme(bytes: &[u8]) -> Result<ThemeFile, ThemeFileError> {
    let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if length > MAX_THEME_FILE {
        return Err(ThemeFileError::TooLarge { bytes: length });
    }
    serde_json::from_slice(bytes).map_err(|source| ThemeFileError::Malformed { source })
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn a_theme_file_names_its_base_view_and_colours() {
        let theme = parse_theme(
            br##"{"name": "Ocean", "base": "dark", "view": "light",
                 "colours": {"panel": "#102030", "accent": "#3366cc"}}"##,
        )
        .unwrap();

        assert_eq!(theme.name.as_deref(), Some("Ocean"));
        assert_eq!(theme.base.as_deref(), Some("dark"));
        assert_eq!(theme.view.as_deref(), Some("light"));
        assert_eq!(
            theme.colours.get("panel").map(String::as_str),
            Some("#102030")
        );
        assert_eq!(parse_theme(b"{}").unwrap(), ThemeFile::default());
    }

    #[test]
    fn damaged_unknown_and_oversized_theme_files_are_refused_with_the_reason() {
        assert!(matches!(
            parse_theme(b"{ not json"),
            Err(ThemeFileError::Malformed { .. })
        ));
        assert!(matches!(
            parse_theme(br#"{"colour": {}}"#),
            Err(ThemeFileError::Malformed { .. })
        ));
        let large = vec![b' '; usize::try_from(MAX_THEME_FILE).unwrap() + 1];
        assert!(matches!(
            parse_theme(&large),
            Err(ThemeFileError::TooLarge { .. })
        ));
    }

    #[test]
    fn the_themes_folder_lists_json_files_by_name_and_a_missing_folder_lists_none() {
        let dir = TempDir::new().unwrap();
        let folder = themes_folder(dir.path());

        assert!(read_themes(&folder).is_empty());

        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("b.json"), br#"{"name": "B"}"#).unwrap();
        fs::write(folder.join("A.JSON"), b"{ broken").unwrap();
        fs::write(folder.join("notes.txt"), b"ignored").unwrap();
        let read = read_themes(&folder);

        assert_eq!(read.len(), 2);
        assert_eq!(read[0].key(), "A");
        assert!(read[0].result.is_err());
        assert_eq!(read[1].file_name(), "b.json");
        assert_eq!(read[1].result.as_ref().unwrap().name.as_deref(), Some("B"));
    }
}
