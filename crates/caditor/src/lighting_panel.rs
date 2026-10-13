use std::ops::RangeInclusive;

use egui::{Color32, Slider, Ui};

use crate::{
    appearance::{SPACE_M, SPACE_S},
    commands::Command,
    display_style::DisplayStyle,
    guide::Page,
    icons,
    layout::RightPanel,
    lighting::{self, LightFrame, Lighting},
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

pub fn show(
    ui: &mut Ui,
    lighting: Lighting,
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
