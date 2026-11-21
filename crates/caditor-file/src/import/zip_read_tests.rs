use crate::import::{
    MAX_UNPACKED_PART,
    zip_read::{Archive, EntryError},
};

const MATCH_LENGTH: usize = 258;
const FIXED_LITERAL_ZERO: (u32, u32) = (0x30, 8);
const FIXED_LONGEST_MATCH: (u32, u32) = (0xc5, 8);
const FIXED_NEAREST_DISTANCE: (u32, u32) = (0, 5);
const FIXED_END_OF_BLOCK: (u32, u32) = (0, 7);
const DEFLATED: u16 = 8;
const PART: &str = "3D/3dmodel.model";

#[derive(Default)]
struct Bits {
    bytes: Vec<u8>,
    used: u32,
}

impl Bits {
    fn push(&mut self, value: u32, count: u32) {
        for bit in 0..count {
            if self.used.is_multiple_of(8) {
                self.bytes.push(0);
            }

            let last = self.bytes.last_mut().unwrap();

            *last |= u8::from((value >> bit) & 1 == 1) << (self.used % 8);
            self.used += 1;
        }
    }

    fn code(&mut self, (code, length): (u32, u32)) {
        self.push(code.reverse_bits() >> (32 - length), length);
    }
}

pub(super) fn deflated_zeros(length: usize) -> Vec<u8> {
    let mut bits = Bits::default();
    let last_block = 1;
    let fixed_codes = 1;

    bits.push(last_block, 1);
    bits.push(fixed_codes, 2);

    let repeated = length.saturating_sub(1);

    bits.code(FIXED_LITERAL_ZERO);
    for _ in 0..repeated / MATCH_LENGTH {
        bits.code(FIXED_LONGEST_MATCH);
        bits.code(FIXED_NEAREST_DISTANCE);
    }
    for _ in 0..repeated % MATCH_LENGTH {
        bits.code(FIXED_LITERAL_ZERO);
    }
    bits.code(FIXED_END_OF_BLOCK);
    bits.bytes
}

fn package(name: &str, packed: &[u8]) -> Vec<u8> {
    let name_length = u16::try_from(name.len()).unwrap();
    let packed_length = u32::try_from(packed.len()).unwrap();

    let mut bytes = Vec::new();

    bytes.extend(0x0403_4b50_u32.to_le_bytes());
    bytes.extend([0; 22]);
    bytes.extend(name_length.to_le_bytes());
    bytes.extend([0; 2]);
    bytes.extend(name.as_bytes());
    bytes.extend(packed);

    let central = u32::try_from(bytes.len()).unwrap();

    bytes.extend(0x0201_4b50_u32.to_le_bytes());
    bytes.extend([0; 6]);
    bytes.extend(DEFLATED.to_le_bytes());
    bytes.extend([0; 8]);
    bytes.extend(packed_length.to_le_bytes());
    bytes.extend([0; 4]);
    bytes.extend(name_length.to_le_bytes());
    bytes.extend([0; 12]);
    bytes.extend(0_u32.to_le_bytes());
    bytes.extend(name.as_bytes());

    bytes.extend(0x0605_4b50_u32.to_le_bytes());
    bytes.extend([0; 6]);
    bytes.extend(1_u16.to_le_bytes());
    bytes.extend([0; 4]);
    bytes.extend(central.to_le_bytes());
    bytes.extend([0; 2]);
    bytes
}

#[test]
fn deflated_zeros_unpack_to_their_length() {
    for length in [1, 2, 258, 259, 1000, 70_000] {
        let unpacked = miniz_oxide::inflate::decompress_to_vec(&deflated_zeros(length)).unwrap();

        assert_eq!(unpacked.len(), length);
        assert!(unpacked.iter().all(|byte| *byte == 0));
    }
}

#[test]
fn a_part_unpacks_within_its_own_limit_whatever_the_package_has_left() {
    let small = package(PART, &deflated_zeros(1000));
    let large = package(PART, &deflated_zeros(MAX_UNPACKED_PART + 1));
    let package_left = usize::MAX;

    let read = Archive::new(&small).unwrap().read(PART, package_left);

    assert_eq!(read.map(|bytes| bytes.len()), Ok(1000));
    assert_eq!(
        Archive::new(&large)
            .unwrap()
            .read(PART, package_left)
            .map(|_| ()),
        Err(EntryError::TooLarge)
    );
}
