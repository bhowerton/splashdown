//! Apps: the one in front, bringing one to the front, the apps running,
//! and an app's name and icon from its program. The Windows twin of
//! `mac/apps.rs`.

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GetDIBits,
};
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, EnumWindows, GetForegroundWindow, GetIconInfo, HICON, ICONINFO, IsIconic,
    IsWindowVisible, PrivateExtractIconsW, SW_RESTORE, SetForegroundWindow, ShowWindow,
};
use windows::core::{BOOL, PCWSTR, PWSTR};

use super::registry::*;
use super::uia::*;
use super::{from_wide, wide};
use crate::PlatformError;

/// Full path of a process's executable.
pub(crate) fn exe_path(pid: u32) -> Option<String> {
    // SAFETY: plain values; the handle is closed below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: `process` is open with the access this needs; `buf` holds
    // `len` UTF-16 units and `len` is a valid in/out pointer.
    let ok = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
    };
    // SAFETY: `process` was opened above and is closed once.
    let _ = unsafe { CloseHandle(process) };
    ok.ok()?;
    Some(from_wide(&buf[..(len as usize).min(buf.len())]))
}

/// The executable's "File description" (e.g. "Google Chrome"), the closest
/// Windows has to an app's display name.
pub(crate) fn file_description(path: &str) -> Option<String> {
    let path_w = wide(path);
    // SAFETY: `path_w` is a NUL-terminated path that outlives the call.
    let size = unsafe { GetFileVersionInfoSizeW(PCWSTR(path_w.as_ptr()), None) };
    if size == 0 {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    // SAFETY: `data` is writable for the `size` bytes passed.
    unsafe {
        GetFileVersionInfoW(
            PCWSTR(path_w.as_ptr()),
            None,
            size,
            data.as_mut_ptr().cast(),
        )
    }
    .ok()?;

    let query = |sub: &str| -> Option<(*const u8, usize)> {
        let sub = wide(sub);
        let mut ptr = std::ptr::null_mut();
        let mut len = 0u32;
        // SAFETY: `data` is the version block filled in above and `sub` a
        // NUL-terminated query; `ptr` and `len` are valid out-pointers. The
        // result points into `data`.
        let ok = unsafe {
            VerQueryValueW(
                data.as_ptr().cast(),
                PCWSTR(sub.as_ptr()),
                &mut ptr,
                &mut len,
            )
        };
        (ok.as_bool() && !ptr.is_null()).then_some((ptr.cast_const().cast::<u8>(), len as usize))
    };

    // The first language/code page pair, else US English / Unicode.
    let translation = query("\\VarFileInfo\\Translation")
        .filter(|&(_, len)| len >= 4)
        .map(|(ptr, _)| {
            // SAFETY: at least 4 bytes inside `data`; read unaligned since
            // VerQueryValueW makes no alignment promise.
            let pair = unsafe { std::ptr::read_unaligned(ptr.cast::<[u16; 2]>()) };
            format!("{:04x}{:04x}", pair[0], pair[1])
        })
        .unwrap_or_else(|| "040904b0".into());
    let (ptr, len) = query(&format!("\\StringFileInfo\\{translation}\\FileDescription"))?;
    // `len` counts UTF-16 units, including the terminator.
    let units: Vec<u16> = (0..len)
        // SAFETY: the string's `len` units lie inside `data`; read
        // unaligned for the same reason as above.
        .map(|i| unsafe { std::ptr::read_unaligned(ptr.cast::<u16>().add(i)) })
        .collect();
    let name = from_wide(&units).trim().to_string();
    (!name.is_empty()).then_some(name)
}

pub(super) fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// The foreground window's app, named like `capture_focus` names it.
pub(crate) fn frontmost_app() -> Option<(i32, String, String)> {
    // SAFETY: takes no arguments; may return a null window.
    let window = unsafe { GetForegroundWindow() };
    if window.is_invalid() {
        return None;
    }
    let pid = window_pid(window)?;
    let path = exe_path(pid);
    let id = path
        .as_deref()
        .map(|p| file_name(p).to_string())
        .unwrap_or_default();
    let name = path
        .map(|path| {
            file_description(&path).unwrap_or_else(|| {
                let file = file_name(&path);
                file.strip_suffix(".exe").unwrap_or(file).to_string()
            })
        })
        .unwrap_or_default();
    Some((pid as i32, name, id))
}

/// Brings the app's frontmost visible window forward. Windows allows this
/// because the foreground process (us) is the one asking.
pub(crate) fn activate_app(pid: i32) -> crate::Result<()> {
    let window = u32::try_from(pid)
        .ok()
        .and_then(top_window)
        .ok_or_else(|| PlatformError::Failed("that app has no open window".into()))?;
    // SAFETY: `window` came from EnumWindows just now; one closed since
    // makes these calls fail, nothing worse.
    let shown = unsafe {
        if IsIconic(window).as_bool() {
            let _ = ShowWindow(window, SW_RESTORE);
        }
        SetForegroundWindow(window).as_bool()
    };
    if shown {
        Ok(())
    } else {
        Err(PlatformError::Failed("couldn't switch to that app".into()))
    }
}

/// The first visible top-level window of `pid` in Z order (its frontmost).
pub(super) fn top_window(pid: u32) -> Option<HWND> {
    struct Search {
        pid: u32,
        found: Option<HWND>,
    }
    unsafe extern "system" fn visit(window: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` points at the `Search` below, which outlives
        // EnumWindows.
        let search = unsafe { &mut *(lparam.0 as *mut Search) };
        if is_app_window(window) && window_pid(window) == Some(search.pid) {
            search.found = Some(window);
            return BOOL(0);
        }
        BOOL(1)
    }
    let mut search = Search { pid, found: None };
    // SAFETY: `visit` matches WNDENUMPROC and `search` lives until
    // EnumWindows returns. Stopping early reports an error, which is ignored.
    let _ = unsafe { EnumWindows(Some(visit), LPARAM(&raw mut search as isize)) };
    search.found
}

/// A window people switch to: visible, not a tool window (palettes,
/// floating bars) and not owned by another (dialogs, popups).
pub(super) fn is_app_window(window: HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        GW_OWNER, GWL_EXSTYLE, GetWindow, GetWindowLongPtrW, WS_EX_TOOLWINDOW,
    };
    // SAFETY: `window` is a live handle; these only read its state.
    unsafe {
        IsWindowVisible(window).as_bool()
            && GetWindow(window, GW_OWNER).is_err()
            && (GetWindowLongPtrW(window, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0) == 0
    }
}

/// Apps with a window open, by the names people know them by (the
/// program's description, "Notepad", and its file name, "notepad"), with
/// their process ids: for "switch to …", which mustn't start another copy.
pub(crate) fn running_apps() -> Vec<(String, i32)> {
    unsafe extern "system" fn visit(window: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` points at the Vec below, which outlives
        // EnumWindows.
        let pids = unsafe { &mut *(lparam.0 as *mut Vec<u32>) };
        if is_app_window(window)
            && let Some(pid) = window_pid(window)
            && !pids.contains(&pid)
        {
            pids.push(pid);
        }
        BOOL(1)
    }
    let mut pids: Vec<u32> = Vec::new();
    // SAFETY: `visit` matches WNDENUMPROC and `pids` lives until
    // EnumWindows returns.
    let _ = unsafe { EnumWindows(Some(visit), LPARAM(&raw mut pids as isize)) };
    let mut apps = Vec::new();
    for pid in pids {
        let Some(path) = exe_path(pid) else {
            continue;
        };
        let Ok(pid) = i32::try_from(pid) else {
            continue;
        };
        if let Some(description) = file_description(&path) {
            apps.push((description, pid));
        }
        if let Some(stem) = std::path::Path::new(&path).file_stem() {
            apps.push((stem.to_string_lossy().into_owned(), pid));
        }
    }
    apps
}

/// The display name of an app known by its executable name: from its "App
/// Paths" registration, else from a running copy. No icon yet; Settings
/// shows a letter instead.
pub(crate) fn app_display(app_id: &str) -> Option<crate::AppDisplay> {
    let key = format!("Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{app_id}");
    let path = reg_string(HKEY_CURRENT_USER, &key, None)
        .or_else(|| reg_string(HKEY_LOCAL_MACHINE, &key, None))
        .map(|p| p.trim_matches('"').to_string())
        .or_else(|| running_exe(app_id))?;
    Some(crate::AppDisplay {
        name: file_description(&path)?,
        icon_png: icon_png(&path),
    })
}

/// Pixels per side of an app icon in Settings: sharp on high-DPI screens.
pub(super) const ICON_PX: i32 = 64;

/// The executable's own icon as a PNG.
pub(super) fn icon_png(exe: &str) -> Option<Vec<u8>> {
    let mut name = [0u16; 260];
    let wide: Vec<u16> = exe.encode_utf16().collect();
    name.get_mut(..wide.len())?.copy_from_slice(&wide);
    if wide.len() == name.len() {
        return None;
    }
    let mut icon = [HICON::default()];
    // SAFETY: `name` is a NUL-terminated path; one handle is written into
    // `icon`, destroyed below.
    let found =
        unsafe { PrivateExtractIconsW(&name, 0, ICON_PX, ICON_PX, Some(&mut icon), None, 0) };
    if found == 0 || found == u32::MAX || icon[0].is_invalid() {
        return None;
    }
    let pixels = icon_pixels(icon[0]);
    // SAFETY: extracted above, not used after this.
    let _ = unsafe { DestroyIcon(icon[0]) };
    encode_png(&pixels?, ICON_PX as u32)
}

/// An icon's pixels, RGBA, top row first.
pub(super) fn icon_pixels(icon: HICON) -> Option<Vec<u8>> {
    let mut info = ICONINFO::default();
    // SAFETY: `info` is writable; the bitmaps it gets are deleted below.
    unsafe { GetIconInfo(icon, &mut info) }.ok()?;
    let mut pixels = vec![0u8; (ICON_PX * ICON_PX * 4) as usize];
    // The mask: white where an icon without alpha is see-through.
    let mut mask = vec![0u8; pixels.len()];
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: ICON_PX,
            // Negative: top row first.
            biHeight: -ICON_PX,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: a memory DC for the copy, deleted right after; `pixels`
    // holds the 32-bit rows `bmi` describes.
    let (lines, mask_lines) = unsafe {
        let dc = CreateCompatibleDC(None);
        let lines = GetDIBits(
            dc,
            info.hbmColor,
            0,
            ICON_PX as u32,
            Some(pixels.as_mut_ptr().cast()),
            &mut bmi,
            DIB_RGB_COLORS,
        );
        let mut mask_bmi = bmi;
        let mask_lines = GetDIBits(
            dc,
            info.hbmMask,
            0,
            ICON_PX as u32,
            Some(mask.as_mut_ptr().cast()),
            &mut mask_bmi,
            DIB_RGB_COLORS,
        );
        let _ = DeleteDC(dc);
        let _ = DeleteObject(info.hbmColor.into());
        let _ = DeleteObject(info.hbmMask.into());
        (lines, mask_lines)
    };
    if lines != ICON_PX {
        return None;
    }
    // BGRA to RGBA. An icon with no alpha at all (an older one) is opaque
    // except where its mask is white.
    let has_alpha = pixels.as_chunks::<4>().0.iter().any(|p| p[3] != 0);
    let masked = mask_lines == ICON_PX;
    for (p, m) in pixels
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(mask.as_chunks::<4>().0)
    {
        p.swap(0, 2);
        if !has_alpha {
            p[3] = if masked && m[0] != 0 { 0 } else { 255 };
        }
    }
    Some(pixels)
}

pub(super) fn encode_png(rgba: &[u8], side: u32) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, side, side);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().ok()?.write_image_data(rgba).ok()?;
    Some(out)
}

