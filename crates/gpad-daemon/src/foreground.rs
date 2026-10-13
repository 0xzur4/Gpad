//! foreground.rs — profil per aplikasi (Windows, Fase 3).
//!
//! Thread memantau jendela foreground tiap detik. Saat aplikasi foreground
//! cocok dengan `profile.<nama>.process` di config, mode/sensitivitas/gamma
//! profil diterapkan. Hanya diterapkan saat aplikasi BERGANTI — saklar manual
//! (tray/hotkey/HP) tidak dilawan.

use crate::{GAMMA_OVERRIDE, MODE, SENS_OVERRIDE};
use gpad_protocol::{MODE_ABSOLUTE, MODE_RELATIVE};
use std::sync::atomic::Ordering;
use std::time::Duration;
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
use windows::core::PWSTR;

fn foreground_exe() -> Option<String> {
    unsafe {
        let hwnd: HWND = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 260];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            h,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(h);
        if ok.is_err() {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit(['\\', '/']).next().map(|s| s.to_lowercase())
    }
}

pub fn run(profiles: Vec<crate::config::Profile>) {
    if profiles.is_empty() {
        return;
    }
    eprintln!("profil: {} terdaftar", profiles.len());
    let mut last_exe = String::new();
    loop {
        std::thread::sleep(Duration::from_secs(1));
        let exe = match foreground_exe() {
            Some(e) if e != last_exe => e,
            _ => continue,
        };
        last_exe = exe.clone();
        // Kembalikan override saat pindah ke aplikasi tanpa profil.
        let matched = profiles.iter().find(|p| p.process.to_lowercase() == exe);
        match matched {
            Some(p) => {
                if !p.mode.is_empty() {
                    let m = if p.mode == "relative" { MODE_RELATIVE } else { MODE_ABSOLUTE };
                    MODE.store(m, Ordering::Relaxed);
                }
                SENS_OVERRIDE.store(
                    p.sensitivity.map(|s| s.to_bits()).unwrap_or(0),
                    Ordering::Relaxed,
                );
                GAMMA_OVERRIDE.store(p.gamma.map(|g| g.to_bits()).unwrap_or(0), Ordering::Relaxed);
                eprintln!("profil aktif untuk {exe}: mode={} sens={:?} gamma={:?}", p.mode, p.sensitivity, p.gamma);
            }
            None => {
                SENS_OVERRIDE.store(0, Ordering::Relaxed);
                GAMMA_OVERRIDE.store(0, Ordering::Relaxed);
            }
        }
    }
}
