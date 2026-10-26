//! Native window transparency effects on Windows 11.
//!
//! Acrylic material can optionally apply the legacy blur-behind effect. On
//! other platforms these are no-ops.
//!
//! The webview background must also be transparent for the effect to be
//! visible — that is handled on the frontend via the `--df-bg` CSS variable
//! and the `data-window-transparency` attribute on the wallpaper shell.

use tauri::WebviewWindow;

#[cfg(windows)]
use std::ffi::c_void;
#[cfg(windows)]
use windows_sys::Win32::Foundation::HWND;

/// Effective native window transparency state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowTransparency {
    Opaque,
    Transparent,
}

impl WindowTransparency {
    pub fn from_settings(_legacy_mode: &str, opacity: f64) -> Self {
        if opacity >= 1.0 || opacity.is_nan() {
            Self::Opaque
        } else {
            Self::Transparent
        }
    }
}

pub fn should_apply_acrylic_blur(mode: WindowTransparency, acrylic_blur: bool) -> bool {
    mode == WindowTransparency::Transparent && acrylic_blur
}

#[cfg(any(windows, test))]
fn should_skip_native_backdrop(
    version: Option<crate::platform::windows_version::WindowsVersion>,
) -> bool {
    matches!(version, Some(version) if version.major == 6 && version.minor == 1)
}

#[cfg(windows)]
fn legacy_acrylic_activation_color() -> window_vibrancy::Color {
    // The visible dimming is controlled by frontend CSS opacity. The native API
    // only needs a tiny alpha value to keep blur-behind active.
    (0u8, 0u8, 0u8, 1u8)
}

#[cfg(windows)]
#[repr(C)]
struct AccentPolicy {
    accent_state: u32,
    accent_flags: u32,
    gradient_color: u32,
    animation_id: u32,
}

#[cfg(windows)]
#[repr(C)]
struct WindowCompositionAttribData {
    attrib: u32,
    data: *mut c_void,
    data_size: usize,
}

#[cfg(windows)]
type SetWindowCompositionAttribute =
    unsafe extern "system" fn(HWND, *mut WindowCompositionAttribData) -> i32;

#[cfg(windows)]
fn set_window_composition_attribute() -> Option<SetWindowCompositionAttribute> {
    use std::sync::OnceLock;
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};

    static SET_WINDOW_COMPOSITION_ATTRIBUTE: OnceLock<Option<SetWindowCompositionAttribute>> =
        OnceLock::new();

    *SET_WINDOW_COMPOSITION_ATTRIBUTE.get_or_init(|| {
        let user32 = unsafe { GetModuleHandleA(c"user32.dll".as_ptr().cast()) };
        if user32.is_null() {
            return None;
        }

        let proc =
            unsafe { GetProcAddress(user32, c"SetWindowCompositionAttribute".as_ptr().cast()) }?;

        // SetWindowCompositionAttribute is an undocumented user32 export. The
        // function pointer ABI and parameter layout are the common Windows
        // "system" signature used for ACCENT_POLICY/WCA_ACCENT_POLICY.
        Some(unsafe {
            std::mem::transmute::<unsafe extern "system" fn() -> isize, SetWindowCompositionAttribute>(
                proc,
            )
        })
    })
}

