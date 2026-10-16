use std::borrow::Cow;

use crate::commands::{Command, Offer, Reason};

pub const NOTHING_TO_REPEAT: &str =
    "Run a modelling or sketch command first; there is nothing to repeat yet";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RepeatRefusal {
    #[error("{NOTHING_TO_REPEAT}")]
    NothingYet,
    #[error("{} is not available here", .0.title())]
    NotHere(Command),
}

impl Reason for RepeatRefusal {
    fn reason(&self) -> Cow<'static, str> {
        match self {
            Self::NothingYet => Cow::Borrowed(NOTHING_TO_REPEAT),
            Self::NotHere(_) => Cow::Owned(self.to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Repetition {
    last: Option<Command>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Repeat {
    Again(Command),
    Refused(RepeatRefusal),
}

impl Repetition {
    pub fn settle(
        &mut self,
        asked: bool,
        ran: &[Command],
        offers: &mut Vec<Offer>,
    ) -> Option<Repeat> {
        let before = self.last;
        if let Some(command) = ran.iter().rev().find(|command| command.is_repeatable()) {
            self.last = Some(*command);
        }
        offers.retain(|offer| offer.command != Command::RepeatLast);

        let outcome = asked.then(|| match before {
            None => Repeat::Refused(RepeatRefusal::NothingYet),
            Some(command) if offered(offers, command).is_some() => Repeat::Again(command),
            Some(command) => Repeat::Refused(RepeatRefusal::NotHere(command)),
        });
        let offer = match self.last {
            None => Offer {
                command: Command::RepeatLast,
                availability: Err(Cow::Borrowed(NOTHING_TO_REPEAT)),
                pending: None,
                detail: None,
            },
            Some(command) => match offered(offers, command) {
                Some(found) => Offer {
                    command: Command::RepeatLast,
                    availability: found.availability.clone(),
                    pending: found.pending,
                    detail: Some(found.title()),
                },
                None => Offer {
                    command: Command::RepeatLast,
                    availability: Err(RepeatRefusal::NotHere(command).reason()),
                    pending: None,
                    detail: Some(command.title()),
                },
            },
        };
        offers.push(offer);
        outcome
    }
}

fn offered(offers: &[Offer], command: Command) -> Option<&Offer> {
    offers.iter().find(|offer| offer.command == command)
}
