use super::Machine;

const NO_SYSTEM_BATTERY: u8 = 128;
const UNKNOWN_BATTERY_STATUS: u8 = 255;

pub(super) fn detect() -> Machine {
    match caditor_windows::battery_flag() {
        Ok(flag) => classify(flag),
        Err(_) => Machine::Unknown,
    }
}

fn classify(battery_flag: u8) -> Machine {
    match battery_flag {
        NO_SYSTEM_BATTERY => Machine::Desktop,
        UNKNOWN_BATTERY_STATUS => Machine::Unknown,
        _ => Machine::Portable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_system_battery_is_a_desktop_and_any_battery_makes_it_portable() {
        assert_eq!(classify(128), Machine::Desktop);
        assert_eq!(classify(255), Machine::Unknown);

        for flag in [0, 1, 2, 4, 8, 9] {
            assert_eq!(classify(flag), Machine::Portable);
        }
    }
}
