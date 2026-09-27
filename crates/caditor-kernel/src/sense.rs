#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Sense {
    Same,
    Reversed,
}

impl Sense {
    pub fn is_same(self) -> bool {
        self == Self::Same
    }

    #[must_use]
    pub fn reversed(self) -> Self {
        match self {
            Self::Same => Self::Reversed,
            Self::Reversed => Self::Same,
        }
    }

    #[must_use]
    pub fn combined(self, other: Self) -> Self {
        if self == other {
            Self::Same
        } else {
            Self::Reversed
        }
    }

    pub fn sign(self) -> f64 {
        match self {
            Self::Same => 1.0,
            Self::Reversed => -1.0,
        }
    }

    pub fn from_sign(value: f64) -> Self {
        if value < 0.0 {
            Self::Reversed
        } else {
            Self::Same
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn senses_combine_like_signs() {
        assert_eq!(Sense::Same.reversed(), Sense::Reversed);
        assert_eq!(Sense::Reversed.combined(Sense::Reversed), Sense::Same);
        assert_eq!(Sense::Same.combined(Sense::Reversed), Sense::Reversed);
        assert_eq!(Sense::Reversed.sign(), -1.0);
        assert_eq!(Sense::from_sign(-0.5), Sense::Reversed);
        assert!(Sense::from_sign(0.0).is_same());
    }
}
