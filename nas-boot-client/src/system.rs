use anyhow::{Context, Result};
use log::info;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Console::GetConsoleWindow;
use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE, SW_SHOW};
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

pub fn set_auto_start(enable: bool) -> Result<()> {
    let run_key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Run",
            KEY_WRITE,
        )
        .context("Failed to open registry key")?;

    let app_path = std::env::current_exe()?.to_string_lossy().to_string();

    if enable {
        run_key
            .set_value("NASBootClient", &app_path)
            .context("Failed to set registry value")?;
        info!("Auto-start enabled");
    } else {
        // Ignore errors if the key doesn't exist
        let _ = run_key.delete_value("NASBootClient");
        info!("Auto-start disabled");
    }

    Ok(())
}

pub fn is_auto_start_enabled() -> Result<bool> {
    let run_key = match RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Run",
        KEY_READ,
    ) {
        Ok(key) => key,
        Err(_) => return Ok(false), // If we can't open the key, assume it's not enabled
    };

    match run_key.get_value::<String, _>("NASBootClient") {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

pub fn show_window(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
    }
}

pub fn hide_window(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

pub fn hide_window_console() {
    let console_window = unsafe { GetConsoleWindow() };
    if !console_window.is_invalid() {
        unsafe {
            let _ = ShowWindow(console_window, SW_HIDE);
        }
    }
}

#[allow(dead_code)]
pub fn show_window_console() {
    let console_window = unsafe { GetConsoleWindow() };
    if !console_window.is_invalid() {
        unsafe {
            let _ = ShowWindow(console_window, SW_SHOW);
        }
    }
}
