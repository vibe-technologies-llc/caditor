use std::{collections::VecDeque, sync::LazyLock};

use caditor_kernel::Solid;
use caditor_step::{ReadError, read_step};
use parking_lot::Mutex;

const BUDGET_BYTES: usize = 256 * 1024 * 1024;

struct Entry {
    digest: blake3::Hash,
    solid: Solid,
    bytes: usize,
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<Entry>,
    bytes: usize,
}

impl Cache {
    fn get(&mut self, digest: &blake3::Hash) -> Option<Solid> {
        let at = self
            .entries
            .iter()
            .position(|entry| entry.digest == *digest)?;
        let entry = self.entries.remove(at)?;
        let solid = entry.solid.clone();
        self.entries.push_back(entry);
        Some(solid)
    }

    fn insert(&mut self, digest: blake3::Hash, solid: &Solid, budget: usize) {
        let bytes = solid.approximate_size();
        if bytes > budget || self.entries.iter().any(|entry| entry.digest == digest) {
            return;
        }
        self.bytes += bytes;
        self.entries.push_back(Entry {
            digest,
            solid: solid.clone(),
            bytes,
        });
        while self.bytes > budget {
            let Some(oldest) = self.entries.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(oldest.bytes);
        }
    }
}

static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(Mutex::default);

pub(crate) fn first_solid(step: &str) -> Result<Option<Solid>, ReadError> {
    first_solid_within(&CACHE, step, BUDGET_BYTES)
}

fn first_solid_within(
    cache: &Mutex<Cache>,
    step: &str,
    budget: usize,
) -> Result<Option<Solid>, ReadError> {
    let digest = blake3::hash(step.as_bytes());
    if let Some(solid) = cache.lock().get(&digest) {
        return Ok(Some(solid));
    }
    let mut model = read_step(step)?;
    if model.solids.is_empty() {
        return Ok(None);
    }
    let solid = model.solids.swap_remove(0).solid;
    cache.lock().insert(digest, &solid, budget);
    Ok(Some(solid))
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use caditor_geometry::{Plane, Point2};
    use caditor_kernel::{LinearExtent, Profile, ProfileCurve, Selection, extrude};
    use caditor_step::{StepBody, write_step};

    use super::*;

    fn block_text(side: f64) -> String {
        let corners = [(0.0, 0.0), (side, 0.0), (side, side), (0.0, side)];
        let curves: Vec<ProfileCurve> = (0..4)
            .map(|index| {
                let (a, b) = (corners[index], corners[(index + 1) % 4]);
                ProfileCurve::line(
                    index as u64 + 1,
                    Point2::new(a.0, a.1),
                    Point2::new(b.0, b.1),
                )
            })
            .collect();
        let regions = Profile::new(&curves)
            .unwrap()
            .select(&Selection::EvenDepth)
            .unwrap();
        let solid = extrude(
            &Plane::XY,
            &regions,
            LinearExtent::one_side(side).unwrap(),
            1,
        )
        .unwrap();
        write_step(
            &[StepBody {
                name: "Block",
                solid: &solid,
            }],
            "Block",
            SystemTime::UNIX_EPOCH,
        )
        .unwrap()
    }

    #[test]
    fn a_text_read_twice_is_parsed_once_and_gives_the_same_solid() {
        let cache = Mutex::default();
        let text = block_text(11.0);

        let first = first_solid_within(&cache, &text, BUDGET_BYTES)
            .unwrap()
            .unwrap();
        assert_eq!(cache.lock().entries.len(), 1);
        let again = first_solid_within(&cache, &text, BUDGET_BYTES)
            .unwrap()
            .unwrap();

        assert_eq!(first, again);
        assert_eq!(cache.lock().entries.len(), 1);
    }

    #[test]
    fn the_oldest_solids_leave_when_the_budget_is_full() {
        let cache = Mutex::default();
        let texts = [block_text(5.0), block_text(6.0), block_text(7.0)];
        let one = first_solid_within(&Mutex::default(), &texts[0], BUDGET_BYTES)
            .unwrap()
            .unwrap()
            .approximate_size();
        let budget = one * 2 + one / 2;

        for text in &texts {
            first_solid_within(&cache, text, budget).unwrap();
        }

        let kept: Vec<blake3::Hash> = cache
            .lock()
            .entries
            .iter()
            .map(|entry| entry.digest)
            .collect();
        assert_eq!(
            kept,
            [
                blake3::hash(texts[1].as_bytes()),
                blake3::hash(texts[2].as_bytes())
            ]
        );
        assert!(cache.lock().bytes <= budget);
    }

    #[test]
    fn text_that_is_not_step_is_an_error_and_is_not_kept() {
        let cache = Mutex::default();

        assert!(first_solid_within(&cache, "not a model", BUDGET_BYTES).is_err());
        assert!(cache.lock().entries.is_empty());
    }
}
