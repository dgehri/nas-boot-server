use log::error;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use parking_lot::Mutex;
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

// Cache structure to avoid frequent Windows API calls
#[derive(Debug)]
struct ActivityCache {
    last_check: Instant,
    last_result: bool,
    cache_duration: Duration,
}

static ACTIVITY_CACHE: OnceLock<Mutex<ActivityCache>> = OnceLock::new();

pub fn is_user_active(idle_threshold_mins: u32) -> bool {
    let cache = ACTIVITY_CACHE.get_or_init(|| {
        Mutex::new(ActivityCache {
            last_check: Instant::now() - Duration::from_secs(60), // Force initial check
            last_result: true,
            cache_duration: Duration::from_secs(10), // Cache for 10 seconds
        })
    });

    let mut cache_guard = cache.lock();
    
    // Return cached result if it's still fresh
    if cache_guard.last_check.elapsed() < cache_guard.cache_duration {
        return cache_guard.last_result;
    }

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

    let is_active = if !result.as_bool() {
        error!("Failed to get last input info");
        true // Assume user is active if we can't determine
    } else {
        // Calculate idle time in milliseconds
        let idle_time = current_tick_count.wrapping_sub(last_input_info.dwTime);
        // Consider user active if idle time is less than threshold
        idle_time < idle_threshold_ms as u32
    };

    // Update cache
    cache_guard.last_check = Instant::now();
    cache_guard.last_result = is_active;

    is_active
}
