//! hotkey.rs — pintasan keyboard global untuk ganti mode (Windows).
//! Berjalan di thread sendiri dengan message loop. Contoh: "ctrl+alt+g".

use crate::MODE;
use gpad_protocol::{MODE_ABSOLUTE, MODE_RELATIVE};
use std::sync::atomic::Ordering;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN, RegisterHotKey, UnregisterHotKey,
    VK_F1,
};
use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};

/// Parse "ctrl+alt+g" -> (modifiers, vk). None jika tidak valid / kosong.
fn parse(spec: &str) -> Option<(HOT_KEY_MODIFIERS, u32)> {
    let spec = spec.trim().to_lowercase();
    if spec.is_empty() {
        return None;
    }
    let mut mods = HOT_KEY_MODIFIERS(0);
    let mut vk: Option<u32> = None;
    for part in spec.split('+') {
        let p = part.trim();
        match p {
            "ctrl" | "control" => mods |= MOD_CONTROL,
            "alt" => mods |= MOD_ALT,
            "shift" => mods |= MOD_SHIFT,
            "win" | "meta" => mods |= MOD_WIN,
            _ if p.len() == 1 => {
                vk = Some(p.chars().next().unwrap().to_ascii_uppercase() as u32);
            }
            _ if p.starts_with('f') => {
                if let Ok(n) = p[1..].parse::<u32>() {
                    if (1..=12).contains(&n) {
                        vk = Some(VK_F1.0 as u32 + (n - 1));
                    }
                }
            }
            _ => return None,
        }
    }
    vk.map(|v| (mods, v))
}

/// Daftarkan hotkey dan jalankan loop sampai proses mati.
pub fn run(spec: &str) {
    let (mods, vk) = match parse(spec) {
        Some(x) => x,
        None => {
            eprintln!("hotkey '{spec}' tidak valid — dinonaktifkan");
            return;
        }
    };
    unsafe {
        if RegisterHotKey(None, 1, mods, vk).is_err() {
            eprintln!("RegisterHotKey gagal untuk '{spec}' — mungkin dipakai aplikasi lain");
            return;
        }
    }
    eprintln!("hotkey ganti mode: {spec}");
    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_HOTKEY {
                let cur = MODE.load(Ordering::Relaxed);
                let next = if cur == MODE_ABSOLUTE { MODE_RELATIVE } else { MODE_ABSOLUTE };
                MODE.store(next, Ordering::Relaxed);
                eprintln!("mode -> {}", if next == MODE_ABSOLUTE { "absolut" } else { "relatif" });
            }
        }
        let _ = UnregisterHotKey(None, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn parses_hotkeys() {
        assert!(parse("ctrl+alt+g").is_some());
        assert!(parse("").is_none());
        assert!(parse("ctrl+foo").is_none());
        let (_, vk) = parse("shift+f5").unwrap();
        assert_eq!(vk, super::VK_F1.0 as u32 + 4);
    }
}
