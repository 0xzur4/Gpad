//! mapping.rs — konversi koordinat 0.0..1.0 dari ponsel ke piksel layar PC.
//!
//! Fase 3:
//!  - Mode absolut dengan area terpilih: seluruh layar | monitor:N | custom.
//!    Opsi keep_aspect agar lingkaran tidak jadi elips.
//!  - Mode relatif: yang dihitung DELTA gerak (seperti touchpad), dengan
//!    sensitivitas dan reset acuan saat stylus diangkat.

/// Area pemetaan untuk mode absolut (piksel, koordinat virtual screen).
#[derive(Clone, Copy, Debug)]
pub struct Area {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Selesaikan area dari string config ("screen" | "monitor:N" | "custom").
/// screen_w/h = ukuran virtual screen; custom = rect dari config.
pub fn resolve_area(spec: &str, screen_w: i32, screen_h: i32, custom: (i32, i32, i32, i32)) -> Area {
    if let Some(n) = spec.strip_prefix("monitor:") {
        if let Ok(idx) = n.parse::<usize>() {
            if let Some(m) = monitors().get(idx) {
                return Area { x: m.0, y: m.1, w: m.2, h: m.3 };
            }
            eprintln!("monitor:{idx} tidak ada — pakai seluruh layar");
        }
    }
    if spec == "custom" && custom.2 > 0 && custom.3 > 0 {
        return Area { x: custom.0, y: custom.1, w: custom.2, h: custom.3 };
    }
    Area { x: 0, y: 0, w: screen_w, h: screen_h }
}

/// Daftar monitor: (x, y, w, h) per monitor.
#[cfg(windows)]
pub fn monitors() -> Vec<(i32, i32, i32, i32)> {
    use windows::Win32::Foundation::{BOOL, LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};

    struct Ctx(Vec<(i32, i32, i32, i32)>);
    unsafe extern "system" fn cb(
        _hmon: HMONITOR,
        _hdc: HDC,
        rect: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let ctx = &mut *(data.0 as *mut Ctx);
        let r = &*rect;
        ctx.0.push((r.left, r.top, r.right - r.left, r.bottom - r.top));
        BOOL(1)
    }

    let mut ctx = Ctx(Vec::new());
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(cb),
            LPARAM(&mut ctx as *mut Ctx as isize),
        );
    }
    ctx.0
}

#[cfg(not(windows))]
pub fn monitors() -> Vec<(i32, i32, i32, i32)> {
    Vec::new() // diuji via custom/screen
}

/// Mode absolut: titik ponsel -> titik area.
/// keep_aspect: sesuaikan agar rasio aspek ponsel terjaga (letterbox).
pub fn absolute(
    x: f32,
    y: f32,
    area: Area,
    keep_aspect: bool,
    phone_aspect: f32, // width/height area gambar di HP
) -> (i32, i32) {
    let (ox, oy, w, h) = if keep_aspect && phone_aspect > 0.0 {
        let area_aspect = area.w as f32 / area.h.max(1) as f32;
        if (phone_aspect - area_aspect).abs() < 0.01 {
            (area.x as f32, area.y as f32, area.w as f32, area.h as f32)
        } else if phone_aspect > area_aspect {
            // HP lebih lebar: batasi tinggi, tengahkan vertikal.
            let h2 = area.w as f32 / phone_aspect;
            (area.x as f32, area.y as f32 + (area.h as f32 - h2) / 2.0, area.w as f32, h2)
        } else {
            // HP lebih tinggi: batasi lebar, tengahkan horizontal.
            let w2 = area.h as f32 * phone_aspect;
            (area.x as f32 + (area.w as f32 - w2) / 2.0, area.y as f32, w2, area.h as f32)
        }
    } else {
        (area.x as f32, area.y as f32, area.w as f32, area.h as f32)
    };
    let px = (ox + x.clamp(0.0, 1.0) * w).round() as i32;
    let py = (oy + y.clamp(0.0, 1.0) * h).round() as i32;
    (
        px.clamp(area.x, area.x + area.w - 1),
        py.clamp(area.y, area.y + area.h - 1),
    )
}

/// Keadaan mode relatif: acuan sentuh + posisi kursor virtual.
pub struct RelativeState {
    anchor: Option<(f32, f32)>,
    pos: (f32, f32),
    bounds: Area, // kursor di-clamp ke area ini
}

impl RelativeState {
    pub fn new(start_x: i32, start_y: i32, bounds: Area) -> Self {
        Self {
            anchor: None,
            pos: (start_x as f32, start_y as f32),
            bounds,
        }
    }

