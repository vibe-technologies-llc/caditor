use caditor_file::Settings;
use egui::Rangef;

const WIDTH_KEY: &str = "window.width";
const HEIGHT_KEY: &str = "window.height";
const X_KEY: &str = "window.x";
const Y_KEY: &str = "window.y";
const MAXIMIZED_KEY: &str = "window.maximized";
const SIDE_WIDTH_KEY: &str = "panels.side_width";
const FEATURES_OPEN_KEY: &str = "panels.features_open";
const PARAMETERS_OPEN_KEY: &str = "panels.parameters_open";
pub const MIN_WINDOW_WIDTH: f64 = 480.0;
pub const MIN_WINDOW_HEIGHT: f64 = 360.0;
const MAX_WINDOW_SIDE: f64 = 16_384.0;
const MAX_POSITION: f64 = 1_048_576.0;
const VISIBLE_CORNER: i64 = 48;
pub const DEFAULT_SIDE_WIDTH: f32 = 330.0;
pub const MIN_SIDE_WIDTH: f32 = 270.0;
const MAX_SIDE_WIDTH: f32 = 1_600.0;
const MAX_PANELS_SHARE: f32 = 0.6;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogicalSize {
    pub width: f64,
    pub height: f64,
}

impl LogicalSize {
    pub fn clamped(width: f64, height: f64) -> Option<Self> {
        (width.is_finite() && height.is_finite()).then(|| Self {
            width: width.round().clamp(MIN_WINDOW_WIDTH, MAX_WINDOW_SIDE),
            height: height.round().clamp(MIN_WINDOW_HEIGHT, MAX_WINDOW_SIDE),
        })
    }

