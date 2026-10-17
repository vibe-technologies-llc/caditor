use caditor_file::Settings;

const ENABLED_KEY: &str = "cloud.enabled";
const ON_HOME_KEY: &str = "cloud.on_home";
const FUSION_CLIENT_KEY: &str = "cloud.fusion.client_id";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudPreferences {
    pub enabled: bool,
    pub on_home: bool,
    pub fusion_client_id: String,
}

impl Default for CloudPreferences {
    fn default() -> Self {
        Self {
            enabled: true,
            on_home: true,
            fusion_client_id: String::new(),
        }
    }
}

impl CloudPreferences {
    pub fn from_settings(raw: &Settings) -> Self {
        Self {
            enabled: raw.flag(ENABLED_KEY).unwrap_or(true),
            on_home: raw.flag(ON_HOME_KEY).unwrap_or(true),
            fusion_client_id: raw
                .text(FUSION_CLIENT_KEY)
                .map(|text| text.trim().to_owned())
                .unwrap_or_default(),
        }
    }

    pub fn write(&self, settings: &mut Settings) {
        settings.set_flag(ENABLED_KEY, self.enabled);
        settings.set_flag(ON_HOME_KEY, self.on_home);
        if self.fusion_client_id.trim().is_empty() {
            settings.remove(FUSION_CLIENT_KEY);
        } else {
            settings.set_text(FUSION_CLIENT_KEY, self.fusion_client_id.trim());
        }
    }

    pub fn shows_on_home(&self) -> bool {
        self.enabled && self.on_home
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_stored_means_cloud_on_and_shown_on_home_with_no_own_app() {
        let preferences = CloudPreferences::from_settings(&Settings::default());

        assert_eq!(preferences, CloudPreferences::default());
        assert!(preferences.enabled && preferences.shows_on_home());
        assert!(preferences.fusion_client_id.is_empty());
    }

    #[test]
    fn the_choices_round_trip_and_an_empty_client_id_is_not_stored() {
        let mut settings = Settings::default();
        let chosen = CloudPreferences {
            enabled: false,
            on_home: false,
            fusion_client_id: "  abc123 ".to_owned(),
        };

        chosen.write(&mut settings);

        assert_eq!(settings.text(FUSION_CLIENT_KEY), Some("abc123"));
        assert_eq!(settings.flag(ENABLED_KEY), Some(false));
        let read = CloudPreferences::from_settings(&settings);
        assert_eq!(read.fusion_client_id, "abc123");
        assert!(!read.enabled && !read.on_home);

        CloudPreferences::default().write(&mut settings);

        assert_eq!(settings.text(FUSION_CLIENT_KEY), None);
    }

    #[test]
    fn turning_cloud_off_hides_it_from_home_whatever_the_home_choice() {
        let off = CloudPreferences {
            enabled: false,
            ..CloudPreferences::default()
        };

        assert!(!off.shows_on_home());
    }
}
