use std::ops::RangeInclusive;

pub const FIT_GRADES: RangeInclusive<u8> = 5..=11;
pub const MAX_FIT_SIZE: f64 = 500.0;
const MICROMETRES: f64 = 1000.0;
const SIZE_ROUNDING: f64 = 1e6;
const SIZE_STEPS: [f64; 13] = [
    3.0, 6.0, 10.0, 18.0, 30.0, 50.0, 80.0, 120.0, 180.0, 250.0, 315.0, 400.0, 500.0,
];
const STANDARD_GRADES: [[i32; 7]; 13] = [
    [4, 6, 10, 14, 25, 40, 60],
    [5, 8, 12, 18, 30, 48, 75],
    [6, 9, 15, 22, 36, 58, 90],
    [8, 11, 18, 27, 43, 70, 110],
    [9, 13, 21, 33, 52, 84, 130],
    [11, 16, 25, 39, 62, 100, 160],
    [13, 19, 30, 46, 74, 120, 190],
    [15, 22, 35, 54, 87, 140, 220],
    [18, 25, 40, 63, 100, 160, 250],
    [20, 29, 46, 72, 115, 185, 290],
    [23, 32, 52, 81, 130, 210, 320],
    [25, 36, 57, 89, 140, 230, 360],
    [27, 40, 63, 97, 155, 250, 400],
];
const F_UPPER: [i32; 13] = [
    -6, -10, -13, -16, -20, -25, -30, -36, -43, -50, -56, -62, -68,
];
const G_UPPER: [i32; 13] = [-2, -4, -5, -6, -7, -9, -10, -12, -14, -15, -17, -18, -20];
const K_LOWER: [i32; 13] = [0, 1, 1, 1, 2, 2, 2, 3, 3, 4, 4, 4, 5];
const K_RAISED_GRADES: RangeInclusive<u8> = 4..=7;
const N_LOWER: [i32; 13] = [4, 8, 10, 12, 15, 17, 20, 23, 27, 31, 34, 37, 40];
const P_LOWER: [i32; 13] = [6, 12, 15, 18, 22, 26, 32, 37, 43, 50, 56, 62, 68];
const EVEN_JS_GRADES: RangeInclusive<u8> = 7..=11;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum FitLetter {
    HoleH,
    ShaftF,
    ShaftG,
    ShaftH,
    ShaftJs,
    ShaftK,
    ShaftN,
    ShaftP,
}

impl FitLetter {
    pub const ALL: [Self; 8] = [
        Self::HoleH,
        Self::ShaftF,
        Self::ShaftG,
        Self::ShaftH,
        Self::ShaftJs,
        Self::ShaftK,
        Self::ShaftN,
        Self::ShaftP,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Self::HoleH => "H",
            Self::ShaftF => "f",
            Self::ShaftG => "g",
            Self::ShaftH => "h",
            Self::ShaftJs => "js",
            Self::ShaftK => "k",
            Self::ShaftN => "n",
            Self::ShaftP => "p",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::HoleH => "H: a hole from its nominal size up",
            Self::ShaftF => "f: a shaft for a running fit",
            Self::ShaftG => "g: a shaft for a sliding fit",
            Self::ShaftH => "h: a shaft from its nominal size down",
            Self::ShaftJs => "js: a shaft centred on its nominal size",
            Self::ShaftK => "k: a shaft for a transition fit",
            Self::ShaftN => "n: a shaft for a firm transition fit",
            Self::ShaftP => "p: a shaft for a press fit",
        }
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct IsoFit {
    letter: FitLetter,
    grade: u8,
}

fn size_row(size: f64) -> Option<usize> {
    let size = (size * SIZE_ROUNDING).round() / SIZE_ROUNDING;

    if !(size > 0.0 && size <= MAX_FIT_SIZE) {
        return None;
    }
    SIZE_STEPS.iter().position(|step| size <= *step)
}

impl IsoFit {
    pub fn new(letter: FitLetter, grade: u8) -> Option<Self> {
        FIT_GRADES
            .contains(&grade)
            .then_some(Self { letter, grade })
    }

    pub fn letter(self) -> FitLetter {
        self.letter
    }

    pub fn grade(self) -> u8 {
        self.grade
    }

    pub fn code(self) -> String {
        format!("{}{}", self.letter.code(), self.grade)
    }

    pub fn from_code(code: &str) -> Option<Self> {
        let digits = code.find(|character: char| character.is_ascii_digit())?;
        let (letters, grade) = code.split_at(digits);

        let letter = FitLetter::ALL
            .into_iter()
            .find(|letter| letter.code() == letters)?;
        Self::new(letter, grade.parse().ok()?)
    }

    fn standard_grade(self, row: usize) -> Option<i32> {
        let column = usize::from(self.grade.checked_sub(*FIT_GRADES.start())?);

        STANDARD_GRADES.get(row)?.get(column).copied()
    }

    pub fn deviations(self, size: f64) -> Option<(f64, f64)> {
        let row = size_row(size)?;
        let grade = self.standard_grade(row)?;
        let at = |table: &[i32; 13]| table.get(row).copied();
        let lowered = |upper: i32| (upper, upper - grade);
        let raised = |lower: i32| (lower + grade, lower);

        let (upper, lower) = match self.letter {
            FitLetter::HoleH => raised(0),
            FitLetter::ShaftH => lowered(0),
            FitLetter::ShaftF => lowered(at(&F_UPPER)?),
            FitLetter::ShaftG => lowered(at(&G_UPPER)?),
            FitLetter::ShaftJs => {
                let even = if EVEN_JS_GRADES.contains(&self.grade) {
                    grade - grade % 2
                } else {
                    grade
                };
                let half = f64::from(even) / 2.0 / MICROMETRES;

                return Some((half, -half));
            }
            FitLetter::ShaftK if K_RAISED_GRADES.contains(&self.grade) => raised(at(&K_LOWER)?),
            FitLetter::ShaftK => raised(0),
            FitLetter::ShaftN => raised(at(&N_LOWER)?),
            FitLetter::ShaftP => raised(at(&P_LOWER)?),
        };
        Some((
            f64::from(upper) / MICROMETRES,
            f64::from(lower) / MICROMETRES,
        ))
    }
}
