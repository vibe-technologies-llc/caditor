use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

use serde_json::Value;

use crate::{
    read::read_file,
    save::{keep_copy, write_atomically},
};

const APPLICATION: &str = "caditor";
const SETTINGS_FILE: &str = "preferences.json";
const UNREADABLE_STEM: &str = "preferences.unreadable";
const MAX_KEPT: u32 = 100;

pub fn config_dir() -> Option<PathBuf> {
    let from_xdg = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute());
    let from_home = || {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .map(|home| home.join(".config"))
    };
    from_xdg
        .or_else(from_home)
        .map(|base| base.join(APPLICATION))
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    values: BTreeMap<String, Value>,
}

impl Settings {
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(SETTINGS_FILE);
        let Ok(bytes) = read_file(&path) else {
            return Self::default();
        };
        match serde_json::from_slice::<BTreeMap<String, Value>>(&bytes) {
            Ok(values) => Self { values },
            Err(error) => {
                log::warn!(
                    "ignoring unreadable preferences in {}: {error}",
                    path.display()
                );
                Self::default()
            }
        }
    }

    pub fn save(&self, dir: &Path) -> io::Result<()> {
        self.save_changes(dir, &Self::default())
    }

    pub fn save_changes(&self, dir: &Path, since: &Self) -> io::Result<()> {
        let path = dir.join(SETTINGS_FILE);
        let mut values = match read_file(&path) {
            Ok(bytes) => match serde_json::from_slice::<BTreeMap<String, Value>>(&bytes) {
                Ok(values) => values,
                Err(error) => {
                    let kept = keep_unreadable(&path)?;
                    log::warn!(
                        "the preferences in {} were unreadable ({error}) and were kept as {}",
                        path.display(),
                        kept.display()
                    );
                    self.values.clone()
                }
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => self.values.clone(),
            Err(error) => return Err(error),
        };
        for (key, value) in &self.values {
            if since.values.get(key) != Some(value) {
                values.insert(key.clone(), value.clone());
            }
        }
        for key in since.values.keys() {
            if !self.values.contains_key(key) {
                values.remove(key);
            }
        }
        let contents = serde_json::to_vec_pretty(&values).map_err(io::Error::other)?;
        std::fs::create_dir_all(dir)?;
        write_atomically(&path, &contents)
    }

    pub fn text(&self, key: &str) -> Option<&str> {
        self.values.get(key)?.as_str()
    }

    pub fn number(&self, key: &str) -> Option<f64> {
        self.values
            .get(key)?
            .as_f64()
            .filter(|value| value.is_finite())
    }

    pub fn flag(&self, key: &str) -> Option<bool> {
        self.values.get(key)?.as_bool()
    }

    pub fn texts(&self, key: &str) -> Option<Vec<String>> {
        self.values
            .get(key)?
            .as_array()?
            .iter()
            .map(|item| item.as_str().map(str::to_owned))
            .collect()
    }

    pub fn keys_under(&self, prefix: &str) -> Vec<String> {
        self.values
            .keys()
            .filter(|key| key.starts_with(prefix))
            .cloned()
            .collect()
    }

    pub fn set_text(&mut self, key: &str, value: &str) {
        self.values
            .insert(key.to_owned(), Value::String(value.to_owned()));
    }

    pub fn set_number(&mut self, key: &str, value: f64) {
        if let Some(number) = serde_json::Number::from_f64(value) {
            self.values.insert(key.to_owned(), Value::Number(number));
        }
    }

    pub fn set_flag(&mut self, key: &str, value: bool) {
        self.values.insert(key.to_owned(), Value::Bool(value));
    }

    pub fn set_texts(&mut self, key: &str, values: &[String]) {
        let list = values.iter().cloned().map(Value::String).collect();
        self.values.insert(key.to_owned(), Value::Array(list));
    }

    pub fn remove(&mut self, key: &str) {
        self.values.remove(key);
    }
}

fn keep_unreadable(path: &Path) -> io::Result<PathBuf> {
    let candidates = (1..=MAX_KEPT).map(|attempt| {
        path.with_file_name(match attempt {
            1 => format!("{UNREADABLE_STEM}.json"),
            _ => format!("{UNREADABLE_STEM}-{attempt}.json"),
        })
    });
    keep_copy(path, candidates.collect())?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "too many unreadable copies of the preferences already exist",
        )
    })
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn settings_round_trip_and_keep_keys_they_do_not_know() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join(SETTINGS_FILE),
            r#"{"units":"in","future.option":{"nested":[1,2]},"orbit":1.5}"#,
        )
        .unwrap();
        let mut settings = Settings::load(dir.path());
        assert_eq!(settings.text("units"), Some("in"));
        assert_eq!(settings.number("orbit"), Some(1.5));
        assert_eq!(settings.flag("orbit"), None);
        settings.set_flag("invert", true);
        settings.set_texts("keys.save", &["Ctrl+S".to_owned()]);
        settings.save(dir.path()).unwrap();
        let again = Settings::load(dir.path());
        assert_eq!(again, settings);
        assert!(
            again
                .keys_under("future.")
                .contains(&"future.option".to_owned())
        );
        assert_eq!(again.texts("keys.save"), Some(vec!["Ctrl+S".to_owned()]));

        std::fs::write(dir.path().join(SETTINGS_FILE), "not json").unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());
        assert_eq!(
            Settings::load(&dir.path().join("missing")),
            Settings::default()
        );
    }

    #[test]
    fn two_windows_keep_each_others_changes() {
        let dir = TempDir::new().unwrap();
        let mut start = Settings::default();
        start.set_text("units", "mm");
        start.set_flag("invert", false);
        start.save(dir.path()).unwrap();

        let first_loaded = Settings::load(dir.path());
        let second_loaded = Settings::load(dir.path());
        let mut first = first_loaded.clone();
        first.set_text("units", "cm");
        first.save_changes(dir.path(), &first_loaded).unwrap();
        let mut second = second_loaded.clone();
        second.set_flag("invert", true);
        second.remove("units");
        second.set_number("zoom", 2.0);
        second.save_changes(dir.path(), &second_loaded).unwrap();

        let merged = Settings::load(dir.path());
        assert_eq!(merged.text("units"), None);
        assert_eq!(merged.flag("invert"), Some(true));
        assert_eq!(merged.number("zoom"), Some(2.0));

        let mut third = merged.clone();
        third.set_text("units", "m");
        first.save_changes(dir.path(), &first_loaded).unwrap();
        third.save_changes(dir.path(), &merged).unwrap();
        let merged = Settings::load(dir.path());
        assert_eq!(merged.text("units"), Some("m"));
        assert_eq!(merged.number("zoom"), Some(2.0));
    }

    #[test]
    fn unreadable_preferences_are_kept_aside_before_they_are_replaced() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(SETTINGS_FILE), "{ broken").unwrap();
        let loaded = Settings::load(dir.path());
        let mut changed = loaded.clone();
        changed.set_text("units", "cm");
        changed.save_changes(dir.path(), &loaded).unwrap();
        assert_eq!(Settings::load(dir.path()).text("units"), Some("cm"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("preferences.unreadable.json")).unwrap(),
            "{ broken"
        );
    }
}
