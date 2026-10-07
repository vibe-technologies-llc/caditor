const END_SIGNATURE: u32 = 0x0605_4b50;
const CENTRAL_SIGNATURE: u32 = 0x0201_4b50;
const LOCAL_SIGNATURE: u32 = 0x0403_4b50;
const END_SIZE: usize = 22;
const MAX_COMMENT: usize = 0xffff;
const CENTRAL_SIZE: usize = 46;
const LOCAL_SIZE: usize = 30;
const STORED: u16 = 0;
const DEFLATED: u16 = 8;

pub struct Archive<'a> {
    bytes: &'a [u8],
    entries: Vec<Entry>,
}

struct Entry {
    name: String,
    method: u16,
    packed: usize,
    local: usize,
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        <[u8; 2]>::try_from(bytes.get(at..at + 2)?).ok()?,
    ))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        <[u8; 4]>::try_from(bytes.get(at..at + 4)?).ok()?,
    ))
}

fn usize_at(bytes: &[u8], at: usize) -> Option<usize> {
    usize::try_from(u32_at(bytes, at)?).ok()
}

impl<'a> Archive<'a> {
    pub fn new(bytes: &'a [u8]) -> Option<Self> {
        let lowest = bytes.len().saturating_sub(END_SIZE + MAX_COMMENT);
        let end = (lowest..=bytes.len().checked_sub(END_SIZE)?)
            .rev()
            .find(|at| u32_at(bytes, *at) == Some(END_SIGNATURE))?;
        let count = usize::from(u16_at(bytes, end + 10)?);
        let mut at = usize_at(bytes, end + 16)?;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            if u32_at(bytes, at)? != CENTRAL_SIGNATURE {
                return None;
            }
            let name_length = usize::from(u16_at(bytes, at + 28)?);
            let extra = usize::from(u16_at(bytes, at + 30)?);
            let comment = usize::from(u16_at(bytes, at + 32)?);
            let name = bytes.get(at + CENTRAL_SIZE..at + CENTRAL_SIZE + name_length)?;
            entries.push(Entry {
                name: String::from_utf8_lossy(name).into_owned(),
                method: u16_at(bytes, at + 10)?,
                packed: usize_at(bytes, at + 20)?,
                local: usize_at(bytes, at + 42)?,
            });
            at += CENTRAL_SIZE + name_length + extra + comment;
        }
        Some(Self { bytes, entries })
    }

    pub fn read(&self, name: &str, limit: usize) -> Option<Vec<u8>> {
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.name.eq_ignore_ascii_case(name))?;
        let bytes = self.bytes;
        if u32_at(bytes, entry.local)? != LOCAL_SIGNATURE {
            return None;
        }
        let name_length = usize::from(u16_at(bytes, entry.local + 26)?);
        let extra = usize::from(u16_at(bytes, entry.local + 28)?);
        let start = entry.local + LOCAL_SIZE + name_length + extra;
        let packed = bytes.get(start..start.checked_add(entry.packed)?)?;
        match entry.method {
            STORED => Some(packed.to_vec()),
            DEFLATED => miniz_oxide::inflate::decompress_to_vec_with_limit(packed, limit).ok(),
            _ => None,
        }
    }
}
