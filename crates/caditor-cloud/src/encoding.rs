const BASE64_URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const HEX: &[u8; 16] = b"0123456789ABCDEF";

pub fn base64_url(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let [first, second, third] = [0, 1, 2].map(|at| chunk.get(at).copied().unwrap_or(0));
        let group = (u32::from(first) << 16) | (u32::from(second) << 8) | u32::from(third);
        for index in 0..=chunk.len() {
            let sextet = (group >> (18 - 6 * index)) & 0x3f;
            let symbol = usize::try_from(sextet)
                .ok()
                .and_then(|at| BASE64_URL.get(at))
                .copied();
            text.extend(symbol.map(char::from));
        }
    }
    text
}

pub fn percent_encode(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            for nibble in [byte >> 4, byte & 0x0f] {
                let digit = HEX.get(usize::from(nibble)).copied().unwrap_or(b'0');
                encoded.push(char::from(digit));
            }
        }
    }
    encoded
}

pub fn percent_decode(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.bytes();
    while let Some(byte) = rest.next() {
        match byte {
            b'+' => bytes.push(b' '),
            b'%' => {
                let high = rest.next().and_then(hex_value)?;
                let low = rest.next().and_then(hex_value)?;
                bytes.push((high << 4) | low);
            }
            other => bytes.push(other),
        }
    }
    String::from_utf8(bytes).ok()
}

fn hex_value(digit: u8) -> Option<u8> {
    char::from(digit)
        .to_digit(16)
        .and_then(|value| u8::try_from(value).ok())
}

pub fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(name, value)| format!("{}={}", percent_encode(name), percent_encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

pub fn query_pairs(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            Some((percent_decode(name)?, percent_decode(value)?))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_url_matches_the_rfc_4648_vectors_without_padding() {
        assert_eq!(base64_url(b""), "");
        assert_eq!(base64_url(b"f"), "Zg");
        assert_eq!(base64_url(b"fo"), "Zm8");
        assert_eq!(base64_url(b"foo"), "Zm9v");
        assert_eq!(base64_url(b"foob"), "Zm9vYg");
        assert_eq!(base64_url(b"fooba"), "Zm9vYmE");
        assert_eq!(base64_url(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_url_uses_the_url_safe_alphabet() {
        assert_eq!(base64_url(&[0xfb, 0xff, 0xfe]), "-__-");
    }

    #[test]
    fn percent_encoding_keeps_unreserved_characters_and_escapes_the_rest() {
        assert_eq!(percent_encode("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(
            percent_encode("urn:adsk.wipprod:dm.lineage:x y"),
            "urn%3Aadsk.wipprod%3Adm.lineage%3Ax%20y"
        );
        assert_eq!(percent_encode("é"), "%C3%A9");
    }

    #[test]
    fn percent_decoding_reads_escapes_and_plus_and_refuses_broken_ones() {
        assert_eq!(percent_decode("a%20b+c").as_deref(), Some("a b c"));
        assert_eq!(percent_decode("%C3%A9").as_deref(), Some("é"));
        assert_eq!(percent_decode("%zz"), None);
        assert_eq!(percent_decode("%4"), None);
        assert_eq!(percent_decode("%FF"), None);
    }

    #[test]
    fn a_form_escapes_names_and_values() {
        assert_eq!(
            form(&[("scope", "data:read data:write"), ("a&b", "c=d")]),
            "scope=data%3Aread%20data%3Awrite&a%26b=c%3Dd"
        );
    }

    #[test]
    fn query_pairs_decode_and_skip_what_cannot_be_read() {
        assert_eq!(
            query_pairs("code=a%2Fb&state=x&flag&bad=%zz&"),
            vec![
                ("code".to_owned(), "a/b".to_owned()),
                ("state".to_owned(), "x".to_owned()),
                ("flag".to_owned(), String::new()),
            ]
        );
    }
}
