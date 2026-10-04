#[cfg(test)]
mod tests;

use std::{
    ffi::{c_int, c_void},
    ptr::NonNull,
};

use libzstd_rs_sys::{
    ZSTD_CCtx, ZSTD_CCtx_refPrefix, ZSTD_CCtx_setParameter, ZSTD_CONTENTSIZE_ERROR,
    ZSTD_CONTENTSIZE_UNKNOWN, ZSTD_DCtx, ZSTD_DCtx_refPrefix, ZSTD_DCtx_setParameter,
    ZSTD_WINDOWLOG_MAX_64, ZSTD_WINDOWLOG_MIN, ZSTD_cParameter, ZSTD_compress2, ZSTD_compressBound,
    ZSTD_createCCtx, ZSTD_createDCtx, ZSTD_dParameter, ZSTD_decompressDCtx, ZSTD_freeCCtx,
    ZSTD_freeDCtx, ZSTD_getFrameContentSize, ZSTD_isError, ZSTD_maxCLevel, ZSTD_minCLevel,
};

mod code {
    pub const PREFIX_UNKNOWN: usize = 10;
    pub const VERSION_UNSUPPORTED: usize = 12;
    pub const FRAME_PARAMETER_UNSUPPORTED: usize = 14;
    pub const WINDOW_TOO_LARGE: usize = 16;
    pub const CORRUPTION_DETECTED: usize = 20;
    pub const CHECKSUM_WRONG: usize = 22;
    pub const LITERALS_HEADER_WRONG: usize = 24;
    pub const DICTIONARY_CORRUPTED: usize = 30;
    pub const DICTIONARY_WRONG: usize = 32;
    pub const PARAMETER_UNSUPPORTED: usize = 40;
    pub const PARAMETER_COMBINATION_UNSUPPORTED: usize = 41;
    pub const PARAMETER_OUT_OF_BOUND: usize = 42;
    pub const WORKSPACE_TOO_SMALL: usize = 66;
    pub const MEMORY_ALLOCATION: usize = 64;
    pub const DESTINATION_TOO_SMALL: usize = 70;
    pub const SOURCE_SIZE_WRONG: usize = 72;
}

const LONG_MATCHING_ON: c_int = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Level(i32);

impl Level {
    pub const FAST: Self = Self(3);
    pub const BALANCED: Self = Self(9);
    pub const SMALL: Self = Self(19);

    pub fn new(level: i32) -> Option<Self> {
        (ZSTD_minCLevel()..=ZSTD_maxCLevel())
            .contains(&level)
            .then_some(Self(level))
    }

    pub fn get(self) -> i32 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ZstdError {
    #[error("the compressor could not allocate its working memory")]
    OutOfMemory,
    #[error("the data is not a complete zstd frame")]
    NotAFrame,
    #[error("the frame does not record the size of its content")]
    UnknownSize,
    #[error("the frame holds {size} bytes, more than the {limit} allowed")]
    TooLarge { size: u64, limit: usize },
    #[error("the frame decoded to {actual} bytes instead of the {expected} it records")]
    WrongSize { expected: usize, actual: usize },
    #[error("the frame is damaged")]
    Corrupted,
    #[error("the frame's checksum does not match its content")]
    ChecksumMismatch,
    #[error("the frame needs a window larger than zstd allows")]
    WindowTooLarge,
    #[error("the frame was written for a version of zstd this one does not read")]
    UnsupportedVersion,
    #[error("the frame uses a feature this zstd does not support")]
    UnsupportedFrame,
    #[error("the dictionary or prefix does not belong to this frame")]
    WrongPrefix,
    #[error("the output buffer is too small for the result")]
    OutputTooSmall,
    #[error("the input is shorter or longer than the frame records")]
    InputSizeWrong,
    #[error("zstd refused a setting")]
    SettingRefused,
    #[error("zstd failed with error code {code}")]
    Unclassified { code: usize },
}

pub fn compress(data: &[u8], level: Level) -> Result<Vec<u8>, ZstdError> {
    compress_after(data, &[], level)
}

pub fn compress_after(data: &[u8], prefix: &[u8], level: Level) -> Result<Vec<u8>, ZstdError> {
    let context = Compressor::new()?;
    context.set_level(level)?;
    if !prefix.is_empty() {
        context.reach_back(prefix.len().saturating_add(data.len()))?;
    }
    context.compress(data, prefix)
}

pub fn decompress(frame: &[u8], limit: usize) -> Result<Vec<u8>, ZstdError> {
    decompress_after(frame, &[], limit)
}

pub fn decompress_after(frame: &[u8], prefix: &[u8], limit: usize) -> Result<Vec<u8>, ZstdError> {
    let size = content_size(frame)?;
    let expected = usize::try_from(size)
        .ok()
        .filter(|size| *size <= limit)
        .ok_or(ZstdError::TooLarge { size, limit })?;
    Decompressor::new()?.decompress(frame, prefix, expected)
}

pub fn content_size(frame: &[u8]) -> Result<u64, ZstdError> {
    #[allow(unsafe_code)]
    let size = unsafe { ZSTD_getFrameContentSize(frame.as_ptr().cast(), frame.len()) };
    match size {
        ZSTD_CONTENTSIZE_ERROR => Err(ZstdError::NotAFrame),
        ZSTD_CONTENTSIZE_UNKNOWN => Err(ZstdError::UnknownSize),
        size => Ok(size),
    }
}

fn window_log(span: usize) -> c_int {
    let bits = span
        .max(1)
        .checked_next_power_of_two()
        .map_or(usize::BITS, usize::trailing_zeros);
    c_int::try_from(bits)
        .unwrap_or(ZSTD_WINDOWLOG_MAX_64)
        .clamp(ZSTD_WINDOWLOG_MIN, ZSTD_WINDOWLOG_MAX_64)
}

fn checked(result: usize) -> Result<usize, ZstdError> {
    if ZSTD_isError(result) == 0 {
        return Ok(result);
    }
    Err(classified(result.wrapping_neg()))
}

fn classified(code: usize) -> ZstdError {
    match code {
        code::MEMORY_ALLOCATION | code::WORKSPACE_TOO_SMALL => ZstdError::OutOfMemory,
        code::PREFIX_UNKNOWN => ZstdError::NotAFrame,
        code::CORRUPTION_DETECTED | code::LITERALS_HEADER_WRONG => ZstdError::Corrupted,
        code::CHECKSUM_WRONG => ZstdError::ChecksumMismatch,
        code::WINDOW_TOO_LARGE => ZstdError::WindowTooLarge,
        code::VERSION_UNSUPPORTED => ZstdError::UnsupportedVersion,
        code::FRAME_PARAMETER_UNSUPPORTED => ZstdError::UnsupportedFrame,
        code::DICTIONARY_WRONG | code::DICTIONARY_CORRUPTED => ZstdError::WrongPrefix,
        code::DESTINATION_TOO_SMALL => ZstdError::OutputTooSmall,
        code::SOURCE_SIZE_WRONG => ZstdError::InputSizeWrong,
        code::PARAMETER_UNSUPPORTED
        | code::PARAMETER_COMBINATION_UNSUPPORTED
        | code::PARAMETER_OUT_OF_BOUND => ZstdError::SettingRefused,
        code => ZstdError::Unclassified { code },
    }
}

struct Compressor(NonNull<ZSTD_CCtx>);

impl Compressor {
    fn new() -> Result<Self, ZstdError> {
        #[allow(unsafe_code)]
        let context = unsafe { ZSTD_createCCtx() };
        NonNull::new(context)
            .map(Self)
            .ok_or(ZstdError::OutOfMemory)
    }

