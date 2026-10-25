use std::{collections::BTreeMap, fmt, sync::Arc};

use caditor_kernel::{Interrupt, Mesh, MeshQuality, Solid, interruptible};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Watertightness {
    NoTriangles,
    DegenerateTriangle,
    EdgeUsedTwice { from: u32, to: u32 },
    EdgeWithoutOpposite { from: u32, to: u32 },
}

impl fmt::Display for Watertightness {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoTriangles => formatter.write_str("the mesh has no triangles"),
            Self::DegenerateTriangle => formatter.write_str("the mesh has a degenerate triangle"),
            Self::EdgeUsedTwice { from, to } => {
                write!(formatter, "the edge {from}->{to} is used more than once")
            }
            Self::EdgeWithoutOpposite { from, to } => {
                write!(formatter, "the edge {from}->{to} has no opposite")
            }
        }
    }
}

pub fn watertightness(mesh: &Mesh) -> Result<(), Watertightness> {
    let mut directed: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    for [a, b, c] in mesh.position_triangles() {
        if a == b || b == c || a == c {
            return Err(Watertightness::DegenerateTriangle);
        }
        for edge in [(a, b), (b, c), (c, a)] {
            *directed.entry(edge).or_default() += 1;
        }
    }
    if directed.is_empty() {
        return Err(Watertightness::NoTriangles);
    }
    for ((from, to), count) in &directed {
        if *count != 1 {
            return Err(Watertightness::EdgeUsedTwice {
                from: *from,
                to: *to,
            });
        }
        if directed.get(&(*to, *from)) != Some(&1) {
            return Err(Watertightness::EdgeWithoutOpposite {
                from: *from,
                to: *to,
            });
        }
    }
    Ok(())
}

pub fn assert_sound(solid: &Solid, quality: &MeshQuality) {
    let never: Interrupt = Arc::new(|| false);
    if let Err(error) = interruptible(never, || solid.validate()) {
        panic!("a solid is invalid: {error}");
    }
    let mesh = match solid.display_mesh(quality) {
        Ok(mesh) => mesh,
        Err(error) => panic!("a valid solid could not be meshed: {error}"),
    };
    if let Err(error) = watertightness(&mesh) {
        panic!("a valid solid meshes badly: {error}");
    }
    let mass = mesh.mass_properties();
    assert!(
        mass.volume.is_finite() && mass.area.is_finite() && mass.centroid.is_finite(),
        "a solid has non-finite mass properties: {mass:?}"
    );
}
