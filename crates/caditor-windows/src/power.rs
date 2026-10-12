use std::io;

use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

pub fn battery_flag() -> io::Result<u8> {
    let mut status = SYSTEM_POWER_STATUS {
        ACLineStatus: 0,
        BatteryFlag: 0,
        BatteryLifePercent: 0,
        SystemStatusFlag: 0,
        BatteryLifeTime: 0,
        BatteryFullLifeTime: 0,
    };
    #[expect(unsafe_code, reason = "the Win32 call is unsafe to make")]
    let read = unsafe { GetSystemPowerStatus(&mut status) };
    if read == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(status.BatteryFlag)
    }
}
