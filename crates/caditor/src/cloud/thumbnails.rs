use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    time::SystemTime,
};

use caditor_cloud::{Client, Design, Service};
use caditor_file::{decode_picture, write_atomically};
use egui::{ColorImage, Context, TextureHandle, TextureOptions};

use super::failure::Failure;

pub const KEPT_THUMBNAILS: usize = 400;
pub const MAX_THUMBNAIL_BYTES: usize = 1 << 20;
pub const AT_ONCE: usize = 3;
const EXTENSION: &str = "png";
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;

pub struct Thumbnail {
    image: Arc<ColorImage>,
    texture: OnceLock<TextureHandle>,
}

impl std::fmt::Debug for Thumbnail {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Thumbnail")
            .field("size", &self.image.size)
            .field("uploaded", &self.texture.get().is_some())
            .finish()
    }
}

impl Thumbnail {
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let pixels = decode_picture(bytes).ok()?;
        let size = [
            usize::try_from(pixels.width).ok()?,
            usize::try_from(pixels.height).ok()?,
        ];
        Some(Self {
            image: Arc::new(ColorImage::from_rgba_unmultiplied(size, &pixels.rgba)),
            texture: OnceLock::new(),
        })
    }

    pub fn texture(&self, context: &Context, name: &str) -> &TextureHandle {
        self.texture.get_or_init(|| {
            context.load_texture(
                name,
                egui::ImageData::Color(Arc::clone(&self.image)),
                TextureOptions::LINEAR,
            )
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbnailCache {
    dir: PathBuf,
}

impl ThumbnailCache {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path_for(&self, service: Service, design: &Design) -> PathBuf {
        let modified = design.modified.map_or(0, |at| at.as_second());
        let identity = [
            service.id().as_bytes(),
            design.project.as_str().as_bytes(),
            design.id.as_str().as_bytes(),
            modified.to_string().as_bytes(),
        ]
        .into_iter()
        .flat_map(|part| part.iter().copied().chain(std::iter::once(0)))
        .fold(FNV_OFFSET, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME)
        });
        self.dir.join(format!("{identity:016x}.{EXTENSION}"))
    }

    pub fn read(&self, service: Service, design: &Design) -> Option<Vec<u8>> {
        fs::read(self.path_for(service, design)).ok()
    }

    pub fn write(&self, service: Service, design: &Design, bytes: &[u8]) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        write_atomically(&self.path_for(service, design), bytes)?;
        self.prune(KEPT_THUMBNAILS)
    }

    pub fn discard(&self, service: Service, design: &Design) {
        let _ = fs::remove_file(self.path_for(service, design));
    }

    fn prune(&self, keep: usize) -> io::Result<()> {
        let mut kept: Vec<(Option<SystemTime>, PathBuf)> = fs::read_dir(&self.dir)?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == EXTENSION))
            .map(|entry| {
                (
                    entry.metadata().and_then(|meta| meta.modified()).ok(),
                    entry.path(),
                )
            })
            .collect();
        kept.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
        for (_, stale) in kept.into_iter().skip(keep) {
            fs::remove_file(stale)?;
        }
        Ok(())
    }
}

