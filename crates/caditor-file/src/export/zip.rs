use std::io::{Seek, SeekFrom, Write};

use miniz_oxide::{
    DataFormat,
    deflate::{
        CompressionLevel,
        core::{CompressorOxide, TDEFLFlush, TDEFLStatus, compress},
    },
};

use super::{ExportError, writing};

pub(super) const LOCAL_HEADER_SIGNATURE: u32 = 0x0403_4b50;
pub(super) const CENTRAL_HEADER_SIGNATURE: u32 = 0x0201_4b50;
pub(super) const END_SIGNATURE: u32 = 0x0605_4b50;
pub(super) const ZIP64_END_SIGNATURE: u32 = 0x0606_4b50;
pub(super) const ZIP64_LOCATOR_SIGNATURE: u32 = 0x0706_4b50;
pub(super) const ZIP64_EXTRA: u16 = 0x0001;
pub(super) const STORED: u16 = 0;
pub(super) const DEFLATED: u16 = 8;
pub(super) const SATURATED: u32 = u32::MAX;
const VERSION_NEEDED: u16 = 20;
const ZIP64_VERSION: u16 = 45;
const NO_FLAGS: u16 = 0;
const JANUARY_1980: u16 = (1 << 5) | 1;
const MIDNIGHT: u16 = 0;
const COMPRESSION_LEVEL: u8 = 6;
const LOCAL_HEADER_LENGTH: u64 = 30;
const CRC_OFFSET: u64 = 14;
const LOCAL_ZIP64_EXTRA_LENGTH: u16 = 20;
const ZIP64_END_LENGTH: u64 = 44;
const DEFLATE_CHUNK: usize = 1 << 16;
const DEFLATE_SLACK_DIVISOR: u64 = 1000;
const DEFLATE_SLACK: u64 = 1 << 10;

#[cfg(test)]
pub(crate) struct ZipEntry<'a> {
    pub name: &'a str,
    pub contents: &'a [u8],
}

struct Written {
    name: String,
    method: u16,
    crc: u32,
    compressed: u64,
    size: u64,
    offset: u64,
}

impl Written {
    fn sizes_need_zip64(&self) -> bool {
        too_large_for_u32(self.compressed) || too_large_for_u32(self.size)
    }

    fn needs_zip64(&self) -> bool {
        self.sizes_need_zip64() || too_large_for_u32(self.offset)
    }
}

fn too_large_for_u32(value: u64) -> bool {
    value >= u64::from(SATURATED)
}

fn saturated(value: u64) -> u32 {
    u32::try_from(value)
        .ok()
        .filter(|value| *value < SATURATED)
        .unwrap_or(SATURATED)
}

pub(crate) struct ZipWriter<W> {
    out: W,
    position: u64,
    written: Vec<Written>,
}

impl<W: Write + Seek> ZipWriter<W> {
    pub fn new(out: W) -> Self {
        Self {
            out,
            position: 0,
            written: Vec::new(),
        }
    }

    fn put(&mut self, bytes: &[u8]) -> Result<(), ExportError> {
        self.out.write_all(bytes).map_err(writing)?;
        self.position += bytes.len() as u64;
        Ok(())
    }

    fn put_u16(&mut self, value: u16) -> Result<(), ExportError> {
        self.put(&value.to_le_bytes())
    }

    fn put_u32(&mut self, value: u32) -> Result<(), ExportError> {
        self.put(&value.to_le_bytes())
    }

    fn put_u64(&mut self, value: u64) -> Result<(), ExportError> {
        self.put(&value.to_le_bytes())
    }

    pub fn add(&mut self, name: &str, contents: &[u8]) -> Result<(), ExportError> {
        let deflated = miniz_oxide::deflate::compress_to_vec(contents, COMPRESSION_LEVEL);
        let (method, data) = if deflated.len() < contents.len() {
            (DEFLATED, deflated.as_slice())
        } else {
            (STORED, contents)
        };
        let entry = Written {
            name: name.to_owned(),
            method,
            crc: crc32fast::hash(contents),
            compressed: data.len() as u64,
            size: contents.len() as u64,
            offset: self.position,
        };
        if entry.sizes_need_zip64() {
            return Err(ExportError::TooLarge);
        }
        self.local_header(&entry, false)?;
        self.put(data)?;
        self.written.push(entry);
        Ok(())
    }