    fn set_level(&self, level: Level) -> Result<(), ZstdError> {
        self.set(ZSTD_cParameter::ZSTD_c_compressionLevel, level.get())
    }

    fn reach_back(&self, span: usize) -> Result<(), ZstdError> {
        self.set(ZSTD_cParameter::ZSTD_c_windowLog, window_log(span))?;
        self.set(
            ZSTD_cParameter::ZSTD_c_enableLongDistanceMatching,
            LONG_MATCHING_ON,
        )
    }

    fn set(&self, parameter: ZSTD_cParameter, value: c_int) -> Result<(), ZstdError> {
        #[allow(unsafe_code)]
        let code = unsafe { ZSTD_CCtx_setParameter(self.0.as_ptr(), parameter, value) };
        checked(code).map(drop)
    }

    fn compress(&self, data: &[u8], prefix: &[u8]) -> Result<Vec<u8>, ZstdError> {
        if !prefix.is_empty() {
            #[allow(unsafe_code)]
            let code = unsafe {
                ZSTD_CCtx_refPrefix(self.0.as_ptr(), prefix.as_ptr().cast(), prefix.len())
            };
            checked(code)?;
        }
        let capacity = checked(ZSTD_compressBound(data.len()))?;
        let mut frame = vec![0_u8; capacity];
        #[allow(unsafe_code)]
        let written = unsafe {
            ZSTD_compress2(
                self.0.as_ptr(),
                frame.as_mut_ptr().cast::<c_void>(),
                frame.len(),
                data.as_ptr().cast(),
                data.len(),
            )
        };
        frame.truncate(checked(written)?);
        Ok(frame)
    }
}

impl Drop for Compressor {
    fn drop(&mut self) {
        #[allow(unsafe_code)]
        unsafe {
            ZSTD_freeCCtx(self.0.as_ptr());
        }
    }
}

struct Decompressor(NonNull<ZSTD_DCtx>);

impl Decompressor {
    fn new() -> Result<Self, ZstdError> {
        #[allow(unsafe_code)]
        let context = unsafe { ZSTD_createDCtx() };
        NonNull::new(context)
            .map(Self)
            .ok_or(ZstdError::OutOfMemory)
    }

    fn accept_any_window(&self) -> Result<(), ZstdError> {
        #[allow(unsafe_code)]
        let code = unsafe {
            ZSTD_DCtx_setParameter(
                self.0.as_ptr(),
                ZSTD_dParameter::ZSTD_d_windowLogMax,
                ZSTD_WINDOWLOG_MAX_64,
            )
        };
        checked(code).map(drop)
    }

    fn decompress(
        &self,
        frame: &[u8],
        prefix: &[u8],
        expected: usize,
    ) -> Result<Vec<u8>, ZstdError> {
        self.accept_any_window()?;
        if !prefix.is_empty() {
            #[allow(unsafe_code)]
            let code = unsafe {
                ZSTD_DCtx_refPrefix(self.0.as_ptr(), prefix.as_ptr().cast(), prefix.len())
            };
            checked(code)?;
        }
        let mut content = vec![0_u8; expected];
        #[allow(unsafe_code)]
        let written = unsafe {
            ZSTD_decompressDCtx(
                self.0.as_ptr(),
                content.as_mut_ptr().cast::<c_void>(),
                content.len(),
                frame.as_ptr().cast(),
                frame.len(),
            )
        };
        let actual = checked(written)?;
        if actual != expected {
            return Err(ZstdError::WrongSize { expected, actual });
        }
        Ok(content)
    }
}

impl Drop for Decompressor {
    fn drop(&mut self) {
        #[allow(unsafe_code)]
        unsafe {
            ZSTD_freeDCtx(self.0.as_ptr());
        }
    }
}
