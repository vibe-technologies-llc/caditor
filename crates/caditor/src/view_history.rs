use std::collections::VecDeque;

use caditor_render::Viewpoint;

pub const KEPT_VIEWS: usize = 32;
pub const NO_EARLIER_VIEW: &str =
    "There is no earlier view in this session yet; orbit, pan, zoom or choose another view first";
const IDLE_SECONDS: f64 = 0.4;
const SAME_PLACE_SHARE: f64 = 1e-9;
const SAME_TURN_COSINE: f64 = 1.0 - 1e-12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gesture {
    Drag,
    Wheel,
    Keys,
}

impl Gesture {
    fn goes_on(self, idle: f64) -> bool {
        match self {
            Self::Drag => true,
            Self::Wheel | Self::Keys => idle <= IDLE_SECONDS,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Moving {
    start: Viewpoint,
    gesture: Gesture,
    last: f64,
}

#[derive(Debug, Default)]
pub struct ViewHistory {
    views: VecDeque<Viewpoint>,
    moving: Option<Moving>,
}

impl ViewHistory {
    pub fn leave(&mut self, from: Viewpoint) {
        self.settle();
        self.keep(from);
    }

    pub fn moving(&mut self, gesture: Gesture, from: Viewpoint, now: f64) {
        match &mut self.moving {
            Some(moving) if moving.gesture == gesture && gesture.goes_on(now - moving.last) => {
                moving.last = now;
            }
            _ => {
                self.settle();
                self.moving = Some(Moving {
                    start: from,
                    gesture,
                    last: now,
                });
            }
        }
    }

    pub fn stop_dragging(&mut self) {
        if self
            .moving
            .is_some_and(|moving| moving.gesture == Gesture::Drag)
        {
            self.settle();
        }
    }

    pub fn back(&mut self, current: Viewpoint) -> Option<Viewpoint> {
        self.settle();
        while let Some(view) = self.views.pop_back() {
            if !same(&view, &current) {
                return Some(view);
            }
        }
        None
    }

    pub fn availability(&self, current: Viewpoint) -> Result<(), &'static str> {
        let earlier = self
            .moving
            .iter()
            .map(|moving| moving.start)
            .chain(self.views.iter().copied())
            .any(|view| !same(&view, &current));
        if earlier {
            Ok(())
        } else {
            Err(NO_EARLIER_VIEW)
        }
    }

    pub fn clear(&mut self) {
        self.views.clear();
        self.moving = None;
    }

    fn settle(&mut self) {
        if let Some(moving) = self.moving.take() {
            self.keep(moving.start);
        }
    }

    fn keep(&mut self, view: Viewpoint) {
        if self.views.back().is_some_and(|last| same(last, &view)) {
            return;
        }
        self.views.push_back(view);
        while self.views.len() > KEPT_VIEWS {
            self.views.pop_front();
        }
    }
}

fn same(a: &Viewpoint, b: &Viewpoint) -> bool {
    let reach = a.distance.max(b.distance);
    a.target.distance(b.target) <= reach * SAME_PLACE_SHARE
        && (a.distance - b.distance).abs() <= reach * SAME_PLACE_SHARE
        && a.orientation.dot(b.orientation).abs() >= SAME_TURN_COSINE
}

#[cfg(test)]
mod tests {
    use caditor_geometry::{Point3, Vector3};

    use super::*;

    fn from(x: f64) -> Viewpoint {
        Viewpoint::looking_from(Vector3::new(x, -1.0, 1.0), Point3::ZERO, 100.0).unwrap()
    }

    #[test]
    fn going_back_returns_the_views_left_newest_first_and_skips_the_current_one() {
        let mut history = ViewHistory::default();
        history.leave(from(1.0));
        history.leave(from(2.0));
        history.leave(from(2.0));
        history.leave(from(3.0));

        assert_eq!(history.back(from(4.0)), Some(from(3.0)));
        assert_eq!(history.back(from(3.0)), Some(from(2.0)));
        assert_eq!(history.back(from(2.0)), Some(from(1.0)));
        assert_eq!(history.back(from(1.0)), None);
        assert_eq!(history.availability(from(1.0)), Err(NO_EARLIER_VIEW));
    }

    #[test]
    fn a_gesture_is_kept_once_from_where_it_started() {
        let mut history = ViewHistory::default();
        history.moving(Gesture::Drag, from(1.0), 0.0);
        history.moving(Gesture::Drag, from(1.5), 5.0);
        history.stop_dragging();
        history.moving(Gesture::Wheel, from(2.0), 6.0);
        history.moving(Gesture::Wheel, from(2.1), 6.2);
        history.moving(Gesture::Wheel, from(3.0), 7.0);
        history.moving(Gesture::Keys, from(4.0), 7.1);

        assert!(history.availability(from(5.0)).is_ok());
        assert_eq!(history.back(from(5.0)), Some(from(4.0)));
        assert_eq!(history.back(from(4.0)), Some(from(3.0)));
        assert_eq!(history.back(from(3.0)), Some(from(2.0)));
        assert_eq!(history.back(from(2.0)), Some(from(1.0)));
        assert_eq!(history.back(from(1.0)), None);
    }

    #[test]
    fn only_the_newest_views_are_kept_and_a_new_session_forgets_them() {
        let mut history = ViewHistory::default();
        for step in 0..KEPT_VIEWS + 8 {
            history.leave(from(step as f64));
        }

        let mut count = 0;
        let mut current = from(-1.0);
        while let Some(view) = history.back(current) {
            current = view;
            count += 1;
        }
        assert_eq!(count, KEPT_VIEWS);
        assert_eq!(current, from(8.0));

        history.leave(from(1.0));
        history.moving(Gesture::Drag, from(2.0), 0.0);
        history.clear();
        assert_eq!(history.availability(from(3.0)), Err(NO_EARLIER_VIEW));
    }
}
