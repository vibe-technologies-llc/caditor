#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MetricSize {
    M1_6,
    M2,
    M2_5,
    M3,
    M4,
    M5,
    M6,
    M8,
    M10,
    M12,
    M16,
    M20,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SizeTable {
    name: &'static str,
    major: f64,
    pitch: f64,
    tap_drill: f64,
    clearance: [f64; 3],
    counterbore: (f64, f64),
    countersink: f64,
}

impl MetricSize {
    pub const ALL: [Self; 12] = [
        Self::M1_6,
        Self::M2,
        Self::M2_5,
        Self::M3,
        Self::M4,
        Self::M5,
        Self::M6,
        Self::M8,
        Self::M10,
        Self::M12,
        Self::M16,
        Self::M20,
    ];

    fn table(self) -> SizeTable {
        let size = |name, major, pitch, tap_drill, clearance, counterbore, countersink| SizeTable {
            name,
            major,
            pitch,
            tap_drill,
            clearance,
            counterbore,
            countersink,
        };
        match self {
            Self::M1_6 => size("M1.6", 1.6, 0.35, 1.25, [1.7, 1.8, 2.0], (3.5, 1.9), 3.7),
            Self::M2 => size("M2", 2.0, 0.4, 1.6, [2.2, 2.4, 2.6], (4.4, 2.3), 4.4),
            Self::M2_5 => size("M2.5", 2.5, 0.45, 2.05, [2.7, 2.9, 3.1], (5.4, 2.8), 5.5),
            Self::M3 => size("M3", 3.0, 0.5, 2.5, [3.2, 3.4, 3.6], (6.5, 3.4), 6.9),
            Self::M4 => size("M4", 4.0, 0.7, 3.3, [4.3, 4.5, 4.8], (8.0, 4.4), 9.2),
            Self::M5 => size("M5", 5.0, 0.8, 4.2, [5.3, 5.5, 5.8], (10.0, 5.4), 11.5),
            Self::M6 => size("M6", 6.0, 1.0, 5.0, [6.4, 6.6, 7.0], (11.0, 6.5), 13.7),
            Self::M8 => size("M8", 8.0, 1.25, 6.8, [8.4, 9.0, 10.0], (15.0, 8.6), 18.3),
            Self::M10 => size(
                "M10",
                10.0,
                1.5,
                8.5,
                [10.5, 11.0, 12.0],
                (18.0, 10.6),
                22.7,
            ),
            Self::M12 => size(
                "M12",
                12.0,
                1.75,
                10.2,
                [13.0, 13.5, 14.5],
                (20.0, 12.6),
                27.2,
            ),
            Self::M16 => size(
                "M16",
                16.0,
                2.0,
                14.0,
                [17.0, 17.5, 18.5],
                (26.0, 16.6),
                33.9,
            ),
            Self::M20 => size(
                "M20",
                20.0,
                2.5,
                17.5,
                [21.0, 22.0, 24.0],
                (33.0, 20.6),
                40.7,
            ),
        }
    }

    pub fn name(self) -> &'static str {
        self.table().name
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|size| size.name() == name)
    }

    pub fn major_diameter(self) -> f64 {
        self.table().major
    }

    pub fn pitch(self) -> f64 {
        self.table().pitch
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HoleFit {
    Close,
    Normal,
    Loose,
    Tapped,
}

impl HoleFit {
    pub const ALL: [Self; 4] = [Self::Close, Self::Normal, Self::Loose, Self::Tapped];

    pub fn id(self) -> &'static str {
        match self {
            Self::Close => "close",
            Self::Normal => "normal",
            Self::Loose => "loose",
            Self::Tapped => "tapped",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|fit| fit.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Close => "Close",
            Self::Normal => "Normal",
            Self::Loose => "Loose",
            Self::Tapped => "Tapped",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Close => {
                "A clearance hole the screw passes through with little play (ISO 273 fine)"
            }
            Self::Normal => "A clearance hole the screw passes through (ISO 273 medium)",
            Self::Loose => {
                "A clearance hole with room to line parts up (ISO 273 coarse), forgiving of a \
                 print that comes out small"
            }
            Self::Tapped => {
                "A hole of the tap drill size, threaded afterwards with a tap or by a \
                 self-tapping screw"
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HoleStandard {
    pub size: MetricSize,
    pub fit: HoleFit,
}

impl HoleStandard {
    pub fn diameter(self) -> f64 {
        let table = self.size.table();
        let [close, normal, loose] = table.clearance;
        match self.fit {
            HoleFit::Close => close,
            HoleFit::Normal => normal,
            HoleFit::Loose => loose,
            HoleFit::Tapped => table.tap_drill,
        }
    }

    pub fn counterbore(self) -> (f64, f64) {
        self.size.table().counterbore
    }

    pub fn countersink(self) -> f64 {
        self.size.table().countersink
    }

    pub fn is_threaded(self) -> bool {
        self.fit == HoleFit::Tapped
    }

    pub fn thread(self) -> Option<String> {
        self.is_threaded()
            .then(|| format!("{} × {}", self.size.name(), trimmed(self.size.pitch())))
    }

    pub fn label(self) -> String {
        match self.thread() {
            Some(thread) => format!("{thread} tapped"),
            None => format!(
                "{} {} fit",
                self.size.name(),
                self.fit.label().to_lowercase()
            ),
        }
    }
}

fn trimmed(value: f64) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}