pub fn forget(dir: &Path) -> io::Result<()> {
    match fs::remove_dir_all(dir) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

pub fn load(
    client: &Client,
    cache: Option<&ThumbnailCache>,
    service: Service,
    design: &Design,
) -> Result<Option<Arc<Thumbnail>>, Failure> {
    if let Some(cache) = cache
        && let Some(bytes) = cache.read(service, design)
    {
        match Thumbnail::from_bytes(&bytes) {
            Some(thumbnail) => return Ok(Some(Arc::new(thumbnail))),
            None => cache.discard(service, design),
        }
    }
    let Some(bytes) = client.thumbnail(design)? else {
        return Ok(None);
    };
    if bytes.len() > MAX_THUMBNAIL_BYTES {
        return Ok(None);
    }
    let Some(thumbnail) = Thumbnail::from_bytes(&bytes) else {
        log::info!(
            "{service} sent a picture of “{}” caditor cannot read",
            design.name
        );
        return Ok(None);
    };
    if let Some(cache) = cache
        && let Err(error) = cache.write(service, design, &bytes)
    {
        log::warn!(
            "the picture of “{}” could not be kept: {error}",
            design.name
        );
    }
    Ok(Some(Arc::new(thumbnail)))
}

#[cfg(test)]
struct Flat {
    side: u32,
    rgba: Vec<u8>,
    given: bool,
}

#[cfg(test)]
impl caditor_file::PixelRows for Flat {
    type Error = std::convert::Infallible;

    fn width(&self) -> u32 {
        self.side
    }

    fn height(&self) -> u32 {
        self.side
    }

    fn next_rows(&mut self) -> Option<Result<&[u8], std::convert::Infallible>> {
        if std::mem::replace(&mut self.given, true) {
            return None;
        }
        Some(Ok(&self.rgba))
    }
}

#[cfg(test)]
pub fn sample_png(side: u32) -> Vec<u8> {
    let texels = usize::try_from(side * side).unwrap();
    let mut flat = Flat {
        side,
        rgba: [200, 30, 30, 255].repeat(texels),
        given: false,
    };
    caditor_file::png_bytes(&mut flat, &caditor_document::CancelToken::never()).unwrap()
}

#[cfg(test)]
mod tests {
    use caditor_cloud::{
        DesignId, ProjectId,
        fake::{FakeProvider, ScriptedTransport, Step},
    };
    use jiff::Timestamp;
    use tempfile::TempDir;

    use super::*;

    fn design(id: &str, modified: i64) -> Design {
        Design {
            id: DesignId::new(id),
            project: ProjectId::new("p"),
            name: "Frame".to_owned(),
            modified: Timestamp::from_second(modified).ok(),
        }
    }

    fn client(provider: FakeProvider) -> (Client, Arc<FakeProvider>) {
        let provider = Arc::new(provider);
        let client = Client::new(
            Arc::clone(&provider) as Arc<dyn caditor_cloud::Provider>,
            Arc::new(ScriptedTransport::new()),
            caditor_cloud::Session {
                access_token: "a".to_owned(),
                refresh_token: Some("r".to_owned()),
                expires_at: u64::MAX,
            },
        );
        (client, provider)
    }

    fn thumbnail_calls(provider: &FakeProvider) -> usize {
        provider
            .calls()
            .iter()
            .filter(|call| call.starts_with("thumbnail"))
            .count()
    }

    #[test]
    fn a_decoded_picture_keeps_its_size() {
        let thumbnail = Thumbnail::from_bytes(&sample_png(4)).unwrap();

        assert_eq!(thumbnail.image.size, [4, 4]);
    }

    #[test]
    fn bytes_that_are_no_picture_make_no_thumbnail() {
        assert!(Thumbnail::from_bytes(b"not a picture").is_none());
        assert!(Thumbnail::from_bytes(b"").is_none());
    }

    #[test]
    fn a_fetched_picture_is_kept_and_the_next_load_needs_no_request() {
        let dir = TempDir::new().unwrap();
        let cache = ThumbnailCache::new(dir.path().join("thumbnails"));
        let frame = design("d1", 100);
        let (client, provider) =
            client(FakeProvider::new().thumbnail(frame.id.clone(), &sample_png(4)));

        let first = load(&client, Some(&cache), Service::Fusion, &frame).unwrap();
        let second = load(&client, Some(&cache), Service::Fusion, &frame).unwrap();

        assert!(first.is_some());
        assert!(second.is_some());
        assert_eq!(thumbnail_calls(&provider), 1);
    }

    #[test]
    fn a_newer_version_of_a_design_is_fetched_again() {
        let dir = TempDir::new().unwrap();
        let cache = ThumbnailCache::new(dir.path().join("thumbnails"));
        let (client, provider) =
            client(FakeProvider::new().thumbnail(DesignId::new("d1"), &sample_png(4)));

        load(&client, Some(&cache), Service::Fusion, &design("d1", 100)).unwrap();
        load(&client, Some(&cache), Service::Fusion, &design("d1", 200)).unwrap();

        assert_eq!(thumbnail_calls(&provider), 2);
    }

    #[test]
    fn a_damaged_cache_file_is_replaced_by_a_fresh_request() {
        let dir = TempDir::new().unwrap();
        let cache = ThumbnailCache::new(dir.path().join("thumbnails"));
        let frame = design("d1", 100);
        cache.write(Service::Fusion, &frame, b"damaged").unwrap();
        let (client, provider) =
            client(FakeProvider::new().thumbnail(frame.id.clone(), &sample_png(4)));

        let loaded = load(&client, Some(&cache), Service::Fusion, &frame).unwrap();

        assert!(loaded.is_some());
        assert_eq!(thumbnail_calls(&provider), 1);
        assert!(Thumbnail::from_bytes(&cache.read(Service::Fusion, &frame).unwrap()).is_some());
    }

    #[test]
    fn a_design_without_a_picture_loads_as_none_and_keeps_nothing() {
        let dir = TempDir::new().unwrap();
        let cache = ThumbnailCache::new(dir.path().join("thumbnails"));
        let frame = design("d1", 100);
        let (client, _) = client(FakeProvider::new());

        let loaded = load(&client, Some(&cache), Service::Fusion, &frame).unwrap();

        assert!(loaded.is_none());
        assert!(cache.read(Service::Fusion, &frame).is_none());
    }

    #[test]
    fn an_unreadable_or_oversized_answer_is_no_picture() {
        let frame = design("d1", 100);
        let (garbled, _) =
            client(FakeProvider::new().thumbnail(frame.id.clone(), b"<html>no</html>"));
        let (huge, _) = client(
            FakeProvider::new().thumbnail(frame.id.clone(), &vec![0; MAX_THUMBNAIL_BYTES + 1]),
        );

        assert!(
            load(&garbled, None, Service::Fusion, &frame)
                .unwrap()
                .is_none()
        );
        assert!(
            load(&huge, None, Service::Fusion, &frame)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_failed_request_is_reported_and_keeps_nothing() {
        let dir = TempDir::new().unwrap();
        let cache = ThumbnailCache::new(dir.path().join("thumbnails"));
        let frame = design("d1", 100);
        let (client, _) = client(FakeProvider::new().failing(Step::Thumbnail, || {
            caditor_cloud::Error::Refused {
                service: Service::Fusion,
                status: 500,
            }
        }));

        assert!(load(&client, Some(&cache), Service::Fusion, &frame).is_err());
        assert!(cache.read(Service::Fusion, &frame).is_none());
    }

    #[test]
    fn only_the_newest_pictures_are_kept() {
        let dir = TempDir::new().unwrap();
        let cache = ThumbnailCache::new(dir.path().join("thumbnails"));
        for index in 0..5 {
            cache
                .write(Service::Fusion, &design(&format!("d{index}"), 1), b"x")
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        cache.prune(2).unwrap();

        let kept = fs::read_dir(dir.path().join("thumbnails")).unwrap().count();
        assert_eq!(kept, 2);
        assert!(cache.read(Service::Fusion, &design("d4", 1)).is_some());
        assert!(cache.read(Service::Fusion, &design("d0", 1)).is_none());
    }

    #[test]
    fn forgetting_removes_the_folder_and_tolerates_its_absence() {
        let dir = TempDir::new().unwrap();
        let cache = ThumbnailCache::new(dir.path().join("thumbnails"));
        cache
            .write(Service::Fusion, &design("d1", 1), b"x")
            .unwrap();

        forget(&dir.path().join("thumbnails")).unwrap();
        forget(&dir.path().join("thumbnails")).unwrap();

        assert!(!dir.path().join("thumbnails").exists());
    }

    #[test]
    fn services_and_designs_do_not_share_a_cache_file() {
        let cache = ThumbnailCache::new(PathBuf::from("cache"));

        let paths = [
            cache.path_for(Service::Fusion, &design("d1", 1)),
            cache.path_for(Service::Onshape, &design("d1", 1)),
            cache.path_for(Service::Fusion, &design("d2", 1)),
        ];

        assert_ne!(paths[0], paths[1]);
        assert_ne!(paths[0], paths[2]);
    }
}
