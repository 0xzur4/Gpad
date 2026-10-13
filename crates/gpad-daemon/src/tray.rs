//! tray.rs — ikon system tray Windows (v0.5.0).
//! Menampilkan status koneksi + fps + RTT. Menu: Keluar.
//!
//! Semua update dilakukan di thread tray ini (tray-icon bukan Send).

use crate::status_snapshot;
use std::time::Duration;
use tray_icon::{
    Icon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

/// Ikon 32x32: monitor teal dengan garis pena putih. Dibuat saat runtime.
fn make_icon() -> Icon {
    const S: usize = 32;
    let mut rgba = vec![0u8; S * S * 4];
    for y in 0..S {
        for x in 0..S {
            let i = (y * S + x) * 4;
            // bingkai monitor teal
            let in_frame = x >= 4 && x < 28 && y >= 6 && y < 24;
            let in_screen = x >= 6 && x < 26 && y >= 8 && y < 20;
            if in_frame && !in_screen {
                rgba[i] = 0x14;
                rgba[i + 1] = 0x8A;
                rgba[i + 2] = 0x8A;
                rgba[i + 3] = 0xFF;
            } else if in_screen {
                // layar gelap + garis pena putih diagonal
                rgba[i] = 0x11;
                rgba[i + 1] = 0x22;
                rgba[i + 2] = 0x2A;
                rgba[i + 3] = 0xFF;
                if (x as i32 - y as i32).abs() <= 1 {
                    rgba[i] = 0xFF;
                    rgba[i + 1] = 0xFF;
                    rgba[i + 2] = 0xFF;
                }
            }
        }
    }
    Icon::from_rgba(rgba, 32, 32).expect("ikon")
}

fn status_text() -> String {
    let (st, fps, rtt) = status_snapshot();
    match st {
        2 => format!("Terhubung — {fps} fps, RTT {rtt} ms"),
        1 => "Menghubungkan…".to_string(),
        _ => "Menunggu HP (colok USB, tap Hubungkan)".to_string(),
    }
}

/// Jalankan tray di thread ini (blocking sampai Quit).
pub fn run() {
    let menu = Menu::new();
    let status_item = MenuItem::new(format!("Status: {}", status_text()), false, None);
    let quit_item = MenuItem::new("Keluar", true, None);
    if let Err(e) = menu.append_items(&[&status_item, &PredefinedMenuItem::separator(), &quit_item])
    {
        eprintln!("tray menu gagal: {e:?}");
        return;
    }

    let tray = match TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(format!("Gpad — {}", status_text()))
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

    let quit_id = quit_item.id().clone();
    let mut last_status = status_text();
    loop {
        // recv_timeout: cek event menu, tapi tetap update status tiap 2 dtk.
        match MenuEvent::receiver().recv_timeout(Duration::from_secs(2)) {
            Ok(ev) if ev.id == quit_id => {
                eprintln!("keluar via tray");
                std::process::exit(0);
            }
            Ok(_) => {}
            Err(_) => {
                // timeout — perbarui tampilan status bila berubah
                let cur = status_text();
                if cur != last_status {
                    last_status = cur.clone();
                    let txt = format!("Status: {cur}");
                    let _ = status_item.set_text(txt.clone());
                    let _ = tray.set_tooltip(Some(format!("Gpad — {txt}")));
                }
            }
        }
    }
}
