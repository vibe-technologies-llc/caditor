use std::{
    env,
    panic::{self, AssertUnwindSafe},
};

use arbitrary::Unstructured;

pub const CASES_VARIABLE: &str = "CADITOR_CASES";
pub const BYTES_PER_CASE: usize = 4096;

const INCREMENT: u64 = 0x9E37_79B9_7F4A_7C15;
const FIRST_MIX: u64 = 0xBF58_476D_1CE4_E5B9;
const SECOND_MIX: u64 = 0x94D0_49BB_1331_11EB;

pub struct Stream(u64);

impl Stream {
    pub const fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_word(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(INCREMENT);
        let mut mixed = self.0;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(FIRST_MIX);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(SECOND_MIX);
        mixed ^ (mixed >> 31)
    }

    pub fn bytes(&mut self, length: usize) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(length + 8);
        while bytes.len() < length {
            bytes.extend_from_slice(&self.next_word().to_le_bytes());
        }
        bytes.truncate(length);
        bytes
    }
}

pub fn cases(default: u64) -> u64 {
    env::var(CASES_VARIABLE)
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(default)
}

pub fn replay_case(seed: u64, mut check: impl FnMut(u64, &mut Unstructured)) {
    let bytes = Stream::new(seed).bytes(BYTES_PER_CASE);
    let mut input = Unstructured::new(&bytes);
    check(seed, &mut input);
}

pub fn for_each_case(name: &str, default: u64, mut check: impl FnMut(u64, &mut Unstructured)) {
    for seed in 0..cases(default) {
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| replay_case(seed, &mut check)));
        if let Err(failure) = outcome {
            eprintln!(
                "{name} failed on seed {seed}; rerun it with {CASES_VARIABLE}={}",
                seed + 1
            );
            panic::resume_unwind(failure);
        }
    }
}
