use anyhow::{Context, Result};
use egui::IconData;
use log::info;
use windows::core::HSTRING;
use windows::core::PCWSTR;
use windows::Win32::Foundation::HINSTANCE;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject,
};
use windows::Win32::Graphics::Gdi::{BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS};
use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
use windows::Win32::UI::WindowsAndMessaging::{DrawIconEx, GetIconInfo, DI_NORMAL};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, IsIconic, PostMessageW, ShowWindow, SW_HIDE, SW_NORMAL, SW_RESTORE, WM_CLOSE,
};
use windows::Win32::UI::WindowsAndMessaging::{LoadImageW, IMAGE_ICON, LR_DEFAULTSIZE};
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

pub fn is_auto_start_enabled() -> bool {
    let run_key = match RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Run",
        KEY_READ,
    ) {
        Ok(key) => key,
        Err(_) => return false, // If we can't open the key, assume it's not enabled
    };

    run_key.get_value::<String, _>("NASBootClient").is_ok()
}

// Find the application window by title
pub fn find_app_window() -> Result<HWND, windows::core::Error> {
    // Use HSTRING for proper Windows string handling
    let window_title = HSTRING::from("NAS Boot Client");

    let hwnd = unsafe { FindWindowW(PCWSTR::null(), &window_title)? };

    if hwnd.is_invalid() {
        return Err(windows::core::Error::from_win32());
    }

    Ok(hwnd)
}

// Show and bring window to front
pub fn show_window(hwnd: HWND) {
    unsafe {
        // SW_RESTORE will restore from minimized state if needed
        let _ = ShowWindow(hwnd, SW_RESTORE);

        // Set focus to the window
        let _ = ShowWindow(hwnd, SW_NORMAL);
    }
}

// Hide the window
pub fn hide_window(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

/// Check if the window is minimized
pub fn is_window_minimized(hwnd: HWND) -> bool {
    unsafe { IsIconic(hwnd).as_bool() }
}

/// Check if the window is visible
pub fn is_window_visible(hwnd: HWND) -> bool {
    unsafe { IsWindowVisible(hwnd).as_bool() }
}

// Close the window by sending a WM_CLOSE message
pub fn close_window(hwnd: HWND) -> Result<(), windows::core::Error> {
    unsafe {
        if PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).is_ok() {
            Ok(())
        } else {
            Err(windows::core::Error::from_win32())
        }
    }
}

pub fn load_icon_from_resource() -> Option<IconData> {
    unsafe {
        // Get the current module handle
        let hinstance =
            windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();

        // Load the icon from resources
        // Note: Windows resources use wide strings, so we need to convert
        let icon_name = windows::core::w!("nas_black_ico");

        let hicon = LoadImageW(
            Some(HINSTANCE(hinstance.0)),
            PCWSTR(icon_name.as_ptr()),
            IMAGE_ICON,
            0, // width (0 = default)
            0, // height (0 = default)
            LR_DEFAULTSIZE,
        );

        match hicon {
            Ok(icon) if !icon.is_invalid() => {
                // Convert HICON to IconData

                let mut icon_info = windows::Win32::UI::WindowsAndMessaging::ICONINFO::default();
                if GetIconInfo(
                    windows::Win32::UI::WindowsAndMessaging::HICON(icon.0),
                    &mut icon_info,
                )
                .is_ok()
                {
                    let hdc = GetDC(None);
                    let mem_dc = CreateCompatibleDC(Some(hdc));

                    // Get icon dimensions (assuming 32x32 for tray icon)
                    let width = 32i32;
                    let height = 32i32;

                    let bitmap = CreateCompatibleBitmap(hdc, width, height);
                    let old_bitmap = SelectObject(mem_dc, bitmap.into());

                    // Draw the icon to the bitmap
                    let _ = DrawIconEx(
                        mem_dc,
                        0,
                        0,
                        windows::Win32::UI::WindowsAndMessaging::HICON(icon.0),
                        width,
                        height,
                        0,
                        None,
                        DI_NORMAL,
                    );

                    // Prepare bitmap info for getting pixel data
                    let mut bmi = BITMAPINFO {
                        bmiHeader: BITMAPINFOHEADER {
                            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                            biWidth: width,
                            biHeight: -height, // negative for top-down DIB
                            biPlanes: 1,
                            biBitCount: 32,
                            biCompression: 0, // BI_RGB
                            biSizeImage: 0,
                            biXPelsPerMeter: 0,
                            biYPelsPerMeter: 0,
                            biClrUsed: 0,
                            biClrImportant: 0,
                        },
                        bmiColors: [windows::Win32::Graphics::Gdi::RGBQUAD::default(); 1],
                    };

                    // Calculate buffer size
                    let pixel_count = (width * height) as usize;
                    let mut pixels = vec![0u8; pixel_count * 4]; // RGBA

                    // Get bitmap bits
                    let result = GetDIBits(
                        mem_dc,
                        bitmap,
                        0,
                        height as u32,
                        Some(pixels.as_mut_ptr().cast()),
                        &mut bmi,
                        DIB_RGB_COLORS,
                    );

                    // Clean up GDI objects
                    SelectObject(mem_dc, old_bitmap);
                    let _ = DeleteObject(bitmap.into());
                    let _ = DeleteDC(mem_dc);
                    ReleaseDC(None, hdc);
                    let _ = DeleteObject(icon_info.hbmColor.into());
                    let _ = DeleteObject(icon_info.hbmMask.into());

                    if result > 0 {
                        // Convert BGRA to RGBA
                        for chunk in pixels.chunks_mut(4) {
                            chunk.swap(0, 2); // Swap B and R channels
                        }

                        Some(IconData {
                            rgba: pixels,
                            width: width as u32,
                            height: height as u32,
                        })
                    } else {
                        log::error!("Failed to get bitmap bits");
                        None
                    }
                } else {
                    log::error!("Failed to get icon info");
                    None
                }
            }
            _ => {
                log::error!("Failed to load icon from resources");
                None
            }
        }
    }
}