/// Apply (or clear) the transparency effect on a single window. Safe to call
/// on any platform; on non-Windows targets it is a no-op.
pub fn apply_to_window(window: &WebviewWindow, mode: WindowTransparency, acrylic_blur: bool) {
    #[cfg(windows)]
    {
        // Windows 7 does not support these native backdrop effects. Applying
        // or clearing them can interfere with repainting its opaque windows.
        if should_skip_native_backdrop(crate::platform::windows_version::current_windows_version())
        {
            return;
        }

        use window_vibrancy::{clear_acrylic, clear_mica};
        // Always clear stale native backdrops first so toggling the material
        // switch or returning to an opaque window does not stack effects.
        let _ = clear_mica(window);
        let _ = clear_acrylic(window);
        let _ = clear_legacy_acrylic(window);
        let result = match mode {
            WindowTransparency::Opaque => Ok::<(), String>(()),
            WindowTransparency::Transparent if should_apply_acrylic_blur(mode, acrylic_blur) => {
                apply_legacy_acrylic(window)
            }
            WindowTransparency::Transparent => Ok(()),
        };
        if let Err(error) = result {
            tracing::warn!(
                window_label = window.label(),
                mode = ?mode,
                "Failed to apply window transparency: {error}"
            );
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (window, mode, acrylic_blur);
    }
}

#[cfg(windows)]
fn apply_legacy_acrylic(window: &WebviewWindow) -> Result<(), String> {
    set_legacy_acrylic(
        window,
        LegacyAccentState::EnableAcrylicBlurBehind,
        Some(legacy_acrylic_activation_color()),
    )
}

#[cfg(windows)]
fn clear_legacy_acrylic(window: &WebviewWindow) -> Result<(), String> {
    set_legacy_acrylic(window, LegacyAccentState::Disabled, None)
}

#[cfg(windows)]
fn set_legacy_acrylic(
    window: &WebviewWindow,
    accent_state: LegacyAccentState,
    color: Option<window_vibrancy::Color>,
) -> Result<(), String> {
    let hwnd = window
        .hwnd()
        .map_err(|error| format!("failed to get HWND: {error}"))?
        .0 as HWND;

    let set_window_composition_attribute = set_window_composition_attribute()
        .ok_or_else(|| "SetWindowCompositionAttribute is unavailable".to_string())?;

    let mut color = color.unwrap_or_default();
    if matches!(accent_state, LegacyAccentState::EnableAcrylicBlurBehind) && color.3 == 0 {
        // The legacy Acrylic API ignores fully transparent acrylic. Keep one
        // alpha step so the blur-behind effect remains active.
        color.3 = 1;
    }

    let gradient_color = (u32::from(color.0))
        | (u32::from(color.1) << 8)
        | (u32::from(color.2) << 16)
        | (u32::from(color.3) << 24);
    let mut policy = AccentPolicy {
        accent_state: accent_state as u32,
        accent_flags: 0,
        gradient_color,
        animation_id: 0,
    };
    let mut data = WindowCompositionAttribData {
        attrib: 0x13,
        data: &mut policy as *mut _ as *mut c_void,
        data_size: std::mem::size_of::<AccentPolicy>(),
    };

    let success = unsafe { set_window_composition_attribute(hwnd, &mut data) };
    if success == 0 {
        return Err("SetWindowCompositionAttribute failed".to_string());
    }

    Ok(())
}

#[cfg(windows)]
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
enum LegacyAccentState {
    Disabled = 0,
    EnableAcrylicBlurBehind = 4,
}

/// Apply the effect to every main window managed by the app.
pub fn apply_to_all_main_windows(
    app: &tauri::AppHandle,
    mode: WindowTransparency,
    acrylic_blur: bool,
) {
    for window in crate::app::main_windows(app) {
        apply_to_window(&window, mode, acrylic_blur);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acrylic_blur_is_enabled_only_for_transparent_windows_when_switch_is_on() {
        assert!(should_apply_acrylic_blur(
            WindowTransparency::Transparent,
            true
        ));
        assert!(!should_apply_acrylic_blur(
            WindowTransparency::Transparent,
            false
        ));
        assert!(!should_apply_acrylic_blur(WindowTransparency::Opaque, true));
    }

    #[test]
    fn native_backdrop_is_skipped_only_on_windows_7() {
        use crate::platform::windows_version::WindowsVersion;

        let version = |major, minor, build| WindowsVersion {
            major,
            minor,
            build,
            workstation: true,
        };

        assert!(should_skip_native_backdrop(Some(version(6, 1, 7601))));
        assert!(!should_skip_native_backdrop(Some(version(6, 2, 9200))));
        assert!(!should_skip_native_backdrop(Some(version(10, 0, 19045))));
        assert!(!should_skip_native_backdrop(Some(version(10, 0, 26100))));
        assert!(!should_skip_native_backdrop(None));
    }

    #[test]
    fn transparency_state_is_derived_from_opacity() {
        assert_eq!(
            WindowTransparency::from_settings("none", 0.2),
            WindowTransparency::Transparent
        );
        assert_eq!(
            WindowTransparency::from_settings("transparent", 0.0),
            WindowTransparency::Transparent
        );
        assert_eq!(
            WindowTransparency::from_settings("acrylic", 1.0),
            WindowTransparency::Opaque
        );
        assert_eq!(
            WindowTransparency::from_settings("acrylic", 0.99),
            WindowTransparency::Transparent
        );
        assert_eq!(
            WindowTransparency::from_settings("mica", 0.8),
            WindowTransparency::Transparent
        );
    }
}
