use tempfile::TempDir;

use super::*;

fn digest(text: &str) -> blake3::Hash {
    blake3::hash(text.as_bytes())
}

#[test]
fn texts_marked_in_one_session_are_known_in_the_next() {
    let dir = TempDir::new().unwrap();
    let mut first = CheckedTexts::kept_in(dir.path());

    first.mark(digest("bolt"));
    first.persist().unwrap();
    let next = CheckedTexts::kept_in(dir.path());

    assert!(next.contains(&digest("bolt")));
    assert!(!next.contains(&digest("nut")));
}

#[test]
fn two_sessions_keep_what_each_marked() {
    let dir = TempDir::new().unwrap();
    let mut one = CheckedTexts::kept_in(dir.path());
    let mut other = CheckedTexts::kept_in(dir.path());

    one.mark(digest("bolt"));
    other.mark(digest("nut"));
    one.persist().unwrap();
    other.persist().unwrap();
    let next = CheckedTexts::kept_in(dir.path());

    assert!(next.contains(&digest("bolt")));
    assert!(next.contains(&digest("nut")));
    assert!(other.contains(&digest("bolt")));
}

#[test]
fn a_damaged_file_forgets_every_text_rather_than_trusting_any() {
    let dir = TempDir::new().unwrap();
    let mut checked = CheckedTexts::kept_in(dir.path());
    checked.mark(digest("bolt"));
    checked.persist().unwrap();
    let path = dir.path().join(CHECKED_FILE);
    let mut bytes = std::fs::read(&path).unwrap();

    if let Some(byte) = bytes.get_mut(MAGIC.len()) {
        *byte ^= 1;
    }
    std::fs::write(&path, &bytes).unwrap();
    let next = CheckedTexts::kept_in(dir.path());

    assert!(!next.contains(&digest("bolt")));
    assert!(next.order.is_empty());
}

#[test]
fn the_oldest_texts_leave_beyond_the_limit() {
    let mut checked = CheckedTexts::default();

    for index in 0..=MAX_CHECKED {
        checked.mark(digest(&index.to_string()));
    }

    assert_eq!(checked.order.len(), MAX_CHECKED);
    assert!(!checked.contains(&digest("0")));
    assert!(checked.contains(&digest("1")));
    assert!(checked.contains(&digest(&MAX_CHECKED.to_string())));
}

#[test]
fn texts_are_kept_only_in_memory_without_a_folder() {
    let mut checked = CheckedTexts::default();

    checked.mark(digest("bolt"));

    assert_eq!(checked.persist(), Ok(()));
    assert!(checked.contains(&digest("bolt")));
}
