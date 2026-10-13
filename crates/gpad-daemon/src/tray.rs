//! tray.rs — ikon system tray Windows (Fase 3).
//! Menu: ganti mode absolut/relatif + keluar. Tooltip menampilkan mode aktif.

use crate::MODE;
use gpad_protocol::{MODE_ABSOLUTE, MODE_RELATIVE};
use std::sync::atomic::Ordering;
use tray_icon::{
    Icon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

/// Ikon 32x32: kotak teal dengan titik pena putih. Dibuat saat runtime,
// agar tidak perlu file aset.
fn make_icon() -> Icon {
    const S: usize = 32;
    let mut rgba = vec![0u8; S * S * 4];
    for y in 0..S {
        for x in 0..S {
            let i = (y * S + x) * 4;
            // lingkaran teal
            let dx = x as f32 - 15.5;
            let dy = y as f32 - 15.5;
            if dx * dx + dy * dy < 14.0 * 14.0 {
                rgba[i] = 0x14; // R
                rgba[i + 1] = 0x8A; // G
                rgba[i + 2] = 0x8A; // B
                rgba[i + 3] = 0xFF; // A
                // titik putih tengah (mata pena)
                if dx * dx + dy * dy < 4.5 * 4.5 {
                    rgba[i] = 0xFF;
                    rgba[i + 1] = 0xFF;
                    rgba[i + 2] = 0xFF;
                }
            }
        }
    }
    Icon::from_rgba(rgba, 32, 32).expect("ikon")
}

fn mode_name() -> &'static str {
    if MODE.load(Ordering::Relaxed) == MODE_ABSOLUTE {
        "Absolut"
    } else {
        "Relatif"
    }
}

/// Jalankan tray di thread ini (blocking sampai Quit).
pub fn run() {
    let menu = Menu::new();
    let mode_item = MenuItem::new(format!("Mode: {} (klik: ganti)", mode_name()), true, None);
    let quit_item = MenuItem::new("Keluar", true, None);
    if let Err(e) = menu.append_items(&[
        &mode_item,
        &PredefinedMenuItem::separator(),
        &quit_item,
    ]) {
        eprintln!("tray menu gagal: {e:?}");
        return;
    }

    let tray = match TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(format!("Gpad — mode {}", mode_name()))
        .with_icon(make_icon())
        .build()
    {
        Ok(t) => t,
        Err(e) => {
            eprintln!("tray gagal dibuat: {e:?}");
            return;
        }
    };
    eprintln!("tray aktif");

    let mode_id = mode_item.id().clone();
    let quit_id = quit_item.id().clone();

    while let Ok(ev) = MenuEvent::receiver().recv() {
        if ev.id == quit_id {
            eprintln!("keluar via tray");
            std::process::exit(0);
        } else if ev.id == mode_id {
            let cur = MODE.load(Ordering::Relaxed);
            let next = if cur == MODE_ABSOLUTE { MODE_RELATIVE } else { MODE_ABSOLUTE };
            MODE.store(next, Ordering::Relaxed);
            let name = mode_name();
            let _ = mode_item.set_text(format!("Mode: {name} (klik: ganti)"));
            let _ = tray.set_tooltip(Some(format!("Gpad — mode {name}")));
            eprintln!("mode -> {name} (via tray)");
        }
    }
}