    pub fn add_deflated(
        &mut self,
        name: &str,
        size_bound: u64,
        fill: impl FnOnce(&mut Deflating<'_, W>) -> Result<(), ExportError>,
    ) -> Result<(), ExportError> {
        let zip64 = too_large_for_u32(
            size_bound
                .saturating_add(size_bound / DEFLATE_SLACK_DIVISOR)
                .saturating_add(DEFLATE_SLACK),
        );
        let mut entry = Written {
            name: name.to_owned(),
            method: DEFLATED,
            crc: 0,
            compressed: 0,
            size: 0,
            offset: self.position,
        };
        self.local_header(&entry, zip64)?;
        let data_start = self.position;
        let mut deflating = Deflating {
            zip: self,
            compressor: Box::new(CompressorOxide::with_format_and_level(
                DataFormat::Raw,
                CompressionLevel::DefaultLevel,
            )),
            crc: crc32fast::Hasher::new(),
            pending: Vec::with_capacity(DEFLATE_CHUNK),
            output: vec![0; DEFLATE_CHUNK],
            size: 0,
        };
        fill(&mut deflating)?;
        let (crc, size) = deflating.finish()?;
        entry.crc = crc;
        entry.size = size;
        entry.compressed = self.position - data_start;
        if !zip64 && entry.sizes_need_zip64() {
            return Err(ExportError::TooLarge);
        }
        self.patch_local_header(&entry, zip64)?;
        self.written.push(entry);
        Ok(())
    }

    fn local_header(&mut self, entry: &Written, zip64: bool) -> Result<(), ExportError> {
        let name_length = u16::try_from(entry.name.len()).map_err(|_| ExportError::TooLarge)?;
        self.put_u32(LOCAL_HEADER_SIGNATURE)?;
        self.put_u16(if zip64 { ZIP64_VERSION } else { VERSION_NEEDED })?;
        self.put_u16(NO_FLAGS)?;
        self.put_u16(entry.method)?;
        self.put_u16(MIDNIGHT)?;
        self.put_u16(JANUARY_1980)?;
        self.local_sizes(entry, zip64)?;
        self.put_u16(name_length)?;
        self.put_u16(if zip64 { LOCAL_ZIP64_EXTRA_LENGTH } else { 0 })?;
        self.put(entry.name.as_bytes())?;
        if zip64 {
            self.put_u16(ZIP64_EXTRA)?;
            self.put_u16(LOCAL_ZIP64_EXTRA_LENGTH - 4)?;
            self.put_u64(entry.size)?;
            self.put_u64(entry.compressed)?;
        }
        Ok(())
    }

    fn local_sizes(&mut self, entry: &Written, zip64: bool) -> Result<(), ExportError> {
        self.put_u32(entry.crc)?;
        if zip64 {
            self.put_u32(SATURATED)?;
            self.put_u32(SATURATED)
        } else {
            self.put_u32(saturated(entry.compressed))?;
            self.put_u32(saturated(entry.size))
        }
    }

    fn patch_local_header(&mut self, entry: &Written, zip64: bool) -> Result<(), ExportError> {
        let end = self.position;
        self.out
            .seek(SeekFrom::Start(entry.offset + CRC_OFFSET))
            .map_err(writing)?;
        self.local_sizes(entry, zip64)?;
        if zip64 {
            let extra = entry.offset + LOCAL_HEADER_LENGTH + entry.name.len() as u64 + 4;
            self.out.seek(SeekFrom::Start(extra)).map_err(writing)?;
            self.put_u64(entry.size)?;
            self.put_u64(entry.compressed)?;
        }
        self.out.seek(SeekFrom::Start(end)).map_err(writing)?;
        self.position = end;
        Ok(())
    }

    pub fn finish(mut self) -> Result<W, ExportError> {
        let written = std::mem::take(&mut self.written);
        let directory_start = self.position;
        for entry in &written {
            self.central_header(entry)?;
        }
        let directory_size = self.position - directory_start;
        let count = written.len() as u64;
        let zip64 = written.iter().any(Written::needs_zip64)
            || too_large_for_u32(directory_start)
            || too_large_for_u32(directory_size)
            || count >= u64::from(u16::MAX);
        if zip64 {
            let record = self.position;
            self.put_u32(ZIP64_END_SIGNATURE)?;
            self.put_u64(ZIP64_END_LENGTH)?;
            self.put_u16(ZIP64_VERSION)?;
            self.put_u16(ZIP64_VERSION)?;
            self.put_u32(0)?;
            self.put_u32(0)?;
            self.put_u64(count)?;
            self.put_u64(count)?;
            self.put_u64(directory_size)?;
            self.put_u64(directory_start)?;
            self.put_u32(ZIP64_LOCATOR_SIGNATURE)?;
            self.put_u32(0)?;
            self.put_u64(record)?;
            self.put_u32(1)?;
        }
        let short_count = u16::try_from(count)
            .ok()
            .filter(|count| *count < u16::MAX)
            .unwrap_or(u16::MAX);
        self.put_u32(END_SIGNATURE)?;
        self.put_u16(0)?;
        self.put_u16(0)?;
        self.put_u16(short_count)?;
        self.put_u16(short_count)?;
        self.put_u32(saturated(directory_size))?;
        self.put_u32(saturated(directory_start))?;
        self.put_u16(0)?;
        self.out.flush().map_err(writing)?;
        Ok(self.out)
    }

    fn central_header(&mut self, entry: &Written) -> Result<(), ExportError> {
        let name_length = u16::try_from(entry.name.len()).map_err(|_| ExportError::TooLarge)?;
        let large: Vec<u64> = [entry.size, entry.compressed, entry.offset]
            .into_iter()
            .filter(|value| too_large_for_u32(*value))
            .collect();
        let extra_length = if large.is_empty() {
            0
        } else {
            4 + 8 * large.len() as u16
        };
        let version = if large.is_empty() {
            VERSION_NEEDED
        } else {
            ZIP64_VERSION
        };
        self.put_u32(CENTRAL_HEADER_SIGNATURE)?;
        self.put_u16(version)?;
        self.put_u16(version)?;
        self.put_u16(NO_FLAGS)?;
        self.put_u16(entry.method)?;
        self.put_u16(MIDNIGHT)?;
        self.put_u16(JANUARY_1980)?;
        self.put_u32(entry.crc)?;
        self.put_u32(saturated(entry.compressed))?;
        self.put_u32(saturated(entry.size))?;
        self.put_u16(name_length)?;
        self.put_u16(extra_length)?;
        self.put_u16(0)?;
        self.put_u16(0)?;
        self.put_u16(0)?;
        self.put_u32(0)?;
        self.put_u32(saturated(entry.offset))?;
        self.put(entry.name.as_bytes())?;
        if !large.is_empty() {
            self.put_u16(ZIP64_EXTRA)?;
            self.put_u16(extra_length - 4)?;
            for value in large {
                self.put_u64(value)?;
            }
        }
        Ok(())
    }
}

pub(crate) struct Deflating<'z, W> {
    zip: &'z mut ZipWriter<W>,
    compressor: Box<CompressorOxide>,
    crc: crc32fast::Hasher,
    pending: Vec<u8>,
    output: Vec<u8>,
    size: u64,
}

