use super::ExportError;

pub(super) const LOCAL_HEADER_SIGNATURE: u32 = 0x0403_4b50;
pub(super) const CENTRAL_HEADER_SIGNATURE: u32 = 0x0201_4b50;
pub(super) const END_SIGNATURE: u32 = 0x0605_4b50;
pub(super) const STORED: u16 = 0;
pub(super) const DEFLATED: u16 = 8;
const VERSION_NEEDED: u16 = 20;
const NO_FLAGS: u16 = 0;
const JANUARY_1980: u16 = (1 << 5) | 1;
const MIDNIGHT: u16 = 0;
const COMPRESSION_LEVEL: u8 = 6;

pub(crate) struct ZipEntry<'a> {
    pub name: &'a str,
    pub contents: &'a [u8],
}

struct Written {
    name_length: u16,
    method: u16,
    crc: u32,
    compressed: u32,
    size: u32,
    offset: u32,
}

pub(crate) fn archive(entries: &[ZipEntry<'_>]) -> Result<Vec<u8>, ExportError> {
    let count = u16::try_from(entries.len()).map_err(|_| ExportError::TooLarge)?;
    let mut bytes = Vec::new();
    let mut written = Vec::with_capacity(entries.len());
    for entry in entries {
        written.push(local_entry(&mut bytes, entry)?);
    }
    let directory_start = offset(&bytes)?;
    for (entry, written) in entries.iter().zip(&written) {
        push_u32(&mut bytes, CENTRAL_HEADER_SIGNATURE);
        push_u16(&mut bytes, VERSION_NEEDED);
        push_u16(&mut bytes, VERSION_NEEDED);
        push_common(&mut bytes, written);
        for unused in [0; 4] {
            push_u16(&mut bytes, unused);
        }
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, written.offset);
        bytes.extend_from_slice(entry.name.as_bytes());
    }
    let directory_size = offset(&bytes)? - directory_start;
    push_u32(&mut bytes, END_SIGNATURE);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, count);
    push_u16(&mut bytes, count);
    push_u32(&mut bytes, directory_size);
    push_u32(&mut bytes, directory_start);
    push_u16(&mut bytes, 0);
    Ok(bytes)
}

fn local_entry(bytes: &mut Vec<u8>, entry: &ZipEntry<'_>) -> Result<Written, ExportError> {
    let deflated = miniz_oxide::deflate::compress_to_vec(entry.contents, COMPRESSION_LEVEL);
    let (method, data) = if deflated.len() < entry.contents.len() {
        (DEFLATED, deflated.as_slice())
    } else {
        (STORED, entry.contents)
    };
    let written = Written {
        name_length: u16::try_from(entry.name.len()).map_err(|_| ExportError::TooLarge)?,
        method,
        crc: crc32fast::hash(entry.contents),
        compressed: u32::try_from(data.len()).map_err(|_| ExportError::TooLarge)?,
        size: u32::try_from(entry.contents.len()).map_err(|_| ExportError::TooLarge)?,
        offset: offset(bytes)?,
    };
    push_u32(bytes, LOCAL_HEADER_SIGNATURE);
    push_u16(bytes, VERSION_NEEDED);
    push_common(bytes, &written);
    push_u16(bytes, 0);
    bytes.extend_from_slice(entry.name.as_bytes());
    bytes.extend_from_slice(data);
    Ok(written)
}

fn push_common(bytes: &mut Vec<u8>, written: &Written) {
    push_u16(bytes, NO_FLAGS);
    push_u16(bytes, written.method);
    push_u16(bytes, MIDNIGHT);
    push_u16(bytes, JANUARY_1980);
    push_u32(bytes, written.crc);
    push_u32(bytes, written.compressed);
    push_u32(bytes, written.size);
    push_u16(bytes, written.name_length);
}

fn offset(bytes: &[u8]) -> Result<u32, ExportError> {
    u32::try_from(bytes.len()).map_err(|_| ExportError::TooLarge)
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
