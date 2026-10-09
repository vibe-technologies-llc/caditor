use crate::hole_standard::{HoleStandard, pitch_text};

const INCH: f64 = 25.4;
const METRIC_INTERNAL_MINOR: f64 = 1.082_532;
const METRIC_EXTERNAL_MINOR: f64 = 1.226_869;
const WHITWORTH_MINOR: f64 = 1.280_654;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ThreadFamily {
    MetricCoarse,
    MetricFine,
    ParallelPipe,
    TaperPipe,
    Trapezoidal,
}

impl ThreadFamily {
    pub const ALL: [Self; 5] = [
        Self::MetricCoarse,
        Self::MetricFine,
        Self::ParallelPipe,
        Self::TaperPipe,
        Self::Trapezoidal,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::MetricCoarse => "metric_coarse",
            Self::MetricFine => "metric_fine",
            Self::ParallelPipe => "iso_228",
            Self::TaperPipe => "iso_7",
            Self::Trapezoidal => "trapezoidal",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|family| family.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::MetricCoarse => "ISO metric coarse",
            Self::MetricFine => "ISO metric fine",
            Self::ParallelPipe => "Pipe, parallel (ISO 228)",
            Self::TaperPipe => "Pipe, taper (ISO 7)",
            Self::Trapezoidal => "Trapezoidal (ISO 2901)",
        }
    }

    pub fn sizes(self) -> Vec<ThreadSize> {
        match self {
            Self::MetricCoarse => METRIC
                .iter()
                .map(|row| ThreadSize {
                    family: self,
                    name: row.name,
                    major: row.major,
                    pitch: row.coarse,
                })
                .collect(),
            Self::MetricFine => METRIC
                .iter()
                .flat_map(|row| {
                    row.fine.iter().map(|pitch| ThreadSize {
                        family: self,
                        name: row.name,
                        major: row.major,
                        pitch: *pitch,
                    })
                })
                .collect(),
            Self::ParallelPipe => pipe_sizes(self, &PARALLEL_PIPE),
            Self::TaperPipe => pipe_sizes(self, &TAPER_PIPE),
            Self::Trapezoidal => TRAPEZOIDAL
                .iter()
                .map(|(name, major, pitch)| ThreadSize {
                    family: self,
                    name,
                    major: *major,
                    pitch: *pitch,
                })
                .collect(),
        }
    }

    pub fn classes(self, side: ThreadSide) -> &'static [ThreadClass] {
        match (self, side) {
            (Self::MetricCoarse | Self::MetricFine, ThreadSide::Internal) => &METRIC_INTERNAL,
            (Self::MetricCoarse | Self::MetricFine, ThreadSide::External) => &METRIC_EXTERNAL,
            (Self::ParallelPipe, ThreadSide::Internal) => &PARALLEL_PIPE_INTERNAL,
            (Self::ParallelPipe, ThreadSide::External) => &PARALLEL_PIPE_EXTERNAL,
            (Self::TaperPipe, ThreadSide::Internal) => &TAPER_PIPE_INTERNAL,
            (Self::TaperPipe, ThreadSide::External) => &TAPER_PIPE_EXTERNAL,
            (Self::Trapezoidal, ThreadSide::Internal) => &TRAPEZOIDAL_INTERNAL,
            (Self::Trapezoidal, ThreadSide::External) => &TRAPEZOIDAL_EXTERNAL,
        }
    }

    pub fn default_class(self, side: ThreadSide) -> ThreadClass {
        self.classes(side)
            .first()
            .copied()
            .unwrap_or(ThreadClass::ONLY)
    }

    pub fn class_from_id(self, id: &str) -> Option<ThreadClass> {
        [ThreadSide::Internal, ThreadSide::External]
            .into_iter()
            .flat_map(|side| self.classes(side).iter().copied())
            .find(|class| class.id() == id)
    }

    pub fn nearest(self, diameter: f64, side: ThreadSide) -> ThreadSize {
        let gap = |size: &ThreadSize| (size.fitting_diameter(side) - diameter).abs();
        self.sizes()
            .into_iter()
            .min_by(|first, second| gap(first).total_cmp(&gap(second)))
            .unwrap_or(ThreadSize::M8)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ThreadSide {
    Internal,
    External,
}

impl ThreadSide {
    pub fn label(self) -> &'static str {
        match self {
            Self::Internal => "Internal",
            Self::External => "External",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ThreadHand {
    #[default]
    Right,
    Left,
}

impl ThreadHand {
    pub const ALL: [Self; 2] = [Self::Right, Self::Left];

    pub fn label(self) -> &'static str {
        match self {
            Self::Right => "Right",
            Self::Left => "Left",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ThreadClass(&'static str);

impl ThreadClass {
    pub const ONLY: Self = Self("");

    pub fn id(self) -> &'static str {
        self.0
    }

    pub fn label(self) -> &'static str {
        match self.0 {
            "" => "The one class of the standard",
            "Rc" => "Rc, taper",
            "Rp" => "Rp, parallel",
            "R" => "R, taper",
            class => class,
        }
    }
}

const METRIC_INTERNAL: [ThreadClass; 5] = [
    ThreadClass("6H"),
    ThreadClass("5H"),
    ThreadClass("7H"),
    ThreadClass("4H"),
    ThreadClass("6G"),
];
const METRIC_EXTERNAL: [ThreadClass; 6] = [
    ThreadClass("6g"),
    ThreadClass("6h"),
    ThreadClass("4h"),
    ThreadClass("8g"),
    ThreadClass("6e"),
    ThreadClass("6f"),
];
const PARALLEL_PIPE_INTERNAL: [ThreadClass; 1] = [ThreadClass::ONLY];
const PARALLEL_PIPE_EXTERNAL: [ThreadClass; 2] = [ThreadClass("A"), ThreadClass("B")];
const TAPER_PIPE_INTERNAL: [ThreadClass; 2] = [ThreadClass("Rc"), ThreadClass("Rp")];
const TAPER_PIPE_EXTERNAL: [ThreadClass; 1] = [ThreadClass("R")];
const TRAPEZOIDAL_INTERNAL: [ThreadClass; 2] = [ThreadClass("7H"), ThreadClass("8H")];
const TRAPEZOIDAL_EXTERNAL: [ThreadClass; 4] = [
    ThreadClass("7e"),
    ThreadClass("8e"),
    ThreadClass("7c"),
    ThreadClass("8c"),
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThreadSize {
    family: ThreadFamily,
    name: &'static str,
    major: f64,
    pitch: f64,
}

impl ThreadSize {
    pub const M8: Self = Self {
        family: ThreadFamily::MetricCoarse,
        name: "M8",
        major: 8.0,
        pitch: 1.25,
    };

    pub fn from_id(family: ThreadFamily, id: &str) -> Option<Self> {
        family.sizes().into_iter().find(|size| size.id() == id)
    }

    pub fn of_hole(standard: HoleStandard) -> Option<Self> {
        let pitch = standard.thread_pitch()?;
        let major = standard.size.major_diameter();
        [ThreadFamily::MetricCoarse, ThreadFamily::MetricFine]
            .into_iter()
            .flat_map(ThreadFamily::sizes)
            .find(|size| size.major == major && size.pitch == pitch)
    }

    pub fn family(self) -> ThreadFamily {
        self.family
    }

    pub fn pitch(self) -> f64 {
        self.pitch
    }

    pub fn id(self) -> String {
        match self.family {
            ThreadFamily::MetricFine => format!("{}x{}", self.name, pitch_text(self.pitch)),
            ThreadFamily::MetricCoarse
            | ThreadFamily::ParallelPipe
            | ThreadFamily::TaperPipe
            | ThreadFamily::Trapezoidal => self.name.to_owned(),
        }
    }

    pub fn label(self) -> String {
        match self.family {
            ThreadFamily::ParallelPipe => format!("G {}", self.name),
            ThreadFamily::TaperPipe => format!("R {}", self.name),
            ThreadFamily::MetricCoarse | ThreadFamily::MetricFine | ThreadFamily::Trapezoidal => {
                self.id()
            }
        }
    }

    pub fn major_diameter(self, side: ThreadSide) -> f64 {
        match (self.family, side) {
            (ThreadFamily::Trapezoidal, ThreadSide::Internal) => {
                self.major + 2.0 * trapezoidal_clearance(self.pitch)
            }
            _ => self.major,
        }
    }

    pub fn minor_diameter(self, side: ThreadSide) -> f64 {
        let pitch = self.pitch;
        match (self.family, side) {
            (ThreadFamily::MetricCoarse | ThreadFamily::MetricFine, ThreadSide::Internal) => {
                self.major - METRIC_INTERNAL_MINOR * pitch
            }
            (ThreadFamily::MetricCoarse | ThreadFamily::MetricFine, ThreadSide::External) => {
                self.major - METRIC_EXTERNAL_MINOR * pitch
            }
            (ThreadFamily::ParallelPipe | ThreadFamily::TaperPipe, _) => {
                self.major - WHITWORTH_MINOR * pitch
            }
            (ThreadFamily::Trapezoidal, ThreadSide::Internal) => self.major - pitch,
            (ThreadFamily::Trapezoidal, ThreadSide::External) => {
                self.major - pitch - 2.0 * trapezoidal_clearance(pitch)
            }
        }
    }

    pub fn fitting_diameter(self, side: ThreadSide) -> f64 {
        match side {
            ThreadSide::Internal => self.minor_diameter(side),
            ThreadSide::External => self.major_diameter(side),
        }
    }

    pub fn drawn_diameter(self, side: ThreadSide) -> f64 {
        match side {
            ThreadSide::Internal => self.major_diameter(side),
            ThreadSide::External => self.minor_diameter(side),
        }
    }

    pub fn fits(self, diameter: f64, side: ThreadSide) -> bool {
        let pitch = self.pitch;
        match side {
            ThreadSide::Internal => {
                diameter > self.minor_diameter(side) - pitch && diameter < self.major_diameter(side)
            }
            ThreadSide::External => {
                diameter > self.minor_diameter(side) && diameter < self.major_diameter(side) + pitch
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThreadDesignation {
    pub size: ThreadSize,
    pub class: ThreadClass,
    pub hand: ThreadHand,
}

impl ThreadDesignation {
    pub fn text(&self) -> String {
        let size = self.size;
        let class = self.class.id();
        let left = self.hand == ThreadHand::Left;
        match size.family {
            ThreadFamily::MetricCoarse | ThreadFamily::MetricFine => {
                let hand = if left { "-LH" } else { "" };
                format!("{}-{class}{hand}", size.id())
            }
            ThreadFamily::ParallelPipe => {
                let hand = if left { "-LH" } else { "" };
                match class {
                    "" => format!("G {}{hand}", size.name),
                    class => format!("G {} {class}{hand}", size.name),
                }
            }
            ThreadFamily::TaperPipe => {
                let hand = if left { " LH" } else { "" };
                format!("{class} {}{hand}", size.name)
            }
            ThreadFamily::Trapezoidal => {
                let hand = if left { " LH" } else { "" };
                format!("{}{hand}-{class}", size.name)
            }
        }
    }
}

struct MetricRow {
    name: &'static str,
    major: f64,
    coarse: f64,
    fine: &'static [f64],
}

const fn metric(name: &'static str, major: f64, coarse: f64, fine: &'static [f64]) -> MetricRow {
    MetricRow {
        name,
        major,
        coarse,
        fine,
    }
}

const METRIC: [MetricRow; 28] = [
    metric("M1.6", 1.6, 0.35, &[0.2]),
    metric("M2", 2.0, 0.4, &[0.25]),
    metric("M2.5", 2.5, 0.45, &[0.35]),
    metric("M3", 3.0, 0.5, &[0.35]),
    metric("M4", 4.0, 0.7, &[0.5]),
    metric("M5", 5.0, 0.8, &[0.5]),
    metric("M6", 6.0, 1.0, &[0.75]),
    metric("M8", 8.0, 1.25, &[1.0, 0.75]),
    metric("M10", 10.0, 1.5, &[1.25, 1.0, 0.75]),
    metric("M12", 12.0, 1.75, &[1.5, 1.25, 1.0]),
    metric("M14", 14.0, 2.0, &[1.5, 1.25, 1.0]),
    metric("M16", 16.0, 2.0, &[1.5, 1.0]),
    metric("M18", 18.0, 2.5, &[2.0, 1.5, 1.0]),
    metric("M20", 20.0, 2.5, &[2.0, 1.5, 1.0]),
    metric("M22", 22.0, 2.5, &[2.0, 1.5, 1.0]),
    metric("M24", 24.0, 3.0, &[2.0, 1.5, 1.0]),
    metric("M27", 27.0, 3.0, &[2.0, 1.5, 1.0]),
    metric("M30", 30.0, 3.5, &[3.0, 2.0, 1.5, 1.0]),
    metric("M33", 33.0, 3.5, &[3.0, 2.0, 1.5]),
    metric("M36", 36.0, 4.0, &[3.0, 2.0, 1.5]),
    metric("M39", 39.0, 4.0, &[3.0, 2.0, 1.5]),
    metric("M42", 42.0, 4.5, &[4.0, 3.0, 2.0, 1.5]),
    metric("M45", 45.0, 4.5, &[4.0, 3.0, 2.0, 1.5]),
    metric("M48", 48.0, 5.0, &[4.0, 3.0, 2.0, 1.5]),
    metric("M52", 52.0, 5.0, &[4.0, 3.0, 2.0, 1.5]),
    metric("M56", 56.0, 5.5, &[4.0, 3.0, 2.0, 1.5]),
    metric("M60", 60.0, 5.5, &[4.0, 3.0, 2.0, 1.5]),
    metric("M64", 64.0, 6.0, &[4.0, 3.0, 2.0, 1.5]),
];

const PARALLEL_PIPE: [(&str, f64, f64); 24] = [
    ("1/16", 7.723, 28.0),
    ("1/8", 9.728, 28.0),
    ("1/4", 13.157, 19.0),
    ("3/8", 16.662, 19.0),
    ("1/2", 20.955, 14.0),
    ("5/8", 22.911, 14.0),
    ("3/4", 26.441, 14.0),
    ("7/8", 30.201, 14.0),
    ("1", 33.249, 11.0),
    ("1 1/8", 37.897, 11.0),
    ("1 1/4", 41.910, 11.0),
    ("1 1/2", 47.803, 11.0),
    ("1 3/4", 53.746, 11.0),
    ("2", 59.614, 11.0),
    ("2 1/4", 65.710, 11.0),
    ("2 1/2", 75.184, 11.0),
    ("2 3/4", 81.534, 11.0),
    ("3", 87.884, 11.0),
    ("3 1/2", 100.330, 11.0),
    ("4", 113.030, 11.0),
    ("4 1/2", 125.730, 11.0),
    ("5", 138.430, 11.0),
    ("5 1/2", 151.130, 11.0),
    ("6", 163.830, 11.0),
];

const TAPER_PIPE: [(&str, f64, f64); 15] = [
    ("1/16", 7.723, 28.0),
    ("1/8", 9.728, 28.0),
    ("1/4", 13.157, 19.0),
    ("3/8", 16.662, 19.0),
    ("1/2", 20.955, 14.0),
    ("3/4", 26.441, 14.0),
    ("1", 33.249, 11.0),
    ("1 1/4", 41.910, 11.0),
    ("1 1/2", 47.803, 11.0),
    ("2", 59.614, 11.0),
    ("2 1/2", 75.184, 11.0),
    ("3", 87.884, 11.0),
    ("4", 113.030, 11.0),
    ("5", 138.430, 11.0),
    ("6", 163.830, 11.0),
];

const TRAPEZOIDAL: [(&str, f64, f64); 23] = [
    ("Tr 8x1.5", 8.0, 1.5),
    ("Tr 10x2", 10.0, 2.0),
    ("Tr 12x3", 12.0, 3.0),
    ("Tr 14x3", 14.0, 3.0),
    ("Tr 16x4", 16.0, 4.0),
    ("Tr 18x4", 18.0, 4.0),
    ("Tr 20x4", 20.0, 4.0),
    ("Tr 22x5", 22.0, 5.0),
    ("Tr 24x5", 24.0, 5.0),
    ("Tr 26x5", 26.0, 5.0),
    ("Tr 28x5", 28.0, 5.0),
    ("Tr 30x6", 30.0, 6.0),
    ("Tr 32x6", 32.0, 6.0),
    ("Tr 36x6", 36.0, 6.0),
    ("Tr 40x7", 40.0, 7.0),
    ("Tr 44x7", 44.0, 7.0),
    ("Tr 48x8", 48.0, 8.0),
    ("Tr 52x8", 52.0, 8.0),
    ("Tr 60x9", 60.0, 9.0),
    ("Tr 70x10", 70.0, 10.0),
    ("Tr 80x10", 80.0, 10.0),
    ("Tr 90x12", 90.0, 12.0),
    ("Tr 100x12", 100.0, 12.0),
];

fn pipe_sizes(family: ThreadFamily, table: &[(&'static str, f64, f64)]) -> Vec<ThreadSize> {
    table
        .iter()
        .map(|(name, major, per_inch)| ThreadSize {
            family,
            name,
            major: *major,
            pitch: INCH / per_inch,
        })
        .collect()
}

fn trapezoidal_clearance(pitch: f64) -> f64 {
    match pitch {
        pitch if pitch < 2.0 => 0.15,
        pitch if pitch < 6.0 => 0.25,
        pitch if pitch < 14.0 => 0.5,
        _ => 1.0,
    }
}
