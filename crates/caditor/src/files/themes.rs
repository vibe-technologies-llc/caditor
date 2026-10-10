use super::{Event, Files};
use crate::themes::{self, UserThemes};

impl Files {
    pub fn themes(&self) -> &UserThemes {
        &self.themes
    }

    pub(super) fn list_themes(&mut self) {
        let Some(folder) = self.themes.folder().map(std::path::Path::to_path_buf) else {
            return;
        };
        self.spawn(
            move || {
                let (themes, refused) = themes::sort(caditor_file::read_themes(&folder));
                Event::ThemesListed { themes, refused }
            },
            || Event::ThemesListed {
                themes: Vec::new(),
                refused: Vec::new(),
            },
        );
    }
}
