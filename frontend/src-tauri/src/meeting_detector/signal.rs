//! Who holds the microphone, from the Windows privacy "capability access" store, plus visible
//! window titles (to tell a Meet tab from other browser mic use).

use windows::core::{BOOL, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible};
use winreg::{enums::HKEY_CURRENT_USER, RegKey};

const STORE: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";

fn push_entries(key: &RegKey, out: &mut Vec<(String, u64, u64)>) {
    for name in key.enum_keys().flatten() {
        if name == "NonPackaged" {
            continue;
        }
        if let Ok(sub) = key.open_subkey(&name) {
            let start = sub.get_value::<u64, _>("LastUsedTimeStart").unwrap_or(0);
            let stop = sub.get_value::<u64, _>("LastUsedTimeStop").unwrap_or(0);
            out.push((name, start, stop));
        }
    }
}

/// (key, LastUsedTimeStart, LastUsedTimeStop) for packaged and NonPackaged apps.
pub fn capability_entries() -> std::io::Result<Vec<(String, u64, u64)>> {
    let root = RegKey::predef(HKEY_CURRENT_USER).open_subkey(STORE)?;
    let mut out = Vec::new();
    push_entries(&root, &mut out);
    if let Ok(np) = root.open_subkey("NonPackaged") {
        push_entries(&np, &mut out);
    }
    Ok(out)
}

fn exe_name(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let r = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(h);
        r.ok()?;
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit('\\').next().map(|s| s.to_ascii_lowercase())
    }
}

/// (exe file name lowercase, title) for visible, titled top-level windows.
pub fn visible_window_titles() -> Vec<(String, String)> {
    unsafe extern "system" fn each(hwnd: HWND, lp: LPARAM) -> BOOL {
        let out = &mut *(lp.0 as *mut Vec<(String, String)>);
        if IsWindowVisible(hwnd).as_bool() {
            let mut buf = [0u16; 512];
            let n = GetWindowTextW(hwnd, &mut buf);
            if n > 0 {
                let mut pid = 0u32;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if let Some(exe) = exe_name(pid) {
                    out.push((exe, String::from_utf16_lossy(&buf[..n as usize])));
                }
            }
        }
        true.into()
    }
    let mut out: Vec<(String, String)> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

/// Meeting apps currently holding the mic. Err only when the store can't be read at all.
pub fn meeting_mic_holders() -> std::io::Result<Vec<&'static str>> {
    let keys = capability_entries()?;
    // Titles only matter when a browser holds the mic; skip the window walk otherwise.
    let browser_holding = keys.iter().any(|(k, s, e)| super::is_holding(*s, *e) && super::browser_exe(k).is_some());
    let windows = if browser_holding { visible_window_titles() } else { Vec::new() };
    Ok(super::meeting_holders(&keys, &windows))
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_the_capability_store() {
        let entries = super::capability_entries().expect("microphone capability store readable");
        assert!(entries.iter().all(|(k, _, _)| !k.is_empty()));
        let windows = super::visible_window_titles();
        for (exe, title) in windows {
            assert!(!title.is_empty(), "every window should have a non-empty title");
            assert!(exe.ends_with(".exe"), "every exe name should end with .exe");
        }
    }
}
