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
pub struct HeatSetInsert {
    pub hole: f64,
    pub length: f64,
    pub wall: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SizeTable {
    name: &'static str,
    major: f64,
    pitch: f64,
    fine_pitches: &'static [f64],
    tap_drill: f64,
    clearance: [f64; 3],
    counterbore: (f64, f64),
    countersink: f64,
    insert: Option<HeatSetInsert>,
}

struct Threads {
    major: f64,
    pitch: f64,
    fine_pitches: &'static [f64],
    tap_drill: f64,
}

struct Head {
    counterbore: (f64, f64),
    countersink: f64,
}

const fn insert(hole: f64, length: f64, wall: f64) -> Option<HeatSetInsert> {
    Some(HeatSetInsert { hole, length, wall })
}

const fn size(
    name: &'static str,
    threads: Threads,
    clearance: [f64; 3],
    head: Head,
    insert: Option<HeatSetInsert>,
) -> SizeTable {
    SizeTable {
        name,
        major: threads.major,
        pitch: threads.pitch,
        fine_pitches: threads.fine_pitches,
        tap_drill: threads.tap_drill,
        clearance,
        counterbore: head.counterbore,
        countersink: head.countersink,
        insert,
    }
}

const fn threads(major: f64, pitch: f64, fine_pitches: &'static [f64], tap_drill: f64) -> Threads {
    Threads {
        major,
        pitch,
        fine_pitches,
        tap_drill,
    }
}

const fn head(counterbore: (f64, f64), countersink: f64) -> Head {
    Head {
        counterbore,
        countersink,
    }
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
        match self {
            Self::M1_6 => size(
                "M1.6",
                threads(1.6, 0.35, &[0.2], 1.25),
                [1.7, 1.8, 2.0],
                head((3.5, 1.9), 3.7),
                None,
            ),
            Self::M2 => size(
                "M2",
                threads(2.0, 0.4, &[0.25], 1.6),
                [2.2, 2.4, 2.6],
                head((4.4, 2.3), 4.4),
                insert(3.2, 3.0, 1.3),
            ),
            Self::M2_5 => size(
                "M2.5",
                threads(2.5, 0.45, &[0.35], 2.05),
                [2.7, 2.9, 3.1],
                head((5.4, 2.8), 5.5),
                insert(4.0, 4.0, 1.6),
            ),
            Self::M3 => size(
                "M3",
                threads(3.0, 0.5, &[0.35], 2.5),
                [3.2, 3.4, 3.6],
                head((6.5, 3.4), 6.9),
                insert(4.0, 5.7, 1.6),
            ),
            Self::M4 => size(
                "M4",
                threads(4.0, 0.7, &[0.5], 3.3),
                [4.3, 4.5, 4.8],
                head((8.0, 4.4), 9.2),
                insert(5.6, 8.1, 2.1),
            ),
            Self::M5 => size(
                "M5",
                threads(5.0, 0.8, &[0.5], 4.2),
                [5.3, 5.5, 5.8],
                head((10.0, 5.4), 11.5),
                insert(6.4, 9.5, 2.6),
            ),
            Self::M6 => size(
                "M6",
                threads(6.0, 1.0, &[0.75], 5.0),
                [6.4, 6.6, 7.0],
                head((11.0, 6.5), 13.7),
                insert(8.0, 12.7, 3.3),
            ),
            Self::M8 => size(
                "M8",
                threads(8.0, 1.25, &[1.0, 0.75], 6.8),
                [8.4, 9.0, 10.0],
                head((15.0, 8.6), 18.3),
                insert(9.7, 12.7, 3.3),
            ),
            Self::M10 => size(
                "M10",
                threads(10.0, 1.5, &[1.25, 1.0, 0.75], 8.5),
                [10.5, 11.0, 12.0],
                head((18.0, 10.6), 22.7),
                None,
            ),
            Self::M12 => size(
                "M12",
                threads(12.0, 1.75, &[1.25, 1.5, 1.0], 10.2),
                [13.0, 13.5, 14.5],
                head((20.0, 12.6), 27.2),
                None,
            ),
            Self::M16 => size(
                "M16",
                threads(16.0, 2.0, &[1.5, 1.0], 14.0),
                [17.0, 17.5, 18.5],
                head((26.0, 16.6), 33.9),
                None,
            ),
            Self::M20 => size(
                "M20",
                threads(20.0, 2.5, &[1.5, 2.0, 1.0], 17.5),
                [21.0, 22.0, 24.0],
                head((33.0, 20.6), 40.7),
                None,
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

    pub fn fine_pitch(self) -> f64 {
        self.fine_pitches().first().copied().unwrap_or(self.pitch())
    }

    pub fn fine_pitches(self) -> &'static [f64] {
        self.table().fine_pitches
    }

    pub fn heat_set_insert(self) -> Option<HeatSetInsert> {
        self.table().insert
    }

    pub fn offers(self, fit: HoleFit) -> bool {
        match fit {
            HoleFit::Close | HoleFit::Normal | HoleFit::Loose | HoleFit::Tapped => true,
            HoleFit::TappedFine(pitch) => self.fine_pitches().get(pitch.index()).is_some(),
            HoleFit::HeatSetInsert => self.heat_set_insert().is_some(),
        }
    }

    pub fn fits(self) -> Vec<HoleFit> {
        HoleFit::KINDS
            .into_iter()
            .filter(|fit| self.offers(*fit))
            .collect()
    }

    pub fn fine_fits(self) -> Vec<HoleFit> {
        FinePitch::ALL
            .into_iter()
            .map(HoleFit::TappedFine)
            .filter(|fit| self.offers(*fit))
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FinePitch {
    First,
    Second,
    Third,
}

impl FinePitch {
    pub const ALL: [Self; 3] = [Self::First, Self::Second, Self::Third];

    fn index(self) -> usize {
        match self {
            Self::First => 0,
            Self::Second => 1,
            Self::Third => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HoleFit {
    Close,
    Normal,
    Loose,
    Tapped,
    TappedFine(FinePitch),
    HeatSetInsert,
}

impl HoleFit {
    pub const KINDS: [Self; 6] = [
        Self::Close,
        Self::Normal,
        Self::Loose,
        Self::Tapped,
        Self::TappedFine(FinePitch::First),
        Self::HeatSetInsert,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Close => "close",
            Self::Normal => "normal",
            Self::Loose => "loose",
            Self::Tapped => "tapped",
            Self::TappedFine(FinePitch::First) => "tapped_fine",
            Self::TappedFine(FinePitch::Second) => "tapped_fine_2",
            Self::TappedFine(FinePitch::Third) => "tapped_fine_3",
            Self::HeatSetInsert => "heat_set_insert",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::KINDS
            .into_iter()
            .chain(FinePitch::ALL.map(Self::TappedFine))
            .find(|fit| fit.id() == id)
    }

    pub fn is_fine(self) -> bool {
        matches!(self, Self::TappedFine(_))
    }

    pub fn same_kind(self, other: Self) -> bool {
        self == other || (self.is_fine() && other.is_fine())
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Close => "Close",
            Self::Normal => "Normal",
            Self::Loose => "Loose",
            Self::Tapped => "Tapped",
            Self::TappedFine(_) => "Fine",
            Self::HeatSetInsert => "Insert",
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
            Self::TappedFine(_) => {
                "A hole of the tap drill size for a fine pitch thread (ISO 261), threaded \
                 afterwards with a fine tap"
            }
            Self::HeatSetInsert => {
                "A hole for a threaded heat-set insert pressed in with a soldering iron, sized \
                 for the common standard brass inserts"
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
    pub fn offered(size: MetricSize, fit: HoleFit) -> Self {
        let fit = match fit {
            _ if size.offers(fit) => fit,
            HoleFit::TappedFine(_) => HoleFit::TappedFine(FinePitch::First),
            HoleFit::Close
            | HoleFit::Normal
            | HoleFit::Loose
            | HoleFit::Tapped
            | HoleFit::HeatSetInsert => HoleFit::Normal,
        };
        Self { size, fit }
    }

    pub fn diameter(self) -> f64 {
        let table = self.size.table();
        let [close, normal, loose] = table.clearance;
        match self.fit {
            HoleFit::Close => close,
            HoleFit::Normal => normal,
            HoleFit::Loose => loose,
            HoleFit::Tapped => table.tap_drill,
            HoleFit::TappedFine(pitch) => {
                let fine = self
                    .size
                    .fine_pitches()
                    .get(pitch.index())
                    .copied()
                    .unwrap_or(self.size.fine_pitch());
                micrometres(table.major - fine)
            }
            HoleFit::HeatSetInsert => table.insert.map_or(normal, |insert| insert.hole),
        }
    }

    pub fn counterbore(self) -> (f64, f64) {
        self.size.table().counterbore
    }

    pub fn countersink(self) -> f64 {
        self.size.table().countersink
    }

    pub fn is_threaded(self) -> bool {
        self.thread_pitch().is_some()
    }

    pub fn thread_pitch(self) -> Option<f64> {
        match self.fit {
            HoleFit::Tapped => Some(self.size.pitch()),
            HoleFit::TappedFine(pitch) => self.size.fine_pitches().get(pitch.index()).copied(),
            HoleFit::Close | HoleFit::Normal | HoleFit::Loose | HoleFit::HeatSetInsert => None,
        }
    }

    pub fn thread(self) -> Option<String> {
        self.thread_pitch()
            .map(|pitch| format!("{} × {}", self.size.name(), trimmed(pitch)))
    }

    pub fn insert(self) -> Option<HeatSetInsert> {
        (self.fit == HoleFit::HeatSetInsert)
            .then(|| self.size.heat_set_insert())
            .flatten()
    }

    pub fn label(self) -> String {
        if let Some(thread) = self.thread() {
            return format!("{thread} tapped");
        }
        if self.fit == HoleFit::HeatSetInsert {
            return format!("{} heat-set insert", self.size.name());
        }
        format!(
            "{} {} fit",
            self.size.name(),
            self.fit.label().to_lowercase()
        )
    }
}

pub fn pitch_text(pitch: f64) -> String {
    trimmed(pitch)
}

fn micrometres(millimetres: f64) -> f64 {
    (millimetres * 1000.0).round() / 1000.0
}

fn trimmed(value: f64) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}