impl<W: Write + Seek> Deflating<'_, W> {
    fn deflate(&mut self, flush: TDEFLFlush) -> Result<(), ExportError> {
        let mut input = std::mem::take(&mut self.pending);
        let mut rest = input.as_slice();
        loop {
            let (status, read, made) =
                compress(&mut self.compressor, rest, &mut self.output, flush);
            let produced = self.output.get(..made).unwrap_or_default();
            self.zip.out.write_all(produced).map_err(writing)?;
            self.zip.position += made as u64;
            rest = rest.get(read..).unwrap_or_default();
            match status {
                TDEFLStatus::Done => break,
                TDEFLStatus::Okay
                    if flush == TDEFLFlush::None && rest.is_empty() && made < self.output.len() =>
                {
                    break;
                }
                TDEFLStatus::Okay => {}
                TDEFLStatus::BadParam | TDEFLStatus::PutBufFailed => {
                    return Err(ExportError::Encoding);
                }
            }
        }
        input.clear();
        self.pending = input;
        Ok(())
    }

    fn finish(mut self) -> Result<(u32, u64), ExportError> {
        self.deflate(TDEFLFlush::Finish)?;
        Ok((self.crc.finalize(), self.size))
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), ExportError> {
        self.crc.update(bytes);
        self.size += bytes.len() as u64;
        self.pending.extend_from_slice(bytes);
        if self.pending.len() >= DEFLATE_CHUNK {
            self.deflate(TDEFLFlush::None)?;
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) fn archive(entries: &[ZipEntry<'_>]) -> Result<Vec<u8>, ExportError> {
    let mut zip = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for entry in entries {
        zip.add(entry.name, entry.contents)?;
    }
    Ok(zip.finish()?.into_inner())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn u16_at(bytes: &[u8], at: usize) -> u16 {
        u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
    }

    fn u32_at(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
    }

    fn u64_at(bytes: &[u8], at: usize) -> u64 {
        u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
    }

    #[test]
    fn a_streamed_entry_that_may_pass_four_gibibytes_keeps_its_sizes_in_a_zip64_field() {
        let text = "<vertex/>".repeat(10_000);
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        zip.add("small.txt", b"small").unwrap();
        let local = zip.position as usize;

        zip.add_deflated("big.model", u64::MAX, |deflating| {
            for chunk in text.as_bytes().chunks(777) {
                deflating.write_bytes(chunk)?;
            }
            Ok(())
        })
        .unwrap();
        let central = zip.position as usize;
        let bytes = zip.finish().unwrap().into_inner();
        let name_length = u16_at(&bytes, local + 26) as usize;
        let extra = local + 30 + name_length;
        let compressed = u64_at(&bytes, extra + 12) as usize;
        let data = &bytes[extra + 20..extra + 20 + compressed];
        let first_central = central;
        let second_central = first_central + 46 + "small.txt".len();

        assert_eq!(u32_at(&bytes, local), LOCAL_HEADER_SIGNATURE);
        assert_eq!(u16_at(&bytes, local + 4), ZIP64_VERSION);
        assert_eq!(u32_at(&bytes, local + 14), crc32fast::hash(text.as_bytes()));
        assert_eq!(
            [u32_at(&bytes, local + 18), u32_at(&bytes, local + 22)],
            [SATURATED; 2]
        );
        assert_eq!(u16_at(&bytes, local + 28), LOCAL_ZIP64_EXTRA_LENGTH);
        assert_eq!(u16_at(&bytes, extra), ZIP64_EXTRA);
        assert_eq!(u64_at(&bytes, extra + 4), text.len() as u64);
        assert_eq!(central, extra + 20 + compressed);
        assert_eq!(
            miniz_oxide::inflate::decompress_to_vec(data).unwrap(),
            text.as_bytes()
        );
        assert_eq!(u32_at(&bytes, second_central), CENTRAL_HEADER_SIGNATURE);
        assert_eq!(u32_at(&bytes, second_central + 20) as usize, compressed);
        assert_eq!(u32_at(&bytes, second_central + 24) as usize, text.len());
        assert_eq!(u32_at(&bytes, second_central + 42) as usize, local);
        assert_eq!(u32_at(&bytes, bytes.len() - 22), END_SIGNATURE);
    }

    #[test]
    fn sizes_and_offsets_past_four_gibibytes_move_into_zip64_records() {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        zip.written.push(Written {
            name: "huge.model".to_owned(),
            method: DEFLATED,
            crc: 7,
            compressed: 5 << 30,
            size: 9 << 30,
            offset: 6 << 30,
        });
        zip.position = 12 << 30;

        let bytes = zip.finish().unwrap().into_inner();
        let extra = 46 + "huge.model".len();
        let record = extra + 4 + 24;
        let locator = record + 12 + ZIP64_END_LENGTH as usize;
        let end = locator + 20;

        assert_eq!(u32_at(&bytes, 0), CENTRAL_HEADER_SIGNATURE);
        assert_eq!(u16_at(&bytes, 6), ZIP64_VERSION);
        assert_eq!(
            [u32_at(&bytes, 20), u32_at(&bytes, 24), u32_at(&bytes, 42)],
            [SATURATED; 3]
        );
        assert_eq!(u16_at(&bytes, 30), 28);
        assert_eq!(u16_at(&bytes, extra), ZIP64_EXTRA);
        assert_eq!(u16_at(&bytes, extra + 2), 24);
        assert_eq!(
            [
                u64_at(&bytes, extra + 4),
                u64_at(&bytes, extra + 12),
                u64_at(&bytes, extra + 20)
            ],
            [9 << 30, 5 << 30, 6 << 30]
        );
        assert_eq!(u32_at(&bytes, record), ZIP64_END_SIGNATURE);
        assert_eq!(u64_at(&bytes, record + 24), 1);
        assert_eq!(u64_at(&bytes, record + 40), record as u64);
        assert_eq!(u64_at(&bytes, record + 48), 12 << 30);
        assert_eq!(u32_at(&bytes, locator), ZIP64_LOCATOR_SIGNATURE);
        assert_eq!(u64_at(&bytes, locator + 8), (12 << 30) + record as u64);
        assert_eq!(u32_at(&bytes, end), END_SIGNATURE);
        assert_eq!(u16_at(&bytes, end + 10), 1);
        assert_eq!(
            [u32_at(&bytes, end + 12), u32_at(&bytes, end + 16)],
            [record as u32, SATURATED]
        );
        assert_eq!(bytes.len(), end + 22);
    }
}
