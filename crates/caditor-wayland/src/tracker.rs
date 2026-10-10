use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropEvent {
    Hovered(Vec<PathBuf>),
    Left,
    Dropped(Vec<PathBuf>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ticket(u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step<O> {
    Accept { offer: O, serial: u32 },
    Refuse { offer: O, serial: u32 },
    Read { offer: O, ticket: Ticket },
    Finish(O),
    Discard(O),
    Emit(DropEvent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Entered {
    pub(crate) ours: bool,
    pub(crate) lists_files: bool,
    pub(crate) serial: u32,
}

#[derive(Debug)]
enum List {
    Reading(Ticket),
    Read(Vec<PathBuf>),
    Unread,
    NotFiles,
}

#[derive(Debug)]
struct Hover<O> {
    offer: O,
    serial: u32,
    list: List,
}

#[derive(Debug)]
struct Landing<O> {
    offer: O,
    ticket: Ticket,
}

#[derive(Debug)]
pub(crate) struct Tracker<O> {
    hover: Option<Hover<O>>,
    landing: Vec<Landing<O>>,
    tickets: u64,
}

impl<O> Default for Tracker<O> {
    fn default() -> Self {
        Self {
            hover: None,
            landing: Vec::new(),
            tickets: 0,
        }
    }
}

impl<O: Clone> Tracker<O> {
    pub(crate) fn entered(&mut self, offer: Option<O>, entered: Entered) -> Vec<Step<O>> {
        let mut steps = self.left();
        let Some(offer) = offer else {
            return steps;
        };
        let Entered {
            ours,
            lists_files,
            serial,
        } = entered;
        if !ours {
            steps.push(Step::Discard(offer));
            return steps;
        }
        let list = if lists_files {
            let ticket = self.ticket();
            steps.push(Step::Accept {
                offer: offer.clone(),
                serial,
            });
            steps.push(Step::Read {
                offer: offer.clone(),
                ticket,
            });
            List::Reading(ticket)
        } else {
            steps.push(Step::Refuse {
                offer: offer.clone(),
                serial,
            });
            List::NotFiles
        };
        self.hover = Some(Hover {
            offer,
            serial,
            list,
        });
        steps
    }

    pub(crate) fn left(&mut self) -> Vec<Step<O>> {
        let Some(hover) = self.hover.take() else {
            return Vec::new();
        };
        let shown = matches!(hover.list, List::Read(_));
        let mut steps = vec![Step::Discard(hover.offer)];
        if shown {
            steps.push(Step::Emit(DropEvent::Left));
        }
        steps
    }

    pub(crate) fn dropped(&mut self) -> Vec<Step<O>> {
        let Some(hover) = self.hover.take() else {
            return Vec::new();
        };
        match hover.list {
            List::Read(paths) => vec![
                Step::Finish(hover.offer),
                Step::Emit(DropEvent::Dropped(paths)),
            ],
            List::Reading(ticket) => {
                self.landing.push(Landing {
                    offer: hover.offer,
                    ticket,
                });
                Vec::new()
            }
            List::Unread => {
                let ticket = self.ticket();
                self.landing.push(Landing {
                    offer: hover.offer.clone(),
                    ticket,
                });
                vec![Step::Read {
                    offer: hover.offer,
                    ticket,
                }]
            }
            List::NotFiles => vec![Step::Discard(hover.offer)],
        }
    }

    pub(crate) fn read(&mut self, ticket: Ticket, paths: Option<Vec<PathBuf>>) -> Vec<Step<O>> {
        if let Some(hover) = self
            .hover
            .as_mut()
            .filter(|hover| matches!(hover.list, List::Reading(reading) if reading == ticket))
        {
            return match paths {
                Some(paths) if !paths.is_empty() => {
                    hover.list = List::Read(paths.clone());
                    vec![Step::Emit(DropEvent::Hovered(paths))]
                }
                Some(_) => {
                    hover.list = List::NotFiles;
                    vec![Step::Refuse {
                        offer: hover.offer.clone(),
                        serial: hover.serial,
                    }]
                }
                None => {
                    hover.list = List::Unread;
                    Vec::new()
                }
            };
        }
        let Some(at) = self
            .landing
            .iter()
            .position(|landing| landing.ticket == ticket)
        else {
            return Vec::new();
        };
        let landing = self.landing.remove(at);
        match paths {
            Some(paths) if !paths.is_empty() => vec![
                Step::Finish(landing.offer),
                Step::Emit(DropEvent::Dropped(paths)),
            ],
            _ => vec![Step::Discard(landing.offer)],
        }
    }

    pub(crate) fn offers(&self) -> impl Iterator<Item = &O> {
        self.hover
            .iter()
            .map(|hover| &hover.offer)
            .chain(self.landing.iter().map(|landing| &landing.offer))
    }

    fn ticket(&mut self) -> Ticket {
        self.tickets += 1;
        Ticket(self.tickets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERIAL: u32 = 7;
    const OVER_US: Entered = Entered {
        ours: true,
        lists_files: true,
        serial: SERIAL,
    };

    fn paths() -> Vec<PathBuf> {
        vec![PathBuf::from("/tmp/part.step"), PathBuf::from("/tmp/a.dxf")]
    }

    fn read_ticket(steps: &[Step<u32>]) -> Ticket {
        steps
            .iter()
            .find_map(|step| match step {
                Step::Read { ticket, .. } => Some(*ticket),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn files_hovered_over_the_window_show_then_drop_without_reading_again() {
        let mut tracker = Tracker::default();

        let entered = tracker.entered(Some(1), OVER_US);
        let ticket = read_ticket(&entered);
        let read = tracker.read(ticket, Some(paths()));
        let dropped = tracker.dropped();

        assert_eq!(
            entered,
            [
                Step::Accept {
                    offer: 1,
                    serial: SERIAL
                },
                Step::Read { offer: 1, ticket }
            ]
        );
        assert_eq!(read, [Step::Emit(DropEvent::Hovered(paths()))]);
        assert_eq!(
            dropped,
            [Step::Finish(1), Step::Emit(DropEvent::Dropped(paths()))]
        );
        assert_eq!(tracker.offers().count(), 0);
    }

    #[test]
    fn leaving_clears_what_was_shown_and_ignores_a_late_list() {
        let mut tracker = Tracker::default();

        let first = read_ticket(&tracker.entered(Some(1), OVER_US));
        let shown = tracker.read(first, Some(paths()));
        let left = tracker.left();
        let second = read_ticket(&tracker.entered(Some(2), OVER_US));
        let left_early = tracker.left();
        let late = tracker.read(second, Some(paths()));

        assert_eq!(shown.len(), 1);
        assert_eq!(left, [Step::Discard(1), Step::Emit(DropEvent::Left)]);
        assert_eq!(left_early, [Step::Discard(2)]);
        assert!(late.is_empty());
    }

    #[test]
    fn a_drag_over_another_surface_or_without_files_is_refused() {
        let mut tracker = Tracker::default();
        let elsewhere = Entered {
            ours: false,
            ..OVER_US
        };
        let text = Entered {
            lists_files: false,
            ..OVER_US
        };

        let other_surface = tracker.entered(Some(1), elsewhere);
        let not_files = tracker.entered(Some(2), text);
        let dropped = tracker.dropped();
        let no_offer = tracker.entered(None, OVER_US);

        assert_eq!(other_surface, [Step::Discard(1)]);
        assert_eq!(
            not_files,
            [Step::Refuse {
                offer: 2,
                serial: SERIAL
            }]
        );
        assert_eq!(dropped, [Step::Discard(2)]);
        assert!(no_offer.is_empty());
    }

    #[test]
    fn a_list_naming_no_files_is_refused_and_not_dropped() {
        let mut tracker = Tracker::default();

        let ticket = read_ticket(&tracker.entered(Some(1), OVER_US));
        let read = tracker.read(ticket, Some(Vec::new()));
        let dropped = tracker.dropped();

        assert_eq!(
            read,
            [Step::Refuse {
                offer: 1,
                serial: SERIAL
            }]
        );
        assert_eq!(dropped, [Step::Discard(1)]);
    }

    #[test]
    fn a_drop_before_the_list_arrives_waits_for_it() {
        let mut tracker = Tracker::default();

        let ticket = read_ticket(&tracker.entered(Some(1), OVER_US));
        let dropped = tracker.dropped();
        let waiting = tracker.offers().count();
        let read = tracker.read(ticket, Some(paths()));

        assert!(dropped.is_empty());
        assert_eq!(waiting, 1);
        assert_eq!(
            read,
            [Step::Finish(1), Step::Emit(DropEvent::Dropped(paths()))]
        );
        assert_eq!(tracker.offers().count(), 0);
    }

    #[test]
    fn a_failed_hover_read_is_read_again_on_the_drop() {
        let mut tracker = Tracker::default();

        let hover = read_ticket(&tracker.entered(Some(1), OVER_US));
        let failed = tracker.read(hover, None);
        let dropped = tracker.dropped();
        let again = read_ticket(&dropped);
        let failed_again = tracker.read(again, None);

        assert!(failed.is_empty());
        assert_ne!(again, hover);
        assert_eq!(failed_again, [Step::Discard(1)]);
    }

    #[test]
    fn entering_again_without_a_leave_ends_the_earlier_drag() {
        let mut tracker = Tracker::default();

        let ticket = read_ticket(&tracker.entered(Some(1), OVER_US));
        tracker.read(ticket, Some(paths()));
        let entered = tracker.entered(Some(2), OVER_US);

        assert_eq!(entered.first(), Some(&Step::Discard(1)));
        assert_eq!(entered.get(1), Some(&Step::Emit(DropEvent::Left)));
    }
}
