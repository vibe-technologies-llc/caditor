use std::time::{Duration, Instant};

use caditor_file::Settings;
use caditor_kernel::MeshQuality;
use caditor_render::{AdapterPreference, GraphicsInfo, GraphicsSettings, Machine, Msaa, Shading};
use egui::{Label, Ui};

use crate::{
    icons,
    preferences::{self, PreferenceChange, PreferencesCommand, Section, Shown},
    widgets::{self, Segment, Tone},
};

const VSYNC_KEY: &str = "graphics.vsync";
const FRAME_LIMIT_KEY: &str = "graphics.frame_limit";
const MSAA_KEY: &str = "graphics.msaa";
const SHADING_KEY: &str = "graphics.shading";
const CURVE_QUALITY_KEY: &str = "graphics.curve_quality";
const ADAPTER_KEY: &str = "graphics.adapter";
const LOWEST_DISPLAY_RATE: f64 = 1.0;
pub const COPY_DETAILS: &str = "Copy details";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FrameLimit {
    Unlimited,
    Fps(u32),
    #[default]
    Display,
}

impl FrameLimit {
    pub const MIN_FPS: u32 = 1;
    pub const MAX_FPS: u32 = 1000;
    pub const DEFAULT_FPS: u32 = 60;

    pub fn fps(rate: u32) -> Self {
        Self::Fps(rate.clamp(Self::MIN_FPS, Self::MAX_FPS))
    }

    pub fn rate(self) -> Option<u32> {
        match self {
            Self::Fps(rate) => Some(rate),
            Self::Unlimited | Self::Display => None,
        }
    }

    pub fn label(self) -> String {
        match self {
            Self::Fps(rate) => format!("{rate} fps"),
            Self::Display | Self::Unlimited => self.mode_label().to_owned(),
        }
    }

    pub fn mode_label(self) -> &'static str {
        match self {
            Self::Display => "Match the display",
            Self::Unlimited => "No limit",
            Self::Fps(_) => "Fixed rate",
        }
    }

    pub fn same_mode(self, other: Self) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&other)
    }

    pub fn modes() -> [Self; 3] {
        [Self::Display, Self::Unlimited, Self::Fps(Self::DEFAULT_FPS)]
    }

    fn key(self) -> String {
        match self {
            Self::Fps(rate) => rate.to_string(),
            Self::Display => "display".to_owned(),
            Self::Unlimited => "unlimited".to_owned(),
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        match key {
            "display" => Some(Self::Display),
            "unlimited" => Some(Self::Unlimited),
            _ => {
                let asked = key
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|rate| rate.is_finite())?;
                let clamped = asked
                    .round()
                    .clamp(f64::from(Self::MIN_FPS), f64::from(Self::MAX_FPS));
                Some(Self::fps(clamped as u32))
            }
        }
    }

    pub fn interval(self, vsync: bool, display_rate: Option<f64>) -> Option<Duration> {
        if vsync {
            return None;
        }
        let rate = match self {
            Self::Fps(rate) => f64::from(rate),
            Self::Display => usable_rate(display_rate)?,
            Self::Unlimited => return None,
        };
        Some(Duration::from_secs_f64(rate.recip()))
    }

    fn hover(self, display_rate: Option<f64>) -> String {
        match self {
            Self::Fps(_) => "Draw at most this many frames a second while the view moves; a \
                             lower limit saves power and keeps laptops cooler"
                .to_owned(),
            Self::Display => match usable_rate(display_rate) {
                Some(rate) => format!(
                    "Draw at most as many frames a second as the display shows ({rate:.0} Hz)"
                ),
                None => "Draw at most as many frames a second as the display shows. Its rate is \
                         not known here, so nothing limits it"
                    .to_owned(),
            },
            Self::Unlimited => {
                "Draw as often as possible while the view moves, for the least delay after a \
                 mouse move"
                    .to_owned()
            }
        }
    }
}

