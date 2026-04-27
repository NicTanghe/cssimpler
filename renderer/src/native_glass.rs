use cssimpler_core::Color;
use winit::window::Window;

#[cfg(target_os = "windows")]
pub(crate) fn apply(window: &Window, tint: Color) -> Result<bool, String> {
    apply_accent(window, ACCENT_ENABLE_ACRYLICBLURBEHIND, tint)?;
    Ok(true)
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn apply(_window: &Window, _tint: Color) -> Result<bool, String> {
    Ok(false)
}

#[cfg(target_os = "windows")]
pub(crate) fn clear(window: &Window) -> Result<(), String> {
    apply_accent(window, ACCENT_DISABLED, Color::rgba(0, 0, 0, 0))
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn clear(_window: &Window) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "windows")]
const ACCENT_DISABLED: u32 = 0;
#[cfg(target_os = "windows")]
const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;

#[cfg(target_os = "windows")]
fn apply_accent(window: &Window, accent_state: u32, tint: Color) -> Result<(), String> {
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};
    use windows_sys::core::BOOL;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    type SetWindowCompositionAttribute =
        unsafe extern "system" fn(HWND, *mut WindowCompositionAttribData) -> BOOL;

    #[repr(C)]
    struct AccentPolicy {
        accent_state: u32,
        accent_flags: u32,
        gradient_color: u32,
        animation_id: u32,
    }

    #[repr(C)]
    struct WindowCompositionAttribData {
        attrib: u32,
        pv_data: *mut c_void,
        cb_data: usize,
    }

    const WCA_ACCENT_POLICY: u32 = 0x13;

    let raw = window
        .window_handle()
        .map_err(|error| error.to_string())?
        .as_raw();
    let hwnd = match raw {
        RawWindowHandle::Win32(handle) => handle.hwnd.get() as HWND,
        _ => return Err("window is not a Win32 window".to_string()),
    };

    let user32 = unsafe { LoadLibraryA(c"user32.dll".as_ptr() as _) };
    if user32.is_null() {
        return Err("failed to load user32.dll".to_string());
    }

    let proc = unsafe { GetProcAddress(user32, c"SetWindowCompositionAttribute".as_ptr() as _) }
        .ok_or_else(|| "SetWindowCompositionAttribute is unavailable".to_string())?;
    let set_window_composition_attribute: SetWindowCompositionAttribute =
        unsafe { std::mem::transmute(proc) };
    let mut policy = AccentPolicy {
        accent_state,
        accent_flags: 0,
        gradient_color: packed_windows_tint(tint),
        animation_id: 0,
    };
    let mut data = WindowCompositionAttribData {
        attrib: WCA_ACCENT_POLICY,
        pv_data: &mut policy as *mut _ as _,
        cb_data: std::mem::size_of::<AccentPolicy>(),
    };

    let ok = unsafe { set_window_composition_attribute(hwnd, &mut data) };
    if ok == 0 {
        return Err("SetWindowCompositionAttribute failed".to_string());
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn packed_windows_tint(tint: Color) -> u32 {
    u32::from(tint.r)
        | (u32::from(tint.g) << 8)
        | (u32::from(tint.b) << 16)
        | (u32::from(tint.a) << 24)
}
