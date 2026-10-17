use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const REFRESH_AHEAD_SECONDS: u64 = 120;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: u64,
}

impl Session {
    pub fn is_expiring(&self, now: u64) -> bool {
        self.expires_at <= now.saturating_add(REFRESH_AHEAD_SECONDS)
    }

    pub fn can_refresh(&self) -> bool {
        self.refresh_token.is_some()
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Session")
            .field("refreshable", &self.can_refresh())
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

pub fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(expires_at: u64) -> Session {
        Session {
            access_token: "access-secret".to_owned(),
            refresh_token: Some("refresh-secret".to_owned()),
            expires_at,
        }
    }

    #[test]
    fn a_session_is_expiring_shortly_before_it_runs_out() {
        assert!(!session(1_000).is_expiring(800));
        assert!(session(1_000).is_expiring(880));
        assert!(session(1_000).is_expiring(2_000));
    }

    #[test]
    fn debug_output_never_holds_a_token() {
        let text = format!("{:?}", session(5));

        assert!(!text.contains("secret"));
        assert!(text.contains("refreshable: true"));
    }

    #[test]
    fn a_session_round_trips_through_json() {
        let stored = serde_json::to_string(&session(7)).unwrap();

        assert_eq!(
            serde_json::from_str::<Session>(&stored).unwrap(),
            session(7)
        );
    }
}
