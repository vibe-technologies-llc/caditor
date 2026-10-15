use std::{ops::RangeInclusive, path::Path};

use egui::{Color32, Slider, Ui};

use crate::{
    appearance::{SPACE_M, SPACE_S},
    backdrop::Status,
    commands::Command,
    display_style::DisplayStyle,
    guide::Page,
    icons,
    layout::RightPanel,
    lighting::{self, LightFrame, Lighting},
    model::display_name,
    preferences::{self, PreferenceChange, PreferencesCommand},
    widgets::{self, PanelTitle, Tone},
};

pub const TITLE: &str = "Lighting";
pub const CLOSE: &str = "Close the lighting";
pub const ABOUT: &str = "The Rendered style lights bodies by their colour, finish and material \
                         with one sun, the shadows it casts, light it bounces off nearby bodies \
                         and a blue sky around them, darker where surfaces crowd in. It never \
                         changes the model.";
pub const OTHER_STYLE: &str = "These settings show only in the Rendered style.";
pub const SHOW_RENDERED: &str = "Show the Rendered style";
pub const RESET: &str = "Reset lighting";
pub const FOLLOWS: &str = "Light follows";
pub const AZIMUTH: &str = "Turned";
pub const ELEVATION: &str = "Height";
pub const STRENGTH: &str = "Sun strength";
pub const TEMPERATURE: &str = "Colour temperature";
pub const ENVIRONMENT: &str = "Sky light";
pub const BOUNCE: &str = "Bounced light";
pub const OCCLUSION: &str = "Ambient occlusion";
pub const EXPOSURE: &str = "Exposure";
pub const SHADOWS: &str = "Cast shadows";
pub const SKY_PICTURE: &str = "Sky picture";
pub const SKY_PICTURE_ABOUT: &str = "A Radiance HDR picture (.hdr) of the surroundings, twice as \
                                     wide as it is high, drawn behind the model and used for the \
                                     sky light. Z is up and the middle of the picture lies along X.";
pub const PLAIN_SKY: &str = "The plain blue sky";
pub const CHOOSE_PICTURE: &str = "Choose picture…";
pub const USE_PLAIN_SKY: &str = "Use the plain sky";
pub const PICTURE_TURNED: &str = "Picture turned";
pub const PICTURE_BRIGHTNESS: &str = "Picture brightness";
const SHADOWS_HOVER: &str = "Bodies shade what lies behind them from the sun";
const PANEL: RightPanel = RightPanel { id: "lighting" };
const HEADER: PanelTitle<'static> = PanelTitle {
    glyph: icons::LIGHTING,
    title: TITLE,
    close: CLOSE,
    help: Some(Page::RenderedView),
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LightingTool {
    pub open: bool,
}

impl LightingTool {
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SkyPicture<'a> {
    pub path: Option<&'a Path>,
    pub status: &'a Status,
}

#[derive(Debug, Default)]
pub struct Asked {
    pub command: Option<PreferencesCommand>,
    pub run: Option<Command>,
}

type Value = fn(&mut Lighting) -> &mut f64;

struct Setting {
    caption: &'static str,
    range: RangeInclusive<f64>,
    suffix: &'static str,
    decimals: usize,
    value: Value,
}

fn slider(
    ui: &mut Ui,
    setting: &Setting,
    lighting: Lighting,
    command: &mut Option<PreferencesCommand>,
) {
    widgets::property(ui, setting.caption, |ui| {
        let mut changed = lighting;
        let response = widgets::slider(
            ui,
            Slider::new((setting.value)(&mut changed), setting.range.clone())
                .suffix(setting.suffix)
                .fixed_decimals(setting.decimals),
        );
        widgets::tie_to_caption(ui, &response);
        let settled = response.drag_stopped() || !response.dragged();
        if settled && (response.changed() || response.drag_stopped()) {
            preferences::change(command, PreferenceChange::Lighting(changed));
        } else if response.changed() {
            *command = Some(PreferencesCommand::Preview(PreferenceChange::Lighting(
                changed,
            )));
        }
    });
}

