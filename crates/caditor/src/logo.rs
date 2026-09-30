use std::io::Cursor;

use egui::{
    ColorImage, Id, Image, Response, Sense, TextureHandle, TextureOptions, Ui, Vec2,
    load::SizedTexture,
};
use png::{BitDepth, ColorType, Decoder};
use thiserror::Error;
use winit::window::{BadIcon, Icon};

use crate::widgets;

#[derive(Clone, Copy)]
struct Render {
    size: u32,
    png: &'static [u8],
}

const SMALL: Render = Render {
    size: 32,
    png: include_bytes!("../../../packaging/icons/caditor-32.png"),
};
const MEDIUM: Render = Render {
    size: 64,
    png: include_bytes!("../../../packaging/icons/caditor-64.png"),
};
const LARGE: Render = Render {
    size: 128,
    png: include_bytes!("../../../packaging/icons/caditor-128.png"),
};
const LARGEST: Render = Render {
    size: 256,
    png: include_bytes!("../../../packaging/icons/caditor-256.png"),
};
const RENDERS: [Render; 4] = [SMALL, MEDIUM, LARGE, LARGEST];
const WINDOW_ICON: Render = LARGE;

#[derive(Debug, Error)]
pub enum LogoError {
    #[error("the {size} px logo is not a readable PNG: {source}")]
    Decode {
        size: u32,
        source: png::DecodingError,
    },
    #[error("the {size} px logo is {color:?} at {depth:?} instead of 8-bit RGBA")]
    NotRgba {
        size: u32,
        color: ColorType,
        depth: BitDepth,
    },
    #[error("the {size} px logo is {width}×{height} pixels")]
    WrongSize { size: u32, width: u32, height: u32 },
    #[error("the {size} px logo is too large to decode")]
    TooLarge { size: u32 },
    #[error("the window icon was refused: {0}")]
    Icon(#[from] BadIcon),
}

struct Logo {
    size: u32,
    rgba: Vec<u8>,
}

fn decode(render: Render) -> Result<Logo, LogoError> {
    let size = render.size;
    let unreadable = |source| LogoError::Decode { size, source };

    let mut reader = Decoder::new(Cursor::new(render.png))
        .read_info()
        .map_err(unreadable)?;
    let (color, depth) = reader.output_color_type();
    let (width, height) = (reader.info().width, reader.info().height);

    if (color, depth) != (ColorType::Rgba, BitDepth::Eight) {
        return Err(LogoError::NotRgba { size, color, depth });
    }
    if (width, height) != (size, size) {
        return Err(LogoError::WrongSize {
            size,
            width,
            height,
        });
    }

    let length = reader
        .output_buffer_size()
        .ok_or(LogoError::TooLarge { size })?;
    let mut rgba = vec![0; length];
    reader.next_frame(&mut rgba).map_err(unreadable)?;
    Ok(Logo { size, rgba })
}

fn render_for(pixels: f32) -> Render {
    RENDERS
        .into_iter()
        .find(|render| render.size as f32 >= pixels)
        .unwrap_or(LARGEST)
}

pub fn window_icon() -> Result<Icon, LogoError> {
    let logo = decode(WINDOW_ICON)?;
    Ok(Icon::from_rgba(logo.rgba, logo.size, logo.size)?)
}

pub fn texture_name(size: u32) -> String {
    format!("caditor-logo-{size}")
}

fn texture(ui: &Ui, render: Render) -> Option<TextureHandle> {
    let id = Id::new(("caditor-logo", render.size));
    if let Some(cached) = ui.data(|data| data.get_temp::<Option<TextureHandle>>(id)) {
        return cached;
    }

    let texture = match decode(render) {
        Ok(logo) => {
            let side = logo.size as usize;
            let image = ColorImage::from_rgba_unmultiplied([side, side], &logo.rgba);
            Some(
                ui.ctx()
                    .load_texture(texture_name(render.size), image, TextureOptions::LINEAR),
            )
        }
        Err(error) => {
            log::warn!("showing no logo: {error}");
            None
        }
    };
    ui.data_mut(|data| data.insert_temp(id, texture.clone()));
    texture
}

pub fn show(ui: &mut Ui, side: f32) -> Response {
    let render = render_for(side * ui.pixels_per_point());
    let size = Vec2::splat(side);

    let response = match texture(ui, render) {
        Some(texture) => ui.add(Image::new(SizedTexture::new(texture.id(), size))),
        None => ui.allocate_response(size, Sense::hover()),
    };
    widgets::decorative(ui, &response);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: &str = include_str!("../../../packaging/caditor.svg");

    #[test]
    fn every_embedded_render_is_square_rgba_at_its_size() {
        for render in RENDERS {
            let logo = decode(render).unwrap();
            let (pixels, rest) = logo.rgba.as_chunks::<4>();

            assert_eq!(logo.size, render.size);
            assert_eq!(logo.rgba.len(), (render.size * render.size * 4) as usize);
            assert!(rest.is_empty());
            assert!(pixels.iter().any(|[.., alpha]| *alpha == 0));
            assert!(pixels.iter().any(|[.., alpha]| *alpha == 255));
        }
    }

    #[test]
    fn the_window_icon_is_accepted() {
        assert!(window_icon().is_ok());
    }

    #[test]
    fn the_smallest_render_covering_the_pixels_is_chosen() {
        assert_eq!(render_for(20.0).size, 32);
        assert_eq!(render_for(32.0).size, 32);
        assert_eq!(render_for(40.0).size, 64);
        assert_eq!(render_for(128.0).size, 128);
        assert_eq!(render_for(1000.0).size, 256);
    }

    #[test]
    fn the_renders_come_from_the_square_scalable_logo() {
        assert!(SVG.contains("width=\"128\" height=\"128\" viewBox=\"0 0 128 128\""));
    }
}
