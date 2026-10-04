use std::cmp::Reverse;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Msaa {
    Off,
    X2,
    #[default]
    X4,
    X8,
}

impl Msaa {
    pub const ALL: [Self; 4] = [Self::Off, Self::X2, Self::X4, Self::X8];

    pub fn samples(self) -> u32 {
        match self {
            Self::Off => 1,
            Self::X2 => 2,
            Self::X4 => 4,
            Self::X8 => 8,
        }
    }

    pub fn at_most(samples: f64) -> Option<Self> {
        if samples.is_nan() {
            return None;
        }
        Some(
            Self::ALL
                .into_iter()
                .rev()
                .find(|level| f64::from(level.samples()) <= samples)
                .unwrap_or(Self::Off),
        )
    }

    pub fn closest(self, offered: &[Self]) -> Self {
        offered
            .iter()
            .copied()
            .chain([Self::Off])
            .min_by_key(|level| (self.doublings_from(*level), Reverse(*level)))
            .unwrap_or(Self::Off)
    }

    fn doublings_from(self, other: Self) -> u32 {
        self.samples()
            .trailing_zeros()
            .abs_diff(other.samples().trailing_zeros())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Shading {
    #[default]
    Standard,
    Enhanced,
}

impl Shading {
    pub const ALL: [Self; 2] = [Self::Standard, Self::Enhanced];

    pub(crate) fn uniform_flag(self) -> f32 {
        match self {
            Self::Standard => 0.0,
            Self::Enhanced => 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AdapterPreference {
    #[default]
    PowerSaving,
    Performance,
}

impl AdapterPreference {
    pub const ALL: [Self; 2] = [Self::PowerSaving, Self::Performance];

    pub(crate) fn power(self) -> wgpu::PowerPreference {
        match self {
            Self::PowerSaving => wgpu::PowerPreference::LowPower,
            Self::Performance => wgpu::PowerPreference::HighPerformance,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphicsSettings {
    pub vsync: bool,
    pub msaa: Msaa,
    pub shading: Shading,
    pub adapter: AdapterPreference,
}

impl Default for GraphicsSettings {
    fn default() -> Self {
        Self {
            vsync: true,
            msaa: Msaa::default(),
            shading: Shading::default(),
            adapter: AdapterPreference::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphicsInfo {
    pub adapter: String,
    pub backend: String,
    pub driver: String,
    pub msaa_offered: Vec<Msaa>,
    pub msaa: Msaa,
    pub vsync_optional: bool,
    pub vsync: bool,
}

impl GraphicsInfo {
    pub(crate) fn describe(adapter: &wgpu::Adapter) -> Self {
        let info = adapter.get_info();
        let driver = [info.driver.trim(), info.driver_info.trim()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        Self {
            adapter: info.name,
            backend: backend_name(info.backend).to_owned(),
            driver,
            msaa_offered: vec![Msaa::Off],
            msaa: Msaa::Off,
            vsync_optional: false,
            vsync: true,
        }
    }

    pub fn offers(&self, msaa: Msaa) -> bool {
        msaa == Msaa::Off || self.msaa_offered.contains(&msaa)
    }
}

fn backend_name(backend: wgpu::Backend) -> &'static str {
    match backend {
        wgpu::Backend::Vulkan => "Vulkan",
        wgpu::Backend::Gl => "OpenGL",
        wgpu::Backend::Metal => "Metal",
        wgpu::Backend::Dx12 => "Direct3D 12",
        wgpu::Backend::BrowserWebGpu => "WebGPU",
        wgpu::Backend::Noop => "None",
    }
}

const UNSYNCED_MODES: [wgpu::PresentMode; 2] =
    [wgpu::PresentMode::Mailbox, wgpu::PresentMode::Immediate];

pub fn present_mode(vsync: bool, offered: &[wgpu::PresentMode]) -> wgpu::PresentMode {
    UNSYNCED_MODES
        .into_iter()
        .find(|mode| !vsync && offered.contains(mode))
        .unwrap_or(wgpu::PresentMode::Fifo)
}

pub fn vsync_optional(offered: &[wgpu::PresentMode]) -> bool {
    UNSYNCED_MODES.iter().any(|mode| offered.contains(mode))
}

#[cfg(test)]
mod tests {
    use wgpu::PresentMode;

    use super::*;

    #[test]
    fn a_level_the_adapter_lacks_falls_back_to_the_nearest_it_offers() {
        let all = [Msaa::X2, Msaa::X4, Msaa::X8];
        let no_eight = [Msaa::X2, Msaa::X4];
        let four_only = [Msaa::X4];

        assert_eq!(Msaa::X8.closest(&all), Msaa::X8);
        assert_eq!(Msaa::X8.closest(&no_eight), Msaa::X4);
        assert_eq!(Msaa::X2.closest(&four_only), Msaa::X4);
        assert_eq!(Msaa::X8.closest(&four_only), Msaa::X4);
        assert_eq!(Msaa::X4.closest(&[]), Msaa::Off);
        assert_eq!(Msaa::Off.closest(&all), Msaa::Off);
    }

    #[test]
    fn a_stored_sample_count_is_read_as_the_largest_level_within_it() {
        assert_eq!(Msaa::at_most(4.0), Some(Msaa::X4));
        assert_eq!(Msaa::at_most(16.0), Some(Msaa::X8));
        assert_eq!(Msaa::at_most(3.0), Some(Msaa::X2));
        assert_eq!(Msaa::at_most(0.0), Some(Msaa::Off));
        assert_eq!(Msaa::at_most(-2.0), Some(Msaa::Off));
        assert_eq!(Msaa::at_most(f64::NAN), None);
    }

    #[test]
    fn vsync_off_prefers_mailbox_then_immediate_and_falls_back_to_fifo() {
        let everything = [
            PresentMode::Fifo,
            PresentMode::Immediate,
            PresentMode::Mailbox,
        ];

        assert_eq!(present_mode(true, &everything), PresentMode::Fifo);
        assert_eq!(present_mode(false, &everything), PresentMode::Mailbox);
        assert_eq!(
            present_mode(false, &[PresentMode::Fifo, PresentMode::Immediate]),
            PresentMode::Immediate
        );
        assert_eq!(
            present_mode(false, &[PresentMode::Fifo, PresentMode::FifoRelaxed]),
            PresentMode::Fifo
        );
        assert!(vsync_optional(&everything));
        assert!(!vsync_optional(&[PresentMode::Fifo]));
    }
}
