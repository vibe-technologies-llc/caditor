use caditor_geometry::Vector3;
use caditor_kernel::TriangleMesh;

const SMALLEST_GUESSED_LONGEST_SIDE: f64 = 1e-4;
const SMALLEST_PLAUSIBLE_MILLIMETRES: f64 = 1.0;
const LARGEST_PLAUSIBLE_MILLIMETRES: f64 = 10_000.0;
const SIGNIFICANT_FIGURES: i32 = 3;
const MOST_DECIMALS: i32 = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshUnit {
    Micrometres,
    Millimetres,
    Centimetres,
    Metres,
}

impl MeshUnit {
    pub const ALL: [Self; 4] = [
        Self::Micrometres,
        Self::Millimetres,
        Self::Centimetres,
        Self::Metres,
    ];

    pub fn millimetres(self) -> f64 {
        match self {
            Self::Micrometres => 1e-3,
            Self::Millimetres => 1.0,
            Self::Centimetres => 10.0,
            Self::Metres => 1_000.0,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Micrometres => "micrometres",
            Self::Millimetres => "millimetres",
            Self::Centimetres => "centimetres",
            Self::Metres => "metres",
        }
    }

    pub fn guess(longest_side: f64) -> Option<Self> {
        if !longest_side.is_finite() {
            return None;
        }
        if longest_side > LARGEST_PLAUSIBLE_MILLIMETRES {
            Some(Self::Micrometres)
        } else if (SMALLEST_GUESSED_LONGEST_SIDE..SMALLEST_PLAUSIBLE_MILLIMETRES)
            .contains(&longest_side)
        {
            Some(Self::Metres)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitBasis {
    Assumed,
    Guessed,
    Chosen,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitReading {
    pub unit: MeshUnit,
    pub basis: UnitBasis,
    pub size: Vector3,
}

impl UnitReading {
    pub(super) fn of(mesh: &TriangleMesh, chosen: Option<MeshUnit>) -> Self {
        let size = size_of(mesh);
        let (unit, basis) = match chosen {
            Some(unit) => (unit, UnitBasis::Chosen),
            None => match MeshUnit::guess(size.max_element()) {
                Some(unit) => (unit, UnitBasis::Guessed),
                None => (MeshUnit::Millimetres, UnitBasis::Assumed),
            },
        };
        Self { unit, basis, size }
    }

    pub fn is_guess(&self) -> bool {
        self.basis == UnitBasis::Guessed
    }

    pub fn size_in_millimetres(&self, unit: MeshUnit) -> Vector3 {
        self.size * unit.millimetres()
    }

    pub(super) fn note(&self) -> String {
        const SCALE_ADVICE: &str =
            "if the part comes out the wrong size, scale it with Scale body.";
        let unit = self.unit.name();
        match self.basis {
            UnitBasis::Assumed => format!(
                "The file gives no units, so its sizes were read as millimetres; {SCALE_ADVICE}"
            ),
            UnitBasis::Guessed => {
                let as_written = self
                    .size_in_millimetres(MeshUnit::Millimetres)
                    .max_element();
                let as_read = self.size_in_millimetres(self.unit).max_element();
                format!(
                    "The file gives no units. Read as millimetres its longest side would be {} \
                     mm, which is unlikely for a part, so its sizes were read as {unit}, making \
                     it {} mm long; {SCALE_ADVICE}",
                    rounded(as_written),
                    rounded(as_read),
                )
            }
            UnitBasis::Chosen => format!(
                "The file gives no units, so its sizes were read as {unit}, as you chose; \
                 {SCALE_ADVICE}"
            ),
        }
    }

    pub(super) fn scale(&self, mesh: TriangleMesh) -> TriangleMesh {
        scaled(mesh, self.unit.millimetres())
    }
}

fn size_of(mesh: &TriangleMesh) -> Vector3 {
    let mut low = Vector3::splat(f64::INFINITY);
    let mut high = Vector3::splat(f64::NEG_INFINITY);
    for position in mesh
        .positions
        .iter()
        .filter(|position| position.is_finite())
    {
        low = low.min(*position);
        high = high.max(*position);
    }
    if low.cmple(high).all() {
        high - low
    } else {
        Vector3::ZERO
    }
}

fn scaled(mut mesh: TriangleMesh, factor: f64) -> TriangleMesh {
    if factor != 1.0 {
        for position in &mut mesh.positions {
            *position *= factor;
        }
    }
    mesh
}

fn rounded(value: f64) -> String {
    if value == 0.0 || !value.is_finite() {
        return "0".to_owned();
    }
    let magnitude = value.abs().log10().floor() as i32;
    let decimals = (SIGNIFICANT_FIGURES - 1 - magnitude).clamp(0, MOST_DECIMALS) as usize;
    let text = format!("{value:.decimals$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}
