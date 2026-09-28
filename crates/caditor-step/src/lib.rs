#[cfg(test)]
mod fixtures;
mod part21;
mod read;
mod write;

pub use crate::{
    read::{ReadError, StepModel, StepSolid, read_step},
    write::{SCHEMA, StepBody, WriteError, write_step},
};
