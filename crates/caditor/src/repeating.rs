use std::borrow::Cow;

use crate::commands::{Command, Offer};

pub const NOTHING_TO_REPEAT: &str =
    "Run a modelling or sketch command first; there is nothing to repeat yet";

#[derive(Debug, Clone, Copy, Default)]
pub struct Repetition {
    last: Option<Command>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Repeat {
    Again(Command),
    Refused(String),
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
            None => Repeat::Refused(NOTHING_TO_REPEAT.to_owned()),
            Some(command) if offered(offers, command).is_some() => Repeat::Again(command),
            Some(command) => Repeat::Refused(not_here(command)),
        });
        let offer = match self.last {
            None => Offer {
                command: Command::RepeatLast,
                availability: Err(Cow::Borrowed(NOTHING_TO_REPEAT)),
                detail: None,
            },
            Some(command) => match offered(offers, command) {
                Some(found) => Offer {
                    command: Command::RepeatLast,
                    availability: found.availability.clone(),
                    detail: Some(found.title()),
                },
                None => Offer {
                    command: Command::RepeatLast,
                    availability: Err(Cow::Owned(not_here(command))),
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

fn not_here(command: Command) -> String {
    format!("{} is not available here", command.title())
}
