use caditor_geometry::Point2;

use crate::{
    build::wrap::WrapError,
    interval::Interval,
    profile::{
        Course, OffsetFailure, Piece, Region, offset_courses, outline_of, polygons_cross,
        signed_area,
    },
    tolerance::LINEAR_RESOLUTION,
};

const RELATIVE_TOLERANCE: f64 = 1e-7;

pub(super) struct Opened {
    pub(super) piece: Piece,
    whole: Interval,
    reversed: bool,
    index: usize,
}

impl Opened {
    pub(super) fn whole(piece: &Piece, index: usize) -> Self {
        Self {
            piece: piece.clone(),
            whole: piece.range(),
            reversed: piece.is_reversed(),
            index,
        }
    }

    pub(super) fn half(piece: Piece, whole: &Piece, index: usize) -> Self {
        Self {
            piece,
            whole: whole.range(),
            reversed: whole.is_reversed(),
            index,
        }
    }

    fn fraction(&self, parameter: f64) -> f64 {
        let share = (parameter - self.whole.start()) / self.whole.length();
        if self.reversed { 1.0 - share } else { share }
    }
}

pub(super) struct Leaning {
    levels: Option<[Vec<Course>; 2]>,
}

impl Leaning {
    pub(super) fn straight() -> Self {
        Self { levels: None }
    }

    pub(super) fn point(
        &self,
        opened: &Opened,
        parameter: f64,
        level: usize,
    ) -> Result<Point2, WrapError> {
        let Some(levels) = &self.levels else {
            return Ok(opened.piece.curve().point(parameter));
        };
        levels
            .get(level)
            .and_then(|courses| courses.get(opened.index))
            .map(|course| course.at(opened.fraction(parameter)))
            .ok_or(WrapError::Unassembled)
    }
}

fn closes(failure: OffsetFailure, pieces: &[Piece]) -> WrapError {
    let mut entities: Vec<u64> = failure
        .strands()
        .into_iter()
        .filter_map(|index| pieces.get(index).map(Piece::entity))
        .collect();
    entities.sort_unstable();
    entities.dedup();
    WrapError::DraftCloses { entities }
}

struct Offsets<'a> {
    pieces: &'a [Piece],
    originals: Vec<Course>,
    moved: [Vec<Course>; 2],
}

impl Offsets<'_> {
    fn entities(&self) -> Vec<u64> {
        let mut entities: Vec<u64> = self.pieces.iter().map(Piece::entity).collect();
        entities.sort_unstable();
        entities.dedup();
        entities
    }
}

fn check_level(
    offsets: &[Vec<Offsets<'_>>],
    level: usize,
    tolerance: f64,
) -> Result<(), WrapError> {
    let mut polygons = Vec::new();
    for loops in offsets {
        for offset in loops {
            let Some(courses) = offset.moved.get(level) else {
                continue;
            };
            let outline = outline_of(courses);
            let area = signed_area(&outline);
            let before = signed_area(&outline_of(&offset.originals));
            if area.abs() <= tolerance * tolerance || area.signum() != before.signum() {
                return Err(WrapError::DraftCloses {
                    entities: offset.entities(),
                });
            }
            polygons.push(outline);
        }
    }
    if polygons_cross(&polygons) {
        return Err(WrapError::DraftCloses {
            entities: Vec::new(),
        });
    }
    Ok(())
}

pub(super) fn leanings(
    regions: &[Region],
    insets: [f64; 2],
) -> Result<Vec<Vec<Leaning>>, WrapError> {
    if insets.iter().all(|inset| *inset == 0.0) {
        return Ok(regions
            .iter()
            .map(|region| region.loops().map(|_| Leaning::straight()).collect())
            .collect());
    }
    let size = regions
        .iter()
        .filter_map(Region::bounds)
        .map(|bounds| {
            bounds.size().length() + bounds.min().abs().max(bounds.max().abs()).max_element()
        })
        .fold(1.0, f64::max);
    let tolerance = (size * RELATIVE_TOLERANCE).max(LINEAR_RESOLUTION);
    let mut offsets = Vec::with_capacity(regions.len());
    for region in regions {
        let mut loops = Vec::new();
        for profile_loop in region.loops() {
            let pieces = profile_loop.pieces();
            let originals: Vec<Course> = pieces.iter().map(Course::of_piece).collect();
            let level = |inset: f64| {
                offset_courses(&originals, true, inset, None, tolerance)
                    .map_err(|failure| closes(failure, pieces))
            };
            let moved = [level(insets[0])?, level(insets[1])?];
            loops.push(Offsets {
                pieces,
                originals,
                moved,
            });
        }
        offsets.push(loops);
    }
    for level in 0..2 {
        check_level(&offsets, level, tolerance)?;
    }
    Ok(offsets
        .into_iter()
        .map(|loops| {
            loops
                .into_iter()
                .map(|offset| Leaning {
                    levels: Some(offset.moved),
                })
                .collect()
        })
        .collect())
}