fn usable_rate(rate: Option<f64>) -> Option<f64> {
    rate.filter(|rate| rate.is_finite() && *rate >= LOWEST_DISPLAY_RATE)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CurveQuality {
    Coarse,
    #[default]
    Smooth,
}

impl CurveQuality {
    pub const ALL: [Self; 2] = [Self::Coarse, Self::Smooth];

    pub fn mesh_quality(self) -> MeshQuality {
        match self {
            Self::Coarse => MeshQuality::COARSE,
            Self::Smooth => MeshQuality::SMOOTH,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Coarse => "Coarse",
            Self::Smooth => "Smooth",
        }
    }

    fn hover(self) -> &'static str {
        match self {
            Self::Coarse => {
                "Fewer, larger facets on round faces and edges: quicker to prepare after an edit \
                 and lighter on memory for very large models"
            }
            Self::Smooth => {
                "Round faces and edges drawn with fine facets, so holes, fillets and revolved \
                 bodies look round"
            }
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Coarse => "coarse",
            Self::Smooth => "smooth",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|quality| quality.key() == key)
    }
}

pub fn adapter_label(adapter: AdapterPreference) -> &'static str {
    match adapter {
        AdapterPreference::Auto => "Auto",
        AdapterPreference::PowerSaving => "Power saving",
        AdapterPreference::Performance => "Performance",
    }
}

fn adapter_phrase(adapter: AdapterPreference) -> &'static str {
    match adapter {
        AdapterPreference::Auto => "automatic",
        AdapterPreference::PowerSaving => "power saving",
        AdapterPreference::Performance => "high performance",
    }
}

pub fn auto_summary(machine: Machine) -> String {
    format!(
        "Auto: {}, {}.",
        adapter_phrase(AdapterPreference::Auto.resolved(machine)),
        machine.reason()
    )
}

fn adapter_hover(adapter: AdapterPreference) -> &'static str {
    match adapter {
        AdapterPreference::Auto => {
            "Choose by the computer: the most powerful graphics card on a desktop, the \
             integrated graphics on a laptop or when caditor cannot tell"
        }
        AdapterPreference::PowerSaving => {
            "Prefer the integrated graphics when the computer has a choice: cooler, quieter and \
             kinder to a battery"
        }
        AdapterPreference::Performance => {
            "Prefer the most powerful graphics card: smoother with very large models, and \
             warmer"
        }
    }
}

fn adapter_key(adapter: AdapterPreference) -> &'static str {
    match adapter {
        AdapterPreference::Auto => "auto",
        AdapterPreference::PowerSaving => "power_saving",
        AdapterPreference::Performance => "performance",
    }
}

fn adapter_from_key(key: &str) -> Option<AdapterPreference> {
    AdapterPreference::ALL
        .into_iter()
        .find(|adapter| adapter_key(*adapter) == key)
}

pub fn msaa_label(msaa: Msaa) -> &'static str {
    match msaa {
        Msaa::Off => "Off",
        Msaa::X2 => "2×",
        Msaa::X4 => "4×",
        Msaa::X8 => "8×",
    }
}

fn msaa_hover(msaa: Msaa) -> String {
    match msaa {
        Msaa::Off => "Draw each pixel once: the quickest, with stepped edges".to_owned(),
        level => format!(
            "Smooth the edges of faces, lines and points with {} samples per pixel; more samples \
             look smoother and ask more of the graphics card",
            level.samples()
        ),
    }
}

fn msaa_refusal(msaa: Msaa) -> String {
    format!(
        "This graphics adapter cannot smooth edges with {} samples per pixel",
        msaa.samples()
    )
}

pub fn shading_label(shading: Shading) -> &'static str {
    match shading {
        Shading::Standard => "Standard",
        Shading::Enhanced => "Enhanced",
    }
}

fn shading_hover(shading: Shading) -> &'static str {
    match shading {
        Shading::Standard => "A key light and a headlight: even shading that is quickest to draw",
        Shading::Enhanced => {
            "Light from the sky and the ground, a key and a fill light, crisper highlights and a \
             soft rim, so the shape of curved faces reads more clearly"
        }
    }
}

