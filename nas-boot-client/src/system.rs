use anyhow::{Context, Result};
use log::info;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Console::GetConsoleWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, PostMessageW, ShowWindow, SW_HIDE, SW_NORMAL, SW_RESTORE, SW_SHOW, WM_CLOSE,
};
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

// Find the application window by title
pub fn find_app_window() -> Result<HWND, windows::core::Error> {
    // Convert the window title to wide string for FindWindowW
    let window_title = "NAS Boot Client";
    let wide_title: Vec<u16> = window_title
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let hwnd = unsafe { FindWindowW(None, wide_title.as_ptr()) };

    if hwnd.0 == 0 {
        return Err(windows::core::Error::from_win32());
    }

    Ok(hwnd)
}

// Show and bring window to front
pub fn show_window(hwnd: HWND) -> Result<(), windows::core::Error> {
    unsafe {
        // SW_RESTORE will restore from minimized state if needed
        ShowWindow(hwnd, SW_RESTORE);

        // Set focus to the window
        ShowWindow(hwnd, SW_NORMAL);

        Ok(())
    }
}

// Close the window by sending a WM_CLOSE message
pub fn close_window(hwnd: HWND) -> Result<(), windows::core::Error> {
    unsafe {
        if PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)).as_bool() {
            Ok(())
        } else {
            Err(windows::core::Error::from_win32())
        }
    }
}