    /// Proses satu event sentuh. Mengembalikan posisi kursor (piksel).
    /// - down: catat acuan, kursor diam.
    /// - move: delta dari acuan * sensitivitas -> geser kursor; acuan = titik kini.
    /// - up: lepas acuan (reset agar tidak lompat saat sentuh berikutnya).
    pub fn event(&mut self, down: bool, up: bool, tx: f32, ty: f32, sensitivity: f32) -> (i32, i32) {
        if up {
            self.anchor = None;
        } else if down && self.anchor.is_none() {
            self.anchor = Some((tx, ty));
        } else if let Some((ax, ay)) = self.anchor {
            let dx = (tx - ax) * sensitivity * self.bounds.w as f32;
            let dy = (ty - ay) * sensitivity * self.bounds.h as f32;
            self.pos.0 = (self.pos.0 + dx).clamp(
                self.bounds.x as f32,
                (self.bounds.x + self.bounds.w - 1) as f32,
            );
            self.pos.1 = (self.pos.1 + dy).clamp(
                self.bounds.y as f32,
                (self.bounds.y + self.bounds.h - 1) as f32,
            );
            self.anchor = Some((tx, ty));
        }
        (self.pos.0.round() as i32, self.pos.1.round() as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_basic_and_clamp() {
        let a = Area { x: 0, y: 0, w: 1920, h: 1080 };
        assert_eq!(absolute(0.0, 0.0, a, false, 1.0), (0, 0));
        assert_eq!(absolute(1.0, 1.0, a, false, 1.0), (1919, 1079));
        assert_eq!(absolute(0.5, 0.5, a, false, 1.0), (960, 540));
        assert_eq!(absolute(-0.5, 2.0, a, false, 1.0), (0, 1079));
    }

    #[test]
    fn absolute_custom_area() {
        let a = resolve_area("custom", 1920, 1080, (100, 100, 800, 600));
        assert_eq!((a.x, a.y, a.w, a.h), (100, 100, 800, 600));
        assert_eq!(absolute(0.0, 0.0, a, false, 1.0), (100, 100));
        assert_eq!(absolute(1.0, 1.0, a, false, 1.0), (899, 699));
    }

    #[test]
    fn keep_aspect_letterboxes() {
        // Area 16:9, HP portrait 9:19.5 -> lebar dibatasi, lingkaran tetap lingkaran.
        let a = Area { x: 0, y: 0, w: 1920, h: 1080 };
        let (x0, _) = absolute(0.0, 0.5, a, true, 9.0 / 19.5);
        let (x1, _) = absolute(1.0, 0.5, a, true, 9.0 / 19.5);
        let w_used = x1 - x0;
        // lebar terpakai = 1080 * (9/19.5) ≈ 498
        assert!((w_used - 498).abs() <= 2, "w_used={w_used}");
        // tanpa keep_aspect: penuh 1920
        let (x0f, _) = absolute(0.0, 0.5, a, false, 9.0 / 19.5);
        let (x1f, _) = absolute(1.0, 0.5, a, false, 9.0 / 19.5);
        assert_eq!(x1f - x0f, 1919);
    }

    #[test]
    fn relative_drag_and_lift() {
        let bounds = Area { x: 0, y: 0, w: 1920, h: 1080 };
        let mut r = RelativeState::new(960, 540, bounds);
        // down di tengah HP: kursor diam
        assert_eq!(r.event(true, false, 0.5, 0.5, 1.0), (960, 540));
        // geser 10% lebar -> kursor +192px
        assert_eq!(r.event(false, false, 0.6, 0.5, 1.0), (1152, 540));
        // angkat: acuan reset
        assert_eq!(r.event(false, true, 0.6, 0.5, 1.0), (1152, 540));
        // sentuh di tempat BARU: kursor TIDAK lompat
        assert_eq!(r.event(true, false, 0.1, 0.1, 1.0), (1152, 540));
        // geser lagi dari acuan baru
        assert_eq!(r.event(false, false, 0.2, 0.1, 1.0), (1344, 540));
    }

    #[test]
    fn relative_clamps_to_bounds() {
        let bounds = Area { x: 0, y: 0, w: 1920, h: 1080 };
        let mut r = RelativeState::new(1900, 540, bounds);
        r.event(true, false, 0.5, 0.5, 1.0);
        // geser jauh ke kanan: mentok di tepi
        assert_eq!(r.event(false, false, 1.0, 0.5, 2.0), (1919, 540));
    }
}
