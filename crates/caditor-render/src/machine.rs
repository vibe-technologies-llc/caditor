use std::sync::OnceLock;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as os;
#[cfg(windows)]
use windows as os;

#[cfg(not(any(unix, windows)))]
mod os {
    use super::Machine;

    pub(super) fn detect() -> Machine {
        Machine::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Machine {
    Portable,
    Desktop,
    Unknown,
}

impl Machine {
    pub fn detect() -> Self {
        static DETECTED: OnceLock<Machine> = OnceLock::new();
        *DETECTED.get_or_init(os::detect)
    }

    pub fn reason(self) -> &'static str {
        match self {
            Self::Portable => "this computer is a laptop or has a battery",
            Self::Desktop => "this computer has no battery",
            Self::Unknown => "caditor cannot tell whether this computer has a battery",
        }
    }
}
