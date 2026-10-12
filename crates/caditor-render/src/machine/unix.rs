use std::{fs, path::Path};

use super::Machine;

const POWER_SUPPLIES: &str = "/sys/class/power_supply";
const CHASSIS_TYPE: &str = "/sys/class/dmi/id/chassis_type";
const BATTERY: &str = "Battery";
const DEVICE_SCOPE: &str = "Device";
const PORTABLE_CHASSIS: [u32; 8] = [8, 9, 10, 11, 14, 30, 31, 32];
const UNINFORMATIVE_CHASSIS: [u32; 3] = [0, 1, 2];

#[derive(Debug, Clone, PartialEq, Eq)]
struct PowerSupply {
    kind: String,
    scope: Option<String>,
}

impl PowerSupply {
    fn is_system_battery(&self) -> bool {
        self.kind == BATTERY && self.scope.as_deref() != Some(DEVICE_SCOPE)
    }
}

pub(super) fn detect() -> Machine {
    classify(
        read_power_supplies(Path::new(POWER_SUPPLIES)).as_deref(),
        read_chassis_type(Path::new(CHASSIS_TYPE)),
    )
}

fn classify(supplies: Option<&[PowerSupply]>, chassis: Option<u32>) -> Machine {
    let has_battery =
        supplies.is_some_and(|supplies| supplies.iter().any(PowerSupply::is_system_battery));
    let informative_chassis = chassis.filter(|chassis| !UNINFORMATIVE_CHASSIS.contains(chassis));
    let portable_chassis =
        informative_chassis.is_some_and(|chassis| PORTABLE_CHASSIS.contains(&chassis));
    if has_battery || portable_chassis {
        Machine::Portable
    } else if informative_chassis.is_some() || supplies.is_some() {
        Machine::Desktop
    } else {
        Machine::Unknown
    }
}

fn read_power_supplies(directory: &Path) -> Option<Vec<PowerSupply>> {
    let entries = fs::read_dir(directory).ok()?;
    let supplies = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let kind = read_trimmed(&entry.path().join("type"))?;
            let scope = read_trimmed(&entry.path().join("scope"));
            Some(PowerSupply { kind, scope })
        })
        .collect();
    Some(supplies)
}

fn read_chassis_type(path: &Path) -> Option<u32> {
    read_trimmed(path)?.parse().ok()
}

fn read_trimmed(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    Some(text.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn supply(kind: &str, scope: Option<&str>) -> PowerSupply {
        PowerSupply {
            kind: kind.to_owned(),
            scope: scope.map(str::to_owned),
        }
    }

    #[test]
    fn a_system_battery_makes_the_computer_portable() {
        let supplies = [supply("Mains", None), supply("Battery", Some("System"))];
        let unscoped = [supply("Battery", None)];

        assert_eq!(classify(Some(&supplies), Some(3)), Machine::Portable);
        assert_eq!(classify(Some(&supplies), None), Machine::Portable);
        assert_eq!(classify(Some(&unscoped), None), Machine::Portable);
    }

    #[test]
    fn a_battery_of_an_attached_device_does_not_count() {
        let mouse = [supply("Mains", None), supply("Battery", Some("Device"))];

        assert_eq!(classify(Some(&mouse), Some(3)), Machine::Desktop);
        assert_eq!(classify(Some(&mouse), None), Machine::Desktop);
    }

    #[test]
    fn a_portable_chassis_makes_the_computer_portable_without_a_battery() {
        for chassis in PORTABLE_CHASSIS {
            assert_eq!(classify(Some(&[]), Some(chassis)), Machine::Portable);
            assert_eq!(classify(None, Some(chassis)), Machine::Portable);
        }
    }

    #[test]
    fn a_fixed_chassis_without_a_battery_is_a_desktop() {
        for chassis in [3, 4, 5, 6, 7, 13, 17, 23, 35] {
            assert_eq!(classify(Some(&[]), Some(chassis)), Machine::Desktop);
            assert_eq!(classify(None, Some(chassis)), Machine::Desktop);
        }
    }

    #[test]
    fn an_unreadable_or_uninformative_computer_is_judged_by_what_can_be_read() {
        assert_eq!(classify(None, None), Machine::Unknown);
        assert_eq!(classify(None, Some(2)), Machine::Unknown);
        assert_eq!(classify(Some(&[]), Some(1)), Machine::Desktop);
        assert_eq!(classify(Some(&[]), None), Machine::Desktop);
    }

    #[test]
    fn the_power_supplies_and_chassis_are_read_from_files() {
        let directory = tempfile::tempdir().unwrap();
        let supplies = directory.path().join("power_supply");
        fs::create_dir(&supplies).unwrap();

        for (name, kind, scope) in [
            ("BAT0", "Battery\n", Some("System\n")),
            ("hidpp_battery_0", "Battery\n", Some("Device\n")),
            ("AC", "Mains\n", None),
        ] {
            let path = supplies.join(name);
            fs::create_dir(&path).unwrap();
            fs::write(path.join("type"), kind).unwrap();
            if let Some(scope) = scope {
                fs::write(path.join("scope"), scope).unwrap();
            }
        }
        let chassis = directory.path().join("chassis_type");
        fs::write(&chassis, "10\n").unwrap();

        let mut read = read_power_supplies(&supplies).unwrap();
        read.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.scope.cmp(&b.scope)));

        assert_eq!(
            read,
            [
                supply("Battery", Some("Device")),
                supply("Battery", Some("System")),
                supply("Mains", None),
            ]
        );
        assert_eq!(read_chassis_type(&chassis), Some(10));
        assert_eq!(read_chassis_type(&directory.path().join("missing")), None);
        assert_eq!(read_power_supplies(&directory.path().join("missing")), None);
    }
}
