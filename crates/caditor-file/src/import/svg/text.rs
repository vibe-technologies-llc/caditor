use std::borrow::Cow;

const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";
const UTF16_LE_BOM: &[u8] = b"\xff\xfe";
const UTF16_BE_BOM: &[u8] = b"\xfe\xff";
const GZIP_MAGIC: &[u8] = &[0x1f, 0x8b];

pub(super) const LATIN_1_NOTE: &str = "The drawing is not UTF-8 or UTF-16 text, so its names \
                                        were read as Latin-1; letters outside it may look wrong.";

pub(super) fn is_packed(bytes: &[u8]) -> bool {
    bytes.starts_with(GZIP_MAGIC)
}

pub(in crate::import) fn looks_like_svg(bytes: &[u8]) -> bool {
    if is_packed(bytes) || bytes.starts_with(UTF16_LE_BOM) || bytes.starts_with(UTF16_BE_BOM) {
        return true;
    }
    let bytes = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
    bytes
        .iter()
        .find(|byte| !byte.is_ascii_whitespace())
        .is_some_and(|byte| *byte == b'<')
}

pub(super) fn decoded(bytes: &[u8]) -> (Cow<'_, str>, bool) {
    if let Some(rest) = bytes.strip_prefix(UTF8_BOM) {
        return (String::from_utf8_lossy(rest), false);
    }
    if let Some(rest) = bytes.strip_prefix(UTF16_LE_BOM) {
        return (utf16(rest, u16::from_le_bytes).into(), false);
    }
    if let Some(rest) = bytes.strip_prefix(UTF16_BE_BOM) {
        return (utf16(rest, u16::from_be_bytes).into(), false);
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => (text.into(), false),
        Err(_) => (bytes.iter().map(|byte| char::from(*byte)).collect(), true),
    }
}

fn utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> String {
    let units = bytes.as_chunks::<2>().0.iter().map(|pair| unit(*pair));
    char::decode_utf16(units)
        .map(|character| character.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}