pub fn rows(ui: &mut Ui, lighting: Lighting, command: &mut Option<PreferencesCommand>) {
    widgets::property(ui, FOLLOWS, |ui| {
        if let Some(frame) = preferences::pick::<LightFrame>(ui, lighting.frame) {
            preferences::change(
                command,
                PreferenceChange::Lighting(Lighting { frame, ..lighting }),
            );
        }
    });
    let settings: [Setting; 8] = [
        Setting {
            caption: AZIMUTH,
            range: lighting::AZIMUTH,
            suffix: "°",
            decimals: 0,
            value: |lighting| &mut lighting.azimuth,
        },
        Setting {
            caption: ELEVATION,
            range: lighting::ELEVATION,
            suffix: "°",
            decimals: 0,
            value: |lighting| &mut lighting.elevation,
        },
        Setting {
            caption: STRENGTH,
            range: lighting::STRENGTH,
            suffix: "",
            decimals: 1,
            value: |lighting| &mut lighting.strength,
        },
        Setting {
            caption: TEMPERATURE,
            range: lighting::TEMPERATURE,
            suffix: " K",
            decimals: 0,
            value: |lighting| &mut lighting.temperature,
        },
        Setting {
            caption: ENVIRONMENT,
            range: lighting::ENVIRONMENT,
            suffix: "",
            decimals: 2,
            value: |lighting| &mut lighting.environment,
        },
        Setting {
            caption: BOUNCE,
            range: lighting::BOUNCE,
            suffix: "",
            decimals: 2,
            value: |lighting| &mut lighting.bounce,
        },
        Setting {
            caption: OCCLUSION,
            range: lighting::OCCLUSION,
            suffix: "",
            decimals: 2,
            value: |lighting| &mut lighting.occlusion,
        },
        Setting {
            caption: EXPOSURE,
            range: lighting::EXPOSURE,
            suffix: " EV",
            decimals: 1,
            value: |lighting| &mut lighting.exposure,
        },
    ];
    for setting in &settings {
        slider(ui, setting, lighting, command);
        if setting.caption == TEMPERATURE {
            let [red, green, blue] = lighting.swatch();
            widgets::property(ui, "", |ui| {
                widgets::swatch(
                    ui,
                    Color32::from_rgb(red, green, blue),
                    &format!("Light colour at {:.0} K", lighting.temperature),
                    false,
                );
            });
        }
    }
    widgets::property(ui, "", |ui| {
        let mut shadows = lighting.shadows;
        if widgets::checkbox(ui, &mut shadows, SHADOWS)
            .on_hover_text(SHADOWS_HOVER)
            .changed()
        {
            preferences::change(
                command,
                PreferenceChange::Lighting(Lighting {
                    shadows,
                    ..lighting
                }),
            );
        }
    });
}

fn sky_picture(ui: &mut Ui, lighting: Lighting, sky: SkyPicture<'_>, asked: &mut Asked) {
    ui.label(widgets::section_title(SKY_PICTURE));
    ui.label(widgets::muted(SKY_PICTURE_ABOUT, ui));
    ui.add_space(SPACE_S);
    widgets::properties(ui, "sky-picture-properties", |ui| {
        widgets::property(ui, SKY_PICTURE, |ui| {
            let name = sky
                .path
                .map_or_else(|| PLAIN_SKY.to_owned(), |path| display_name(Some(path)));
            ui.label(name);
        });
        if matches!(sky.status, Status::Ready(..)) {
            for setting in [
                Setting {
                    caption: PICTURE_TURNED,
                    range: lighting::BACKDROP_TURN,
                    suffix: "°",
                    decimals: 0,
                    value: |lighting| &mut lighting.backdrop_turn,
                },
                Setting {
                    caption: PICTURE_BRIGHTNESS,
                    range: lighting::BACKDROP_EXPOSURE,
                    suffix: " EV",
                    decimals: 1,
                    value: |lighting| &mut lighting.backdrop_exposure,
                },
            ] {
                slider(ui, &setting, lighting, &mut asked.command);
            }
        }
    });
    match sky.status {
        Status::Loading(path) => {
            ui.label(widgets::muted(
                format!("Reading “{}”…", display_name(Some(path))),
                ui,
            ));
        }
        Status::Failed(path, failure) => {
            widgets::callout(ui, Tone::Error, |ui| {
                ui.label(format!(
                    "Could not use “{}” as the sky: {failure}. The plain sky is drawn instead.",
                    display_name(Some(path)),
                ));
            });
        }
        Status::Plain | Status::Ready(..) => {}
    }
    ui.add_space(SPACE_S);
    ui.horizontal_wrapped(|ui| {
        let choose = widgets::small_button(ui, icons::SKY_PICTURE, CHOOSE_PICTURE);
        if ui.add(choose).clicked() {
            asked.run = Some(Command::ChooseBackdrop);
        }
        if sky.path.is_some() {
            let plain = widgets::small_button(ui, icons::RESET_SECTION, USE_PLAIN_SKY);
            if ui.add(plain).clicked() {
                asked.run = Some(Command::ClearBackdrop);
            }
        }
    });
}

pub fn show(
    ui: &mut Ui,
    (lighting, sky): (Lighting, SkyPicture<'_>),
    style: DisplayStyle,
    room: f32,
    tool: &mut LightingTool,
) -> Asked {
    let mut asked = Asked::default();
    let panel = widgets::side_panel(
        ui,
        room,
        PANEL,
        &HEADER,
        |_| {},
        |ui| {
            ui.label(widgets::muted(ABOUT, ui));
            ui.add_space(SPACE_S);
            if !style.is_rendered() {
                widgets::callout(ui, Tone::Info, |ui| {
                    ui.label(OTHER_STYLE);
                    let button = widgets::small_button(ui, icons::LIGHTING, SHOW_RENDERED);
                    if ui.add(button).clicked() {
                        asked.run = Some(Command::Style(DisplayStyle::Rendered));
                    }
                });
                ui.add_space(SPACE_S);
            }
            widgets::properties(ui, "lighting-properties", |ui| {
                rows(ui, lighting, &mut asked.command);
            });
            ui.add_space(SPACE_M);
            sky_picture(ui, lighting, sky, &mut asked);
            ui.add_space(SPACE_M);
            let reset = widgets::small_button(ui, icons::RESET_SECTION, RESET);
            if ui.add(reset).clicked() {
                asked.run = Some(Command::ResetLighting);
            }
        },
    );
    if panel.closed {
        tool.toggle();
    }
    asked
}
