use log::error;
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

pub fn is_user_active(idle_threshold_mins: u32) -> bool {
    // Calculate idle threshold in milliseconds
    let idle_threshold_ms = u64::from(idle_threshold_mins) * 60 * 1000;

    // Get current tick count
    let current_tick_count = unsafe { GetTickCount() };

    // Initialize LASTINPUTINFO structure
    let mut last_input_info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };

    // Get the last input info
    let result = unsafe { GetLastInputInfo(&mut last_input_info) };

    if !result.as_bool() {
        error!("Failed to get last input info");
        return true; // Assume user is active if we can't determine
    }

    // Calculate idle time in milliseconds
    let idle_time = current_tick_count.wrapping_sub(last_input_info.dwTime);

    // Consider user active if idle time is less than threshold
    idle_time < idle_threshold_ms as u32
}
