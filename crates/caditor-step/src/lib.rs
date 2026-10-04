#[cfg(test)]
mod fixtures;
mod part21;
mod read;
mod write;

pub use crate::{
    read::{ReadError, StepModel, StepSolid, read_step},
    write::{
        SCHEMA, StepBody, StepWritten, WriteError, write_step, write_step_keeping_what_can_be,
    },
};