    fn within(self, room: Self) -> Self {
        Self {
            width: self.width.min(room.width).max(MIN_WINDOW_WIDTH),
            height: self.height.min(room.height).max(MIN_WINDOW_HEIGHT),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub x: i32,
    pub y: i32,
}

impl Position {
    fn read(x: f64, y: f64) -> Option<Self> {
        let valid = |value: f64| value.is_finite() && value.abs() <= MAX_POSITION;
        (valid(x) && valid(y)).then(|| Self {
            x: x.round() as i32,
            y: y.round() as i32,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorArea {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

impl MonitorArea {
    fn logical_size(self) -> LogicalSize {
        let scale = if self.scale.is_finite() && self.scale > 0.0 {
            self.scale
        } else {
            1.0
        };
        LogicalSize {
            width: f64::from(self.width) / scale,
            height: f64::from(self.height) / scale,
        }
    }

    fn shows(self, position: Position) -> bool {
        let x = i64::from(position.x) + VISIBLE_CORNER;
        let y = i64::from(position.y) + VISIBLE_CORNER;
        let left = i64::from(self.x);
        let top = i64::from(self.y);
        (left..left + i64::from(self.width)).contains(&x)
            && (top..top + i64::from(self.height)).contains(&y)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WindowPlacement {
    pub size: Option<LogicalSize>,
    pub position: Option<Position>,
    pub maximized: bool,
}

impl WindowPlacement {
    pub fn from_settings(settings: &Settings) -> Self {
        let size = settings
            .number(WIDTH_KEY)
            .zip(settings.number(HEIGHT_KEY))
            .and_then(|(width, height)| LogicalSize::clamped(width, height));
        let position = settings
            .number(X_KEY)
            .zip(settings.number(Y_KEY))
            .and_then(|(x, y)| Position::read(x, y));
        Self {
            size,
            position,
            maximized: settings.flag(MAXIMIZED_KEY).unwrap_or(false),
        }
    }

    pub fn write(&self, settings: &mut Settings) {
        if let Some(size) = self.size {
            settings.set_number(WIDTH_KEY, size.width);
            settings.set_number(HEIGHT_KEY, size.height);
        }
        if let Some(position) = self.position {
            settings.set_number(X_KEY, f64::from(position.x));
            settings.set_number(Y_KEY, f64::from(position.y));
        }
        settings.set_flag(MAXIMIZED_KEY, self.maximized);
    }

    pub fn fitted(self, monitors: &[MonitorArea]) -> Self {
        let largest = monitors
            .iter()
            .map(|monitor| monitor.logical_size())
            .reduce(|a, b| LogicalSize {
                width: a.width.max(b.width),
                height: a.height.max(b.height),
            });
        Self {
            size: match (self.size, largest) {
                (Some(size), Some(room)) => Some(size.within(room)),
                (size, None) => size,
                (None, Some(_)) => None,
            },
            position: self
                .position
                .filter(|position| monitors.iter().any(|monitor| monitor.shows(*position))),
            maximized: self.maximized,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelLayout {
    pub side_width: f32,
    pub features_open: bool,
    pub parameters_open: bool,
}

impl Default for PanelLayout {
    fn default() -> Self {
        Self {
            side_width: DEFAULT_SIDE_WIDTH,
            features_open: true,
            parameters_open: true,
        }
    }
}

impl PanelLayout {
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            side_width: settings
                .number(SIDE_WIDTH_KEY)
                .map_or(DEFAULT_SIDE_WIDTH, |width| side_width(width as f32)),
            features_open: settings.flag(FEATURES_OPEN_KEY).unwrap_or(true),
            parameters_open: settings.flag(PARAMETERS_OPEN_KEY).unwrap_or(true),
        }
    }

    pub fn write(&self, settings: &mut Settings) {
        settings.set_number(SIDE_WIDTH_KEY, f64::from(self.side_width));
        settings.set_flag(FEATURES_OPEN_KEY, self.features_open);
        settings.set_flag(PARAMETERS_OPEN_KEY, self.parameters_open);
    }
}

pub fn panel_room(window: f32, open_panels: usize) -> f32 {
    let open = open_panels.max(1) as f32;
    (window * MAX_PANELS_SHARE / open).max(0.0)
}

pub fn panel_widths(room: f32, least: f32) -> Rangef {
    let min = least.min(room);
    Rangef::new(min, room.max(min))
}

pub fn side_width(width: f32) -> f32 {
    if width.is_finite() {
        width.round().clamp(MIN_SIDE_WIDTH, MAX_SIDE_WIDTH)
    } else {
        DEFAULT_SIDE_WIDTH
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placements_read_back_what_they_wrote_and_refuse_nonsense() {
        let placement = WindowPlacement {
            size: LogicalSize::clamped(1280.4, 800.0),
            position: Position::read(-1920.0, 40.0),
            maximized: true,
        };
        let mut settings = Settings::default();
        placement.write(&mut settings);

        assert_eq!(WindowPlacement::from_settings(&settings), placement);
        assert_eq!(
            placement.size,
            Some(LogicalSize {
                width: 1280.0,
                height: 800.0,
            })
        );

        settings.set_number(WIDTH_KEY, 1e9);
        settings.set_number(HEIGHT_KEY, 2.0);
        settings.set_number(X_KEY, 1e12);
        settings.set_text(MAXIMIZED_KEY, "yes");
        let read = WindowPlacement::from_settings(&settings);

        assert_eq!(
            read.size,
            Some(LogicalSize {
                width: MAX_WINDOW_SIDE,
                height: MIN_WINDOW_HEIGHT,
            })
        );
        assert_eq!(read.position, None);
        assert!(!read.maximized);
        assert_eq!(
            WindowPlacement::from_settings(&Settings::default()),
            WindowPlacement::default()
        );
    }

    #[test]
    fn a_placement_fits_the_monitors_there_are_now() {
        let placement = WindowPlacement {
            size: LogicalSize::clamped(3000.0, 1400.0),
            position: Position::read(2000.0, 100.0),
            maximized: false,
        };
        let laptop = MonitorArea {
            x: 0,
            y: 0,
            width: 2880,
            height: 1800,
            scale: 2.0,
        };
        let external = MonitorArea {
            x: 2880,
            y: 0,
            width: 2560,
            height: 1440,
            scale: 1.0,
        };

        let alone = placement.fitted(&[laptop]);
        let both = placement.fitted(&[laptop, external]);
        let unknown = placement.fitted(&[]);

        assert_eq!(
            alone.size,
            Some(LogicalSize {
                width: 1440.0,
                height: 900.0,
            })
        );
        assert_eq!(alone.position, placement.position);
        assert_eq!(
            both.size,
            Some(LogicalSize {
                width: 2560.0,
                height: 1400.0,
            })
        );
        assert_eq!(placement.fitted(&[external]).position, None);
        assert_eq!(unknown.size, placement.size);
        assert_eq!(unknown.position, None);
    }

    #[test]
    fn panel_layouts_read_back_what_they_wrote_within_their_bounds() {
        let layout = PanelLayout {
            side_width: 412.0,
            features_open: false,
            parameters_open: true,
        };
        let mut settings = Settings::default();
        layout.write(&mut settings);

        assert_eq!(PanelLayout::from_settings(&settings), layout);

        settings.set_number(SIDE_WIDTH_KEY, 5.0);
        assert_eq!(
            PanelLayout::from_settings(&settings).side_width,
            MIN_SIDE_WIDTH
        );
        settings.set_number(SIDE_WIDTH_KEY, 1e7);
        assert_eq!(
            PanelLayout::from_settings(&settings).side_width,
            MAX_SIDE_WIDTH
        );
        assert_eq!(
            PanelLayout::from_settings(&Settings::default()),
            PanelLayout::default()
        );
        assert_eq!(side_width(f32::NAN), DEFAULT_SIDE_WIDTH);
    }
}
