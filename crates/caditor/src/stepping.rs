const UNITS: [&str; 8] = ["mm", "cm", "m", "um", "µm", "°", "deg", "rad"];
const MOST_DIGITS: usize = 30;
const BIG_STEP: i128 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Empty,
    NotPlain,
    TooLong,
}

impl Refusal {
    pub fn message(self) -> &'static str {
        match self {
            Self::Empty => "Type a number first; Up and Down step a plain number",
            Self::NotPlain => {
                "Up and Down step a plain number, not an expression or a name; type the new value instead"
            }
            Self::TooLong => "That number is too long to step",
        }
    }
}

struct Plain<'a> {
    negative: bool,
    digits: String,
    decimals: usize,
    gap: &'a str,
    unit: &'a str,
}

fn plain(text: &str) -> Result<Plain<'_>, Refusal> {
    let text = text.trim();
    if text.is_empty() {
        return Err(Refusal::Empty);
    }
    let (negative, rest) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let number_end = rest
        .find(|character: char| !(character.is_ascii_digit() || character == '.'))
        .unwrap_or(rest.len());
    let (number, tail) = rest.split_at(number_end);
    let (whole, fraction) = match number.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (number, ""),
    };
    let well_formed = !whole.is_empty()
        && whole.chars().all(|character| character.is_ascii_digit())
        && fraction.chars().all(|character| character.is_ascii_digit())
        && (number.find('.').is_none() || !fraction.is_empty());
    if !well_formed {
        return Err(Refusal::NotPlain);
    }
    let unit = tail.trim_start();
    let gap = tail.strip_suffix(unit).unwrap_or_default();
    if !(unit.is_empty() || UNITS.contains(&unit)) {
        return Err(Refusal::NotPlain);
    }
    if whole.len() + fraction.len() > MOST_DIGITS {
        return Err(Refusal::TooLong);
    }
    Ok(Plain {
        negative,
        digits: format!("{whole}{fraction}"),
        decimals: fraction.len(),
        gap,
        unit,
    })
}

pub fn step(text: &str, direction: Direction, big: bool) -> Result<String, Refusal> {
    let plain = plain(text)?;
    let magnitude: i128 = plain.digits.parse().map_err(|_| Refusal::TooLong)?;
    let value = if plain.negative {
        -magnitude
    } else {
        magnitude
    };
    let size = if big { BIG_STEP } else { 1 };
    let stepped = match direction {
        Direction::Up => value + size,
        Direction::Down => value - size,
    };
    let sign = if stepped < 0 { "-" } else { "" };
    let mut digits = stepped.unsigned_abs().to_string();
    while digits.len() <= plain.decimals {
        digits.insert(0, '0');
    }
    let (whole, fraction) = digits.split_at(digits.len() - plain.decimals);
    let point = if plain.decimals == 0 { "" } else { "." };
    let gap = if plain.unit.is_empty() { "" } else { plain.gap };
    Ok(format!("{sign}{whole}{point}{fraction}{gap}{}", plain.unit))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn up(text: &str) -> Result<String, Refusal> {
        step(text, Direction::Up, false)
    }

    fn down(text: &str) -> Result<String, Refusal> {
        step(text, Direction::Down, false)
    }

    #[test]
    fn a_plain_number_steps_by_its_last_digit() {
        assert_eq!(up("10").unwrap(), "11");
        assert_eq!(up("25.4").unwrap(), "25.5");
        assert_eq!(down("25.40").unwrap(), "25.39");
        assert_eq!(up("0.99").unwrap(), "1.00");
        assert_eq!(step("10", Direction::Up, true).unwrap(), "20");
        assert_eq!(step("2.5", Direction::Down, true).unwrap(), "1.5");
    }

    #[test]
    fn stepping_crosses_zero_and_keeps_the_decimals() {
        assert_eq!(down("0").unwrap(), "-1");
        assert_eq!(down("0.0").unwrap(), "-0.1");
        assert_eq!(down("0.5").unwrap(), "0.4");
        assert_eq!(up("-0.5").unwrap(), "-0.4");
        assert_eq!(up("-1").unwrap(), "0");
        assert_eq!(down("-0.05").unwrap(), "-0.06");
        assert_eq!(up("-0.1").unwrap(), "0.0");
    }

    #[test]
    fn a_unit_is_kept_as_typed() {
        assert_eq!(up("12 mm").unwrap(), "13 mm");
        assert_eq!(up("12mm").unwrap(), "13mm");
        assert_eq!(down("45°").unwrap(), "44°");
        assert_eq!(up("1.5 rad").unwrap(), "1.6 rad");
    }

    #[test]
    fn expressions_names_and_odd_units_are_refused() {
        for text in [
            "width",
            "width = 10",
            "2 * 3",
            "10 + 1",
            "(5)",
            "5 furlongs",
            "1e3",
            ".5",
            "5.",
            "+5",
            "10 mm * 2",
        ] {
            assert_eq!(up(text), Err(Refusal::NotPlain), "{text}");
        }
        assert_eq!(up("  "), Err(Refusal::Empty));
        assert_eq!(up(&"9".repeat(40)), Err(Refusal::TooLong));
    }
}
