use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

const ICON_SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];
const ICON_HEADER_BYTES: usize = 6;
const ICON_ENTRY_BYTES: usize = 16;
const ICON_TYPE: u16 = 1;
const ICON_PLANES: u16 = 1;
const ICON_BITS: u16 = 32;
const FULL_SIZE: u32 = 256;

fn main() -> io::Result<()> {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return Ok(());
    }
    let packaging = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging");
    let output = PathBuf::from(env::var_os("OUT_DIR").ok_or_else(|| missing("OUT_DIR"))?);
    let icon = output.join("caditor.ico");
    fs::write(&icon, icon_file(&packaging.join("icons"))?)?;
    let manifest = packaging.join("windows/caditor.exe.manifest");
    println!("cargo:rerun-if-changed={}", manifest.display());

    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon(&text(&icon)?)
        .set_manifest_file(&text(&manifest)?)
        .set("FileDescription", "caditor")
        .set("ProductName", "caditor")
        .set(
            "LegalCopyright",
            "GNU Affero General Public License, version 3 only",
        );
    resource.compile()
}

fn icon_file(icons: &Path) -> io::Result<Vec<u8>> {
    let mut images = Vec::new();
    for size in ICON_SIZES {
        let path = icons.join(format!("caditor-{size}.png"));
        println!("cargo:rerun-if-changed={}", path.display());
        images.push((size, fs::read(&path)?));
    }
    let count = u16::try_from(images.len()).map_err(io::Error::other)?;
    let mut file = Vec::new();
    file.extend_from_slice(&0u16.to_le_bytes());
    file.extend_from_slice(&ICON_TYPE.to_le_bytes());
    file.extend_from_slice(&count.to_le_bytes());
    let mut offset = ICON_HEADER_BYTES + ICON_ENTRY_BYTES * images.len();
    for (size, png) in &images {
        let side = if *size >= FULL_SIZE {
            0
        } else {
            u8::try_from(*size).map_err(io::Error::other)?
        };
        file.extend_from_slice(&[side, side, 0, 0]);
        file.extend_from_slice(&ICON_PLANES.to_le_bytes());
        file.extend_from_slice(&ICON_BITS.to_le_bytes());
        file.extend_from_slice(
            &u32::try_from(png.len())
                .map_err(io::Error::other)?
                .to_le_bytes(),
        );
        file.extend_from_slice(
            &u32::try_from(offset)
                .map_err(io::Error::other)?
                .to_le_bytes(),
        );
        offset += png.len();
    }
    for (_, png) in images {
        file.extend_from_slice(&png);
    }
    Ok(file)
}

fn text(path: &Path) -> io::Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| io::Error::other(format!("{} is not UTF-8", path.display())))
}

fn missing(variable: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("cargo did not set {variable}"),
    )
}
