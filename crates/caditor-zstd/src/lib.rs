#[cfg(test)]
mod tests;

use std::{
    ffi::{CStr, c_int, c_void},
    ptr::NonNull,
};

use libzstd_rs_sys::{
    ZSTD_CCtx, ZSTD_CCtx_refPrefix, ZSTD_CCtx_setParameter, ZSTD_CONTENTSIZE_ERROR,
    ZSTD_CONTENTSIZE_UNKNOWN, ZSTD_DCtx, ZSTD_DCtx_refPrefix, ZSTD_cParameter, ZSTD_compress2,
    ZSTD_compressBound, ZSTD_createCCtx, ZSTD_createDCtx, ZSTD_decompressDCtx, ZSTD_freeCCtx,
    ZSTD_freeDCtx, ZSTD_getErrorName, ZSTD_getFrameContentSize, ZSTD_isError, ZSTD_maxCLevel,
    ZSTD_minCLevel,
};

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
    #[error("zstd reported: {0}")]
    Library(String),
}

pub fn compress(data: &[u8], level: Level) -> Result<Vec<u8>, ZstdError> {
    compress_after(data, &[], level)
}

pub fn compress_after(data: &[u8], prefix: &[u8], level: Level) -> Result<Vec<u8>, ZstdError> {
    let context = Compressor::new()?;
    context.set_level(level)?;
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

fn checked(code: usize) -> Result<usize, ZstdError> {
    if ZSTD_isError(code) == 0 {
        return Ok(code);
    }
    let name = ZSTD_getErrorName(code);
    if name.is_null() {
        return Err(ZstdError::Library(format!("error {code}")));
    }
    #[allow(unsafe_code)]
    let name = unsafe { CStr::from_ptr(name) };
    Err(ZstdError::Library(name.to_string_lossy().into_owned()))
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
        let level: c_int = level.get();
        #[allow(unsafe_code)]
        let code = unsafe {
            ZSTD_CCtx_setParameter(
                self.0.as_ptr(),
                ZSTD_cParameter::ZSTD_c_compressionLevel,
                level,
            )
        };
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

    fn decompress(
        &self,
        frame: &[u8],
        prefix: &[u8],
        expected: usize,
    ) -> Result<Vec<u8>, ZstdError> {
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
