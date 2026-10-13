use std::fmt;

use thiserror::Error;

pub const MAX_PICTURE_BYTES: usize = 64 << 20;
pub const MAX_PICTURE_SIDE: u32 = 16_384;
pub const MAX_PICTURE_PIXELS: u64 = 40_000_000;

const DIGEST_BYTES: usize = 32;
const PLACEHOLDER_QOI: &[u8] =
    b"qoif\0\0\0\x01\0\0\0\x01\x03\x00\xfe\x80\x80\x80\0\0\0\0\0\0\0\x01";
const TGA_TYPES: [u8; 6] = [1, 2, 3, 9, 10, 11];
const TGA_DEPTHS: [u8; 5] = [8, 15, 16, 24, 32];
const TGA_HEADER_BYTES: usize = 18;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PictureFormat {
    Png,
    Jpeg,
    WebP,
    JpegXl,
    Gif,
    Bmp,
    Tiff,
    Qoi,
    Tga,
}

impl PictureFormat {
    pub const ALL: [Self; 9] = [
        Self::Png,
        Self::Jpeg,
        Self::WebP,
        Self::JpegXl,
        Self::Gif,
        Self::Bmp,
        Self::Tiff,
        Self::Qoi,
        Self::Tga,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
            Self::WebP => "WebP",
            Self::JpegXl => "JPEG XL",
            Self::Gif => "GIF",
            Self::Bmp => "BMP",
            Self::Tiff => "TIFF",
            Self::Qoi => "QOI",
            Self::Tga => "TGA",
        }
    }

    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Png => &["png"],
            Self::Jpeg => &["jpg", "jpeg", "jpe", "jfif"],
            Self::WebP => &["webp"],
            Self::JpegXl => &["jxl"],
            Self::Gif => &["gif"],
            Self::Bmp => &["bmp", "dib"],
            Self::Tiff => &["tif", "tiff"],
            Self::Qoi => &["qoi"],
            Self::Tga => &["tga", "icb", "vda", "vst"],
        }
    }

    pub fn extensions_of_all() -> Vec<&'static str> {
        Self::ALL
            .iter()
            .flat_map(|format| format.extensions().iter().copied())
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedPicture {
    Avif,
    Heic,
}

