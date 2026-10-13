use std::{path::PathBuf, sync::Arc};

use caditor_document::Picture;
use caditor_file::{PictureReadError, read_picture_file};
use caditor_render::PictureTexture;

use super::{Event, Files, Purpose};
use crate::{
    editing::{EditingCommand, SketchEditing},
    image_textures,
    image_tools::{self, ImageTarget},
    model::{Action, Model, Notice, display_name},
};

pub const PICTURE_KIND: &str = "Pictures";

#[derive(Debug)]
pub struct ReadImage {
    pub picture: Arc<Picture>,
    pub texture: Result<Arc<PictureTexture>, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ImageReadFailure {
    #[error("{0}")]
    Read(#[from] PictureReadError),
    #[error("reading it stopped unexpectedly")]
    Crashed,
}

impl Files {
    pub(super) fn insert_image(&mut self, target: ImageTarget, model: &Model) {
        if self.pick(Purpose::Picture, model) {
            self.image_target = Some(target);
        }
    }

    pub(super) fn read_picture(&mut self, path: PathBuf, model: &Model) {
        let Some(target) = self.image_target.take() else {
            return;
        };
        let session = model.session();
        let failed = (path.clone(), target.clone());
        self.spawn_own(
            "import",
            move || Event::PictureRead {
                result: read_picture_file(&path)
                    .map(|read| ReadImage {
                        picture: read.picture,
                        texture: image_textures::texture_of(read.pixels),
                    })
                    .map_err(ImageReadFailure::Read),
                path,
                target,
                session,
            },
            move || Event::PictureRead {
                path: failed.0,
                target: failed.1,
                session,
                result: Err(ImageReadFailure::Crashed),
            },
        );
    }

    pub(super) fn picture_read(
        &mut self,
        (path, target, session): (PathBuf, ImageTarget, u64),
        result: Result<ReadImage, ImageReadFailure>,
        model: &mut Model,
        editing: &mut SketchEditing,
    ) {
        if session != model.session() {
            return;
        }
        let name = display_name(Some(&path));
        let read = match result {
            Ok(read) => read,
            Err(error) => {
                model.set_notice(Notice::failure(format!(
                    "Could not insert the picture “{name}”: {error}."
                )));
                return;
            }
        };
        let digest = read.picture.digest();
        match read.texture {
            Ok(texture) => model.pictures_mut().keep(digest, texture),
            Err(reason) => log::warn!("the picture “{name}” cannot be shown: {reason}"),
        }
        let (transaction, feature) = image_tools::create(model, &target, read.picture, &path);
        let before = model.revision();
        model.perform(Action::Apply(transaction));
        if model.revision() == before {
            return;
        }
        if editing.feature().is_none() {
            editing.perform(EditingCommand::OpenSolid(feature), model);
        }
        let feature_name = model
            .document()
            .feature(feature)
            .map_or_else(|| name.clone(), |feature| feature.name.clone());
        model.set_notice(Notice::success(format!(
            "Inserted “{name}” as {feature_name}; calibrate it or type its width to size it."
        )));
    }
}
