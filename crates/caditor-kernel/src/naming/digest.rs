const OFFSET_BASIS: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Digest(u128);

impl Digest {
    pub(crate) fn new(tag: u8) -> Self {
        let mut digest = Self(OFFSET_BASIS);
        digest.byte(tag);
        digest
    }

    pub(crate) fn byte(&mut self, byte: u8) {
        self.0 ^= u128::from(byte);
        self.0 = self.0.wrapping_mul(PRIME);
    }

    fn bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.byte(*byte);
        }
    }

    pub(crate) fn count(&mut self, count: usize) {
        self.bytes(&u32::try_from(count).unwrap_or(u32::MAX).to_le_bytes());
    }

    pub(crate) fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }

    pub(crate) fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }

    pub(crate) fn u128(&mut self, value: u128) {
        self.bytes(&value.to_le_bytes());
    }

    pub(crate) fn finish(self) -> u128 {
        self.0
    }
}

#[cfg(test)]
pub(crate) fn fnv1a(bytes: &[u8]) -> u128 {
    let mut digest = Digest(OFFSET_BASIS);
    digest.bytes(bytes);
    digest.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_published_fnv1a_128_vectors() {
        assert_eq!(fnv1a(b""), 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d);
        assert_eq!(fnv1a(b"a"), 0xd228_cb69_6f1a_8caf_7891_2b70_4e4a_8964);
        assert_eq!(fnv1a(b"foobar"), 0x343e_1662_793c_64bf_6f0d_3597_ba44_6f18);
    }

    #[test]
    fn integers_are_written_little_endian() {
        let mut digest = Digest::new(7);
        digest.u32(0x0102_0304);
        digest.u64(5);
        let expected = fnv1a(&[7, 4, 3, 2, 1, 5, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(digest.finish(), expected);
    }
}
