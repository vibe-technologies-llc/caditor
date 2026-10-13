const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const PADDING: u8 = b'=';
const NOT_A_DIGIT: u8 = u8::MAX;

pub(crate) fn encode(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let digit = |index: u32| {
        char::from(
            ALPHABET
                .get(index as usize & 63)
                .copied()
                .unwrap_or(PADDING),
        )
    };
    for group in bytes.chunks(3) {
        let [first, second, third] = [0, 1, 2].map(|at| group.get(at).copied().unwrap_or(0));
        let joined = u32::from(first) << 16 | u32::from(second) << 8 | u32::from(third);
        text.push(digit(joined >> 18));
        text.push(digit(joined >> 12));
        text.push(if group.len() > 1 {
            digit(joined >> 6)
        } else {
            char::from(PADDING)
        });
        text.push(if group.len() > 2 {
            digit(joined)
        } else {
            char::from(PADDING)
        });
    }
    text
}

pub(crate) fn decode(text: &str) -> Option<Vec<u8>> {
    let digits = text.as_bytes();
    if !digits.len().is_multiple_of(4) {
        return None;
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(digits.len() / 4 * 3).ok()?;
    let (groups, _) = digits.as_chunks::<4>();
    let count = groups.len();
    for (index, group) in groups.iter().enumerate() {
        let last = index + 1 == count;
        let padded = group
            .iter()
            .rev()
            .take_while(|digit| **digit == PADDING)
            .count();
        if padded > 2 || (padded > 0 && !last) {
            return None;
        }
        let mut joined = 0_u32;
        for (at, digit) in group.iter().enumerate() {
            let value = if at >= 4 - padded {
                0
            } else {
                value_of(*digit)?
            };
            joined = joined << 6 | u32::from(value);
        }
        let [_, first, second, third] = joined.to_be_bytes();
        bytes.push(first);
        if padded < 2 {
            bytes.push(second);
        }
        if padded < 1 {
            bytes.push(third);
        }
    }
    Some(bytes)
}

fn value_of(digit: u8) -> Option<u8> {
    let value = match digit {
        b'A'..=b'Z' => digit - b'A',
        b'a'..=b'z' => digit - b'a' + 26,
        b'0'..=b'9' => digit - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => NOT_A_DIGIT,
    };
    (value != NOT_A_DIGIT).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};

    #[test]
    fn bytes_read_back_through_their_text_at_every_padding() {
        for length in 0..40_usize {
            let bytes: Vec<u8> = (0..length)
                .map(|index| u8::try_from(index * 37 % 256).unwrap())
                .collect();
            let text = encode(&bytes);

            assert_eq!(decode(&text), Some(bytes));
        }
        assert_eq!(encode(b"Man"), "TWFu");
        assert_eq!(encode(b"Ma"), "TWE=");
        assert_eq!(encode(b"M"), "TQ==");
    }

    #[test]
    fn text_that_is_not_base64_is_refused() {
        assert_eq!(decode("TWF"), None);
        assert_eq!(decode("TW=u"), None);
        assert_eq!(decode("TQ==TWFu"), None);
        assert_eq!(decode("T!Fu"), None);
        assert_eq!(decode("T==="), None);
    }
}
