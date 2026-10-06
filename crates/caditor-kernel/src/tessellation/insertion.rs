use spade::Point2 as PlanePoint;

const SHUFFLE_SEED: u64 = 0x005e_ed0f_f4ce;
const FIRST_ROUND: usize = 64;
const CURVE_STEPS: f64 = u32::MAX as f64;

struct SplitMix(u64);

impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut mixed = self.0;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        mixed ^ (mixed >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        let bound = bound as u64;
        ((u128::from(self.next()) * u128::from(bound)) >> 64) as usize
    }
}

fn spread(value: u32) -> u64 {
    let mut spread = u64::from(value);
    spread = (spread | (spread << 16)) & 0x0000_ffff_0000_ffff;
    spread = (spread | (spread << 8)) & 0x00ff_00ff_00ff_00ff;
    spread = (spread | (spread << 4)) & 0x0f0f_0f0f_0f0f_0f0f;
    spread = (spread | (spread << 2)) & 0x3333_3333_3333_3333;
    (spread | (spread << 1)) & 0x5555_5555_5555_5555
}

fn step(value: f64, low: f64, extent: f64) -> u32 {
    let fraction = if extent > 0.0 {
        (value - low) / extent
    } else {
        0.0
    };
    (fraction.clamp(0.0, 1.0) * CURVE_STEPS) as u32
}

pub(super) fn insertion_order(points: &[PlanePoint<f64>]) -> Vec<usize> {
    let mut shuffled: Vec<usize> = (0..points.len()).collect();
    let mut random = SplitMix(SHUFFLE_SEED);
    for index in (1..shuffled.len()).rev() {
        shuffled.swap(index, random.below(index + 1));
    }
    let low = points.iter().fold([f64::INFINITY; 2], |[x, y], point| {
        [x.min(point.x), y.min(point.y)]
    });
    let high = points.iter().fold([f64::NEG_INFINITY; 2], |[x, y], point| {
        [x.max(point.x), y.max(point.y)]
    });
    let extent = (high[0] - low[0]).max(high[1] - low[1]);
    let place = |index: &usize| {
        points.get(*index).map_or(0, |point| {
            spread(step(point.x, low[0], extent)) | (spread(step(point.y, low[1], extent)) << 1)
        })
    };
    let mut rounds = Vec::new();
    let mut end = shuffled.len();
    while end > 0 {
        let start = if end <= FIRST_ROUND { 0 } else { end / 2 };
        rounds.push(start..end);
        end = start;
    }
    let mut order = Vec::with_capacity(shuffled.len());
    for round in rounds.into_iter().rev() {
        let mut members = shuffled.get(round).unwrap_or_default().to_vec();
        members.sort_by_cached_key(place);
        order.extend(members);
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_point_is_inserted_once_in_a_fixed_order() {
        let points: Vec<PlanePoint<f64>> = (0..1000)
            .map(|index| {
                let angle = f64::from(index) * 0.01;
                PlanePoint::new(angle.cos(), angle.sin())
            })
            .collect();

        let order = insertion_order(&points);
        let mut sorted = order.clone();
        sorted.sort_unstable();

        assert_eq!(sorted, (0..1000).collect::<Vec<_>>());
        assert_eq!(order, insertion_order(&points));
        assert_ne!(order, sorted);
    }

    #[test]
    fn nothing_and_a_single_point_are_ordered() {
        assert!(insertion_order(&[]).is_empty());
        assert_eq!(insertion_order(&[PlanePoint::new(1.0, 2.0)]), vec![0]);
    }
}