/// The full path of a running app with this executable name, found through
/// the top-level windows.
pub(super) fn running_exe(app_id: &str) -> Option<String> {
    struct Search<'a> {
        app_id: &'a str,
        found: Option<String>,
    }
    unsafe extern "system" fn visit(window: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` points at the `Search` below, which outlives
        // EnumWindows.
        let search = unsafe { &mut *(lparam.0 as *mut Search) };
        // SAFETY: `window` is a live handle from the enumeration.
        let visible = unsafe { IsWindowVisible(window) }.as_bool();
        if let Some(path) = visible
            .then(|| window_pid(window).and_then(exe_path))
            .flatten()
            && file_name(&path).eq_ignore_ascii_case(search.app_id)
        {
            search.found = Some(path);
            return BOOL(0);
        }
        BOOL(1)
    }
    let mut search = Search {
        app_id,
        found: None,
    };
    // SAFETY: `visit` matches WNDENUMPROC and `search` lives until
    // EnumWindows returns. Stopping early reports an error, which is ignored.
    let _ = unsafe { EnumWindows(Some(visit), LPARAM(&raw mut search as isize)) };
    search.found
}

/// Makes this program the one that opens `scheme://` links for the
/// signed-in user: a click then starts it with the link as its argument.
/// Written again at each start, so it follows the program when it moves.
pub(crate) fn register_link_scheme(scheme: &str, name: &str) -> crate::Result<()> {
    let exe = std::env::current_exe().map_err(|e| PlatformError::Failed(e.to_string()))?;
    let exe = exe
        .to_str()
        .ok_or_else(|| PlatformError::Failed("the program's path isn't text".into()))?;
    let class = format!("Software\\Classes\\{scheme}");
    let written = reg_set_string(HKEY_CURRENT_USER, &class, None, &format!("URL:{name}"))
        // Its presence is what makes the class a link type.
        && reg_set_string(HKEY_CURRENT_USER, &class, Some("URL Protocol"), "")
        && reg_set_string(
            HKEY_CURRENT_USER,
            &format!("{class}\\shell\\open\\command"),
            None,
            &format!("\"{exe}\" \"%1\""),
        );
    if written {
        Ok(())
    } else {
        Err(PlatformError::Failed(
            "couldn't write the link type to the registry".into(),
        ))
    }
}