fn shading_key(shading: Shading) -> &'static str {
    match shading {
        Shading::Standard => "standard",
        Shading::Enhanced => "enhanced",
    }
}

fn shading_from_key(key: &str) -> Option<Shading> {
    Shading::ALL
        .into_iter()
        .find(|shading| shading_key(*shading) == key)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Graphics {
    pub vsync: bool,
    pub frame_limit: FrameLimit,
    pub msaa: Msaa,
    pub shading: Shading,
    pub curves: CurveQuality,
    pub adapter: AdapterPreference,
}

impl Default for Graphics {
    fn default() -> Self {
        let render = GraphicsSettings::default();
        Self {
            vsync: render.vsync,
            frame_limit: FrameLimit::default(),
            msaa: render.msaa,
            shading: render.shading,
            curves: CurveQuality::default(),
            adapter: render.adapter,
        }
    }
}

impl Graphics {
    pub fn from_settings(raw: &Settings) -> Self {
        let defaults = Self::default();
        Self {
            vsync: raw.flag(VSYNC_KEY).unwrap_or(defaults.vsync),
            frame_limit: raw
                .text(FRAME_LIMIT_KEY)
                .map(str::to_owned)
                .or_else(|| raw.number(FRAME_LIMIT_KEY).map(|rate| rate.to_string()))
                .and_then(|key| FrameLimit::from_key(&key))
                .unwrap_or(defaults.frame_limit),
            msaa: raw
                .number(MSAA_KEY)
                .and_then(Msaa::at_most)
                .unwrap_or(defaults.msaa),
            shading: raw
                .text(SHADING_KEY)
                .and_then(shading_from_key)
                .unwrap_or(defaults.shading),
            curves: raw
                .text(CURVE_QUALITY_KEY)
                .and_then(CurveQuality::from_key)
                .unwrap_or(defaults.curves),
            adapter: raw
                .text(ADAPTER_KEY)
                .and_then(adapter_from_key)
                .unwrap_or(defaults.adapter),
        }
    }

    pub fn write(&self, settings: &mut Settings) {
        settings.set_flag(VSYNC_KEY, self.vsync);
        settings.set_text(FRAME_LIMIT_KEY, &self.frame_limit.key());
        settings.set_number(MSAA_KEY, f64::from(self.msaa.samples()));
        settings.set_text(SHADING_KEY, shading_key(self.shading));
        settings.set_text(CURVE_QUALITY_KEY, self.curves.key());
        settings.set_text(ADAPTER_KEY, adapter_key(self.adapter));
    }

    pub fn render(&self) -> GraphicsSettings {
        GraphicsSettings {
            vsync: self.vsync,
            msaa: self.msaa,
            shading: self.shading,
            adapter: self.adapter,
        }
    }

    pub fn frame_interval(&self, hardware: &Hardware) -> Option<Duration> {
        let vsync = hardware
            .adapter
            .as_ref()
            .map_or(self.vsync, |adapter| adapter.vsync);
        self.frame_limit.interval(vsync, hardware.refresh_rate)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Hardware {
    pub adapter: Option<GraphicsInfo>,
    pub refresh_rate: Option<f64>,
}

impl Hardware {
    fn details(&self, graphics: &Graphics) -> String {
        let mut lines = Vec::new();
        if let Some(adapter) = &self.adapter {
            lines.push(format!("Adapter: {}", adapter.adapter));
            lines.push(format!("Backend: {}", adapter.backend));
            lines.push(format!("Driver: {}", adapter.driver));
            lines.push(format!(
                "Anti-aliasing: {} (offered: {})",
                msaa_label(adapter.msaa),
                adapter
                    .msaa_offered
                    .iter()
                    .map(|level| msaa_label(*level))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            lines.push(format!(
                "Vsync: {}",
                if adapter.vsync { "on" } else { "off" }
            ));
        }
        if let Some(rate) = usable_rate(self.refresh_rate) {
            lines.push(format!("Display: {rate:.0} Hz"));
        }
        lines.push(format!(
            "Adapter choice: {}",
            adapter_label(graphics.adapter)
        ));
        lines.push(format!("Frame limit: {}", graphics.frame_limit.label()));
        lines.push(format!("Shading: {}", shading_label(graphics.shading)));
        lines.push(format!("Curve smoothness: {}", graphics.curves.label()));
        lines.join("\n")
    }
}

#[derive(Debug, Default)]
pub struct FramePacer {
    slot: Option<Instant>,
}

impl FramePacer {
    pub fn frame_started(&mut self, now: Instant, interval: Option<Duration>) {
        let on_cadence = self.slot.zip(interval).and_then(|(slot, interval)| {
            let due = slot.checked_add(interval)?;
            let late = due.checked_add(interval)?;
            (due <= now && now < late).then_some(due)
        });
        self.slot = Some(on_cadence.unwrap_or(now));
    }

    pub fn next_frame_at(&self, now: Instant, interval: Option<Duration>) -> Option<Instant> {
        let due = self.slot?.checked_add(interval?)?;
        (due > now).then_some(due)
    }
}

pub fn tab(
    ui: &mut Ui,
    shown: &Shown<'_>,
    hardware: &Hardware,
    command: &mut Option<PreferencesCommand>,
) {
    display(ui, shown, hardware, command);
    quality(ui, shown, hardware, command);
    adapter(ui, shown, hardware, command);
}

fn display(
    ui: &mut Ui,
    shown: &Shown<'_>,
    hardware: &Hardware,
    command: &mut Option<PreferencesCommand>,
) {
    let graphics = &shown.preferences.graphics;
    let note = "caditor draws only when something changes; these apply while the view moves or \
                animates.";
    let vsync_optional = hardware
        .adapter
        .as_ref()
        .is_none_or(|adapter| adapter.vsync_optional);
    preferences::section(
        ui,
        shown,
        Section::GraphicsDisplay,
        Some(note.to_owned()),
        |ui| {
            widgets::property(ui, "Vsync", |ui| {
                let mut vsync = graphics.vsync || !vsync_optional;
                let response = ui
                    .add_enabled_ui(vsync_optional, |ui| {
                        widgets::checkbox(ui, &mut vsync, "Wait for the display")
                    })
                    .inner
                    .on_hover_text(
                        "On: each frame waits for the display's refresh, so nothing tears. Off: \
                         a frame is shown as soon as it is drawn, for less delay after a mouse \
                         move",
                    )
                    .on_disabled_hover_text(
                        "This display only shows frames in step with its refresh, so vsync stays \
                         on",
                    );
                widgets::tie_to_caption(ui, &response);
                if response.changed() {
                    preferences::change(command, PreferenceChange::Vsync(vsync));
                }
            });
            let waiting = graphics.vsync || !vsync_optional;
            let waiting_hover = "Turn off Wait for the display to choose a frame rate";
            widgets::property(ui, "Frame rate", |ui| {
                ui.add_enabled_ui(!waiting, |ui| {
                    widgets::combo("frame-rate", graphics.frame_limit.mode_label())
                        .hover(graphics.frame_limit.hover(hardware.refresh_rate))
                        .show(ui, |ui| {
                            for mode in FrameLimit::modes() {
                                let current = graphics.frame_limit;
                                let chosen = widgets::menu_option(
                                    ui,
                                    current.same_mode(mode),
                                    mode.mode_label(),
                                )
                                .on_hover_text(mode.hover(hardware.refresh_rate))
                                .clicked();
                                if chosen && !current.same_mode(mode) {
                                    preferences::change(
                                        command,
                                        PreferenceChange::FrameLimit(mode),
                                    );
                                }
                            }
                        });
                })
                .response
                .on_disabled_hover_text(waiting_hover);
            });
            if let Some(current) = graphics.frame_limit.rate() {
                widgets::property(ui, "Frames a second", |ui| {
                    let mut rate = current;
                    let response = ui
                        .add_enabled_ui(!waiting, |ui| {
                            widgets::slider(
                                ui,
                                egui::Slider::new(
                                    &mut rate,
                                    FrameLimit::MIN_FPS..=FrameLimit::MAX_FPS,
                                )
                                .logarithmic(true)
                                .suffix(" fps"),
                            )
                        })
                        .inner;
                    widgets::tie_to_caption(ui, &response);
                    let make = |rate| PreferenceChange::FrameLimit(FrameLimit::fps(rate));
                    if response.drag_stopped() || (response.changed() && !response.dragged()) {
                        preferences::change(command, make(rate));
                    } else if response.changed() {
                        *command = Some(PreferencesCommand::Preview(make(rate)));
                    }
                });
            }
        },
    );
}

fn quality(
    ui: &mut Ui,
    shown: &Shown<'_>,
    hardware: &Hardware,
    command: &mut Option<PreferencesCommand>,
) {
    let graphics = &shown.preferences.graphics;
    let adapter = hardware.adapter.as_ref();
    let fallback = adapter
        .filter(|adapter| adapter.msaa != graphics.msaa)
        .map(|adapter| adapter.msaa);
    preferences::section(ui, shown, Section::GraphicsQuality, None, |ui| {
        widgets::property(ui, "Anti-aliasing", |ui| {
            let hovers = Msaa::ALL.map(msaa_hover);
            let refusals = Msaa::ALL.map(|msaa| {
                let offered = adapter.is_none_or(|adapter| adapter.offers(msaa));
                (!offered).then(|| msaa_refusal(msaa))
            });
            let segments: Vec<Segment<'_>> = Msaa::ALL
                .iter()
                .zip(hovers.iter().zip(&refusals))
                .map(|(msaa, (hover, refusal))| Segment {
                    label: msaa_label(*msaa),
                    hover,
                    refusal: refusal.as_deref(),
                })
                .collect();
            let selected = Msaa::ALL
                .iter()
                .position(|msaa| *msaa == graphics.msaa)
                .unwrap_or(usize::MAX);
            if let Some(msaa) = widgets::segmented_with(ui, &segments, selected)
                .and_then(|index| Msaa::ALL.get(index).copied())
            {
                preferences::change(command, PreferenceChange::Msaa(msaa));
            }
        });
        if let Some(used) = fallback {
            ui.label("");
            widgets::callout(ui, Tone::Info, |ui| {
                ui.label(format!(
                    "This graphics adapter cannot draw {} anti-aliasing, so {} is used.",
                    msaa_label(graphics.msaa),
                    msaa_label(used)
                ));
            });
            ui.end_row();
        }
        widgets::property(ui, "Shading", |ui| {
            let options = Shading::ALL
                .map(|shading| (shading, shading_label(shading), shading_hover(shading)));
            if let Some(shading) = preferences::choice(ui, &options, graphics.shading) {
                preferences::change(command, PreferenceChange::Shading(shading));
            }
        });
        widgets::property(ui, "Curve smoothness", |ui| {
            let options = CurveQuality::ALL.map(|curves| (curves, curves.label(), curves.hover()));
            if let Some(curves) = preferences::choice(ui, &options, graphics.curves) {
                preferences::change(command, PreferenceChange::CurveQuality(curves));
            }
        });
    });
}

fn adapter(
    ui: &mut Ui,
    shown: &Shown<'_>,
    hardware: &Hardware,
    command: &mut Option<PreferencesCommand>,
) {
    let graphics = &shown.preferences.graphics;
    let note = "Include these details when reporting a drawing problem. Choosing another adapter \
                switches the drawing to it at once; WGPU_POWER_PREF, when set, still decides.";
    preferences::section(
        ui,
        shown,
        Section::GraphicsAdapter,
        Some(note.to_owned()),
        |ui| {
            widgets::property(ui, "Prefer", |ui| {
                let options = AdapterPreference::ALL
                    .map(|adapter| (adapter, adapter_label(adapter), adapter_hover(adapter)));
                if let Some(adapter) = preferences::choice(ui, &options, graphics.adapter) {
                    preferences::change(command, PreferenceChange::Adapter(adapter));
                }
            });
            if graphics.adapter == AdapterPreference::Auto {
                ui.label("");
                ui.add(Label::new(widgets::muted(auto_summary(Machine::detect()), ui)).wrap());
                ui.end_row();
            }
            match &hardware.adapter {
                Some(adapter) => {
                    widgets::property(ui, "Adapter", |ui| {
                        ui.add(Label::new(adapter.adapter.as_str()).wrap());
                    });
                    widgets::property(ui, "Backend", |ui| ui.label(adapter.backend.as_str()));
                    if !adapter.driver.is_empty() {
                        widgets::property(ui, "Driver", |ui| {
                            ui.add(Label::new(adapter.driver.as_str()).wrap());
                        });
                    }
                }
                None => {
                    widgets::property(ui, "Adapter", |ui| {
                        ui.label(widgets::muted("Not drawing yet", ui));
                    });
                }
            }
            if let Some(rate) = usable_rate(hardware.refresh_rate) {
                widgets::property(ui, "Display", |ui| ui.label(format!("{rate:.0} Hz")));
            }
            widgets::property(ui, "Details", |ui| {
                let button = widgets::small_button(ui, icons::COPY_PATH, COPY_DETAILS);
                if ui
                    .add(button)
                    .on_hover_text("Copy the adapter, driver and settings in use")
                    .clicked()
                {
                    ui.ctx().copy_text(hardware.details(graphics));
                }
            });
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphics_settings_are_read_clamped_to_what_caditor_offers() {
        let mut raw = Settings::default();
        raw.set_flag(VSYNC_KEY, false);
        raw.set_text(FRAME_LIMIT_KEY, "75");
        raw.set_number(MSAA_KEY, 16.0);
        raw.set_text(SHADING_KEY, "enhanced");
        raw.set_text(CURVE_QUALITY_KEY, "coarse");
        raw.set_text(ADAPTER_KEY, "performance");

        let read = Graphics::from_settings(&raw);

        assert!(!read.vsync);
        assert_eq!(read.frame_limit, FrameLimit::Fps(75));
        assert_eq!(read.msaa, Msaa::X8);
        assert_eq!(read.shading, Shading::Enhanced);
        assert_eq!(read.curves, CurveQuality::Coarse);
        assert_eq!(read.adapter, AdapterPreference::Performance);
        assert_eq!(read.render().adapter, AdapterPreference::Performance);
        assert_eq!(read.curves.mesh_quality(), MeshQuality::COARSE);

        let mut written = Settings::default();
        read.write(&mut written);
        assert_eq!(written.text(FRAME_LIMIT_KEY), Some("75"));
        assert_eq!(written.number(MSAA_KEY), Some(8.0));
        assert_eq!(written.text(ADAPTER_KEY), Some("performance"));
        assert_eq!(Graphics::from_settings(&written), read);

        let mut odd = Settings::default();
        odd.set_text(FRAME_LIMIT_KEY, "fast");
        odd.set_number(MSAA_KEY, 3.0);
        odd.set_text(SHADING_KEY, "raytraced");
        odd.set_text(CURVE_QUALITY_KEY, "exact");
        odd.set_text(ADAPTER_KEY, "fastest");
        let odd = Graphics::from_settings(&odd);
        assert_eq!(odd.frame_limit, FrameLimit::Display);
        assert_eq!(odd.msaa, Msaa::X2);
        assert_eq!(odd.shading, Shading::Enhanced);
        assert_eq!(odd.curves, CurveQuality::Smooth);
        assert_eq!(odd.adapter, AdapterPreference::Auto);

        let mut numeric = Settings::default();
        numeric.set_number(FRAME_LIMIT_KEY, 5000.0);
        assert_eq!(
            Graphics::from_settings(&numeric).frame_limit,
            FrameLimit::Fps(FrameLimit::MAX_FPS)
        );
        numeric.set_number(FRAME_LIMIT_KEY, 0.0);
        assert_eq!(
            Graphics::from_settings(&numeric).frame_limit,
            FrameLimit::Fps(FrameLimit::MIN_FPS)
        );
        assert_eq!(
            Graphics::from_settings(&Settings::default()),
            Graphics::default()
        );
    }

    #[test]
    fn the_adapter_preference_defaults_to_auto_and_keeps_an_explicit_saved_choice() {
        let mut saved = Settings::default();
        saved.set_text(ADAPTER_KEY, "power_saving");
        let mut written = Settings::default();
        let auto = Graphics::default();
        auto.write(&mut written);

        assert_eq!(auto.adapter, AdapterPreference::Auto);
        assert_eq!(
            Graphics::from_settings(&saved).adapter,
            AdapterPreference::PowerSaving
        );
        assert_eq!(written.text(ADAPTER_KEY), Some("auto"));
        assert_eq!(
            Graphics::from_settings(&written).adapter,
            AdapterPreference::Auto
        );
        assert_eq!(
            Graphics::from_settings(&Settings::default()).adapter,
            AdapterPreference::Auto
        );
    }

    #[test]
    fn the_auto_line_says_what_auto_resolved_to_and_why() {
        assert_eq!(
            auto_summary(Machine::Desktop),
            "Auto: high performance, this computer has no battery."
        );
        assert_eq!(
            auto_summary(Machine::Portable),
            "Auto: power saving, this computer is a laptop or has a battery."
        );
        assert_eq!(
            auto_summary(Machine::Unknown),
            "Auto: power saving, caditor cannot tell whether this computer has a battery."
        );
    }

    #[test]
    fn frame_limits_become_intervals_and_waiting_for_the_display_leaves_vsync_to_pace() {
        let sixtieth = Duration::from_secs_f64(1.0 / 60.0);

        assert_eq!(FrameLimit::Unlimited.interval(false, Some(60.0)), None);
        assert_eq!(FrameLimit::Fps(60).interval(true, Some(144.0)), None);
        assert_eq!(
            FrameLimit::Fps(60).interval(false, Some(144.0)),
            Some(sixtieth)
        );
        assert_eq!(
            FrameLimit::Fps(75).interval(false, None),
            Some(Duration::from_secs_f64(1.0 / 75.0))
        );
        assert_eq!(FrameLimit::Display.interval(true, Some(60.0)), None);
        assert_eq!(
            FrameLimit::Display.interval(false, Some(60.0)),
            Some(sixtieth)
        );
        assert_eq!(FrameLimit::Display.interval(false, None), None);
        assert_eq!(FrameLimit::Display.interval(false, Some(0.0)), None);
        assert_eq!(FrameLimit::Display.interval(false, Some(f64::NAN)), None);
    }

    #[test]
    fn the_pacer_holds_frames_to_the_limit_on_a_steady_cadence_and_never_delays_an_idle_one() {
        let interval = Some(Duration::from_millis(10));
        let start = Instant::now();
        let at = |millis: u64| start + Duration::from_millis(millis);
        let mut pacer = FramePacer::default();

        assert_eq!(pacer.next_frame_at(start, interval), None);
        pacer.frame_started(start, interval);
        assert_eq!(pacer.next_frame_at(at(3), interval), Some(at(10)));
        assert_eq!(pacer.next_frame_at(at(3), None), None);

        pacer.frame_started(at(12), interval);
        assert_eq!(pacer.next_frame_at(at(13), interval), Some(at(20)));

        pacer.frame_started(at(21), interval);
        assert_eq!(pacer.next_frame_at(at(21), interval), Some(at(30)));

        pacer.frame_started(at(500), interval);
        assert_eq!(pacer.next_frame_at(at(505), interval), Some(at(510)));
        assert_eq!(pacer.next_frame_at(at(510), interval), None);
    }
}
