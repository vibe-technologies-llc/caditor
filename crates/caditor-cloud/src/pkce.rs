use graviola::{
    hashing::{Hash, Sha256},
    random,
};

use crate::{encoding::base64_url, error::Error};

const VERIFIER_BYTES: usize = 32;
const STATE_BYTES: usize = 16;
const BOUNDARY_BYTES: usize = 18;

#[derive(Clone, PartialEq, Eq)]
pub struct Verifier(String);

impl Verifier {
    pub fn generate() -> Result<Self, Error> {
        Ok(Self(random_text::<VERIFIER_BYTES>()?))
    }

    pub fn from_text(text: String) -> Self {
        Self(text)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn challenge(&self) -> String {
        base64_url(Sha256::hash(self.0.as_bytes()).as_ref())
    }
}

impl std::fmt::Debug for Verifier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Verifier(..)")
    }
}

pub fn state() -> Result<String, Error> {
    random_text::<STATE_BYTES>()
}

pub fn boundary() -> Result<String, Error> {
    Ok(format!("caditor{}", random_text::<BOUNDARY_BYTES>()?))
}

fn random_text<const BYTES: usize>() -> Result<String, Error> {
    let mut bytes = [0_u8; BYTES];
    random::fill(&mut bytes).map_err(|_| Error::RandomUnavailable)?;
    Ok(base64_url(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_challenge_matches_the_rfc_7636_example() {
        let verifier =
            Verifier::from_text("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".to_owned());

        assert_eq!(
            verifier.challenge(),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn a_generated_verifier_is_long_enough_url_safe_and_new_each_time() {
        let first = Verifier::generate().unwrap();
        let second = Verifier::generate().unwrap();

        assert_eq!(first.as_str().len(), 43);
        assert!(
            first
                .as_str()
                .chars()
                .all(|symbol| symbol.is_ascii_alphanumeric() || symbol == '-' || symbol == '_')
        );
        assert_ne!(first, second);
    }

    #[test]
    fn a_verifier_never_shows_itself_in_debug_output() {
        let verifier = Verifier::from_text("secret-text".to_owned());

        assert!(!format!("{verifier:?}").contains("secret"));
    }

    #[test]
    fn a_state_is_random_and_url_safe() {
        let first = state().unwrap();

        assert_eq!(first.len(), 22);
        assert_ne!(first, state().unwrap());
    }
}