impl UnsupportedPicture {
    pub fn name(self) -> &'static str {
        match self {
            Self::Avif => "AVIF",
            Self::Heic => "HEIC",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sniffed {
    Supported(PictureFormat),
    Unsupported(UnsupportedPicture),
    Unknown,
}

pub fn sniff_picture(bytes: &[u8]) -> Sniffed {
    let starts = |magic: &[u8]| bytes.starts_with(magic);
    if starts(b"\x89PNG\r\n\x1a\n") {
        return Sniffed::Supported(PictureFormat::Png);
    }
    if starts(&[0xff, 0xd8, 0xff]) {
        return Sniffed::Supported(PictureFormat::Jpeg);
    }
    if starts(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        return Sniffed::Supported(PictureFormat::WebP);
    }
    if starts(&[0xff, 0x0a]) || starts(b"\0\0\0\x0cJXL \r\n\x87\n") {
        return Sniffed::Supported(PictureFormat::JpegXl);
    }
    if starts(b"GIF87a") || starts(b"GIF89a") {
        return Sniffed::Supported(PictureFormat::Gif);
    }
    if starts(b"BM") && bytes.len() >= 26 {
        return Sniffed::Supported(PictureFormat::Bmp);
    }
    if starts(b"II*\0") || starts(b"MM\0*") || starts(b"II+\0") || starts(b"MM\0+") {
        return Sniffed::Supported(PictureFormat::Tiff);
    }
    if starts(b"qoif") {
        return Sniffed::Supported(PictureFormat::Qoi);
    }
    if let Some(unsupported) = iso_media(bytes) {
        return Sniffed::Unsupported(unsupported);
    }
    if looks_like_tga(bytes) {
        return Sniffed::Supported(PictureFormat::Tga);
    }
    Sniffed::Unknown
}

fn iso_media(bytes: &[u8]) -> Option<UnsupportedPicture> {
    if bytes.get(4..8) != Some(b"ftyp") {
        return None;
    }
    let brands = bytes.get(8..12)?;
    match brands {
        b"avif" | b"avis" => Some(UnsupportedPicture::Avif),
        b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" | b"mif1" | b"msf1" => {
            Some(UnsupportedPicture::Heic)
        }
        _ => None,
    }
}

fn looks_like_tga(bytes: &[u8]) -> bool {
    let Some(header) = bytes.get(..TGA_HEADER_BYTES) else {
        return false;
    };
    let byte = |index: usize| header.get(index).copied().unwrap_or_default();
    let word = |index: usize| u16::from_le_bytes([byte(index), byte(index + 1)]);
    let colour_map = byte(1);
    let kind = byte(2);
    let depth = byte(16);
    colour_map <= 1
        && TGA_TYPES.contains(&kind)
        && word(12) > 0
        && word(14) > 0
        && TGA_DEPTHS.contains(&depth)
        && (colour_map == 1) == matches!(kind, 1 | 9)
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PictureDigest([u8; DIGEST_BYTES]);

impl PictureDigest {
    pub fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    pub fn hex(&self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    pub fn from_hex(text: &str) -> Option<Self> {
        if text.len() != DIGEST_BYTES * 2 || !text.is_ascii() {
            return None;
        }
        let mut digest = [0; DIGEST_BYTES];
        for (index, byte) in digest.iter_mut().enumerate() {
            let pair = text.get(index * 2..index * 2 + 2)?;
            *byte = u8::from_str_radix(pair, 16).ok()?;
        }
        Some(Self(digest))
    }
}

impl fmt::Debug for PictureDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "PictureDigest({})", self.hex())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PixelSize {
    pub width: u32,
    pub height: u32,
}

impl PixelSize {
    pub fn checked(width: u32, height: u32) -> Result<Self, PictureError> {
        if width == 0 || height == 0 {
            return Err(PictureError::NoPixels);
        }
        if width > MAX_PICTURE_SIDE || height > MAX_PICTURE_SIDE {
            return Err(PictureError::TooWide { width, height });
        }
        if u64::from(width) * u64::from(height) > MAX_PICTURE_PIXELS {
            return Err(PictureError::TooManyPixels { width, height });
        }
        Ok(Self { width, height })
    }

    pub fn aspect(self) -> f64 {
        f64::from(self.height) / f64::from(self.width)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PictureError {
    #[error("it is empty")]
    Empty,
    #[error("it is larger than {} MB", MAX_PICTURE_BYTES >> 20)]
    TooLarge,
    #[error("it is not a picture caditor can read")]
    UnknownFormat,
    #[error("it is an {} picture, which caditor cannot read yet; save it as PNG or JPEG", .0.name())]
    Unsupported(UnsupportedPicture),
    #[error("it holds no pixels")]
    NoPixels,
    #[error("it is {width} by {height} pixels, more than {MAX_PICTURE_SIDE} on a side")]
    TooWide { width: u32, height: u32 },
    #[error("it is {width} by {height} pixels, more than {} million in all", MAX_PICTURE_PIXELS / 1_000_000)]
    TooManyPixels { width: u32, height: u32 },
}

pub struct Picture {
    bytes: Box<[u8]>,
    digest: PictureDigest,
    format: PictureFormat,
    size: PixelSize,
}

impl Picture {
    pub fn new(bytes: Vec<u8>, size: PixelSize) -> Result<Self, PictureError> {
        let format = picture_format(&bytes)?;
        let size = PixelSize::checked(size.width, size.height)?;
        Ok(Self {
            digest: PictureDigest::of(&bytes),
            bytes: bytes.into_boxed_slice(),
            format,
            size,
        })
    }

    pub fn placeholder() -> Self {
        Self {
            bytes: PLACEHOLDER_QOI.into(),
            digest: PictureDigest::of(PLACEHOLDER_QOI),
            format: PictureFormat::Qoi,
            size: PixelSize {
                width: 1,
                height: 1,
            },
        }
    }

    pub fn is_placeholder(&self) -> bool {
        self.digest == PictureDigest::of(PLACEHOLDER_QOI)
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn digest(&self) -> PictureDigest {
        self.digest
    }

    pub fn format(&self) -> PictureFormat {
        self.format
    }

    pub fn size(&self) -> PixelSize {
        self.size
    }

    pub fn held_bytes(&self) -> usize {
        size_of::<Self>() + self.bytes.len()
    }
}

pub fn picture_format(bytes: &[u8]) -> Result<PictureFormat, PictureError> {
    if bytes.is_empty() {
        return Err(PictureError::Empty);
    }
    if bytes.len() > MAX_PICTURE_BYTES {
        return Err(PictureError::TooLarge);
    }
    match sniff_picture(bytes) {
        Sniffed::Supported(format) => Ok(format),
        Sniffed::Unsupported(format) => Err(PictureError::Unsupported(format)),
        Sniffed::Unknown => Err(PictureError::UnknownFormat),
    }
}

impl PartialEq for Picture {
    fn eq(&self, other: &Self) -> bool {
        self.digest == other.digest && self.size == other.size
    }
}

impl fmt::Debug for Picture {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Picture")
            .field("format", &self.format)
            .field("size", &self.size)
            .field("bytes", &self.bytes.len())
            .field("digest", &self.digest)
            .finish()
    }
}
