//! inject.rs — penyuntikan input ke OS.
//!
//! Fase 2: input PENA sungguhan (Windows Ink) agar tekanan & tilt terbaca
//! aplikasi gambar (Krita, Photoshop, dll).
//! Fase 1 (SetCursorPos) digantikan sepenuhnya.

/// Satu laporan pena yang siap disuntik: posisi piksel + atribut pena.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenReport {
    pub x: i32,
    pub y: i32,
    /// 0.0..1.0, sudah lewat kurva tekanan.
    pub pressure01: f32,
    pub tilt_x: i32, // derajat -90..90
    pub tilt_y: i32, // derajat -90..90
    /// Pena dalam jangkauan digitizer (hover / menyentuh).
    pub in_range: bool,
    /// Ujung menyentuh permukaan.
    pub contact: bool,
    pub barrel: bool,
    pub eraser: bool,
}

/// Target suntikan. Diuji dengan MockInjector; di Windows pakai WinInjector.
///
/// Sengaja TANPA bound Send: handle Windows (HSYNTHETICPOINTERDEVICE)
/// berisi raw pointer dan hanya dipakai dari satu thread per koneksi.
pub trait Injector {
    /// Suntikkan satu laporan pena.
    fn pen(&mut self, r: &PenReport);
    /// Ukuran layar dalam piksel.
    fn screen_size(&self) -> (i32, i32);
    /// Posisi kursor saat ini (untuk awal mode relatif). Default: tengah layar.
    fn cursor_pos(&self) -> (i32, i32) {
        let (w, h) = self.screen_size();
        (w / 2, h / 2)
    }
}

// ---------------------------------------------------------------------------
// Windows: synthetic pointer device, tipe PEN (Windows Ink).
// Tekanan 0..1024, tilt derajat, barrel/eraser -> aplikasi WinInk.
// ---------------------------------------------------------------------------
#[cfg(windows)]
pub mod win {
    use super::{Injector, PenReport};
    use windows::Win32::Foundation::{HWND, POINT, RECT};
    use windows::Win32::UI::Controls::{
        CreateSyntheticPointerDevice, DestroySyntheticPointerDevice, HSYNTHETICPOINTERDEVICE,
        POINTER_FEEDBACK_INDIRECT, POINTER_TYPE_INFO,
    };
    use windows::Win32::UI::Input::Pointer::{
        InjectSyntheticPointerInput, POINTER_FLAG_DOWN, POINTER_FLAG_FIRSTBUTTON,
        POINTER_FLAG_INCONTACT, POINTER_FLAG_INRANGE, POINTER_FLAG_UP, POINTER_FLAG_UPDATE,
        POINTER_INFO, POINTER_PEN_INFO,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, GetSystemMetrics, PEN_FLAG_BARREL, PEN_FLAG_ERASER, PEN_FLAG_NONE,
        PEN_MASK_PRESSURE, PEN_MASK_TILT_X, PEN_MASK_TILT_Y, PT_PEN, SM_CXVIRTUALSCREEN,
        SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    };

    pub struct WinInjector {
        device: HSYNTHETICPOINTERDEVICE,
        display_rect: RECT,
        frame_id: u32,
        was_contact: bool,
    }

    impl WinInjector {
        pub fn new() -> Self {
            // INDIRECT = tanpa ripple visual bawaan (tangan ada di HP, bukan di layar).
            let device = unsafe {
                CreateSyntheticPointerDevice(PT_PEN, 1, POINTER_FEEDBACK_INDIRECT)
                    .expect("CreateSyntheticPointerDevice gagal — butuh Windows 8+")
            };
            let (ox, oy, vw, vh) = unsafe {
                (
                    GetSystemMetrics(SM_XVIRTUALSCREEN),
                    GetSystemMetrics(SM_YVIRTUALSCREEN),
                    GetSystemMetrics(SM_CXVIRTUALSCREEN),
                    GetSystemMetrics(SM_CYVIRTUALSCREEN),
                )
            };
            eprintln!("pen device dibuat, area virtual {vw}x{vh}+{ox}+{oy}");
            Self {
                device,
                display_rect: RECT {
                    left: ox,
                    top: oy,
                    right: ox + vw,
                    bottom: oy + vh,
                },
                frame_id: 0,
                was_contact: false,
            }
        }

        fn inject(&mut self, r: &PenReport) {
            // Rakit flags dari transisi kontak (DOWN/UPDATE/UP).
            let mut flags = POINTER_FLAG_INRANGE;
            if r.contact {
                flags |= POINTER_FLAG_INCONTACT | POINTER_FLAG_FIRSTBUTTON;
                flags |= if self.was_contact {
                    POINTER_FLAG_UPDATE
                } else {
                    POINTER_FLAG_DOWN
                };
            } else if self.was_contact {
                flags |= POINTER_FLAG_UP;
            } else {
                flags |= POINTER_FLAG_UPDATE; // gerak hover
            }
            self.was_contact = r.contact;
            self.frame_id = self.frame_id.wrapping_add(1);

            let mut pen_flags = PEN_FLAG_NONE;
            if r.barrel {
                pen_flags |= PEN_FLAG_BARREL;
            }
            if r.eraser {
                pen_flags |= PEN_FLAG_ERASER;
            }

            let info = POINTER_INFO {
                pointerType: PT_PEN,
                pointerId: 1,
                frameId: self.frame_id,
                pointerFlags: flags,
                sourceDevice: Default::default(),
                hwndTarget: HWND::default(),
                ptPixelLocation: POINT { x: r.x, y: r.y },
                ptHimetricLocation: POINT { x: 0, y: 0 },
                ptPixelLocationRaw: POINT { x: r.x, y: r.y },
                ptHimetricLocationRaw: POINT { x: 0, y: 0 },
                dwTime: 0,
                historyCount: 0,
                InputData: 0,
                dwKeyStates: 0,
                PerformanceCount: 0,
                ButtonChangeType: Default::default(),
            };
            let pen_info = POINTER_PEN_INFO {
                pointerInfo: info,
                penFlags: pen_flags,
                penMask: PEN_MASK_PRESSURE | PEN_MASK_TILT_X | PEN_MASK_TILT_Y,
                pressure: (r.pressure01.clamp(0.0, 1.0) * 1024.0).round() as u32,
                rotation: 0,
                tiltX: r.tilt_x.clamp(-90, 90),
                tiltY: r.tilt_y.clamp(-90, 90),
            };
            let mut type_info = POINTER_TYPE_INFO {
                r#type: PT_PEN,
                Anonymous: unsafe { std::mem::zeroed() },
            };
            unsafe {
                type_info.Anonymous.penInfo = pen_info;
                if let Err(e) = InjectSyntheticPointerInput(self.device, &[type_info]) {
                    eprintln!("InjectSyntheticPointerInput gagal: {e:?}");
                }
            }
        }
    }

    impl Injector for WinInjector {
        fn pen(&mut self, r: &PenReport) {
            self.inject(r);
        }

        fn screen_size(&self) -> (i32, i32) {
            (
                self.display_rect.right - self.display_rect.left,
                self.display_rect.bottom - self.display_rect.top,
            )
        }

        fn cursor_pos(&self) -> (i32, i32) {
            let mut pt = POINT { x: 0, y: 0 };
            unsafe {
                if GetCursorPos(&mut pt).is_ok() {
                    return (pt.x, pt.y);
                }
            }
            let (w, h) = self.screen_size();
            (w / 2, h / 2)
        }
    }

    impl Drop for WinInjector {
        fn drop(&mut self) {
            unsafe { DestroySyntheticPointerDevice(self.device) };
        }
    }
}

// ---------------------------------------------------------------------------
// Non-Windows / pengujian: catat laporan pena, jangan sentuh OS.
// ---------------------------------------------------------------------------
#[cfg(not(windows))]
pub mod mock {
    use super::{Injector, PenReport};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    pub struct MockInjector {
        pub reports: Arc<Mutex<Vec<PenReport>>>,
        pub screen: (i32, i32),
    }

    impl MockInjector {
        pub fn with_screen(w: i32, h: i32) -> Self {
            Self {
                reports: Arc::new(Mutex::new(Vec::new())),
                screen: (w, h),
            }
        }
        pub fn take_reports(&self) -> Vec<PenReport> {
            std::mem::take(&mut *self.reports.lock().unwrap())
        }
    }

    impl Injector for MockInjector {
        fn pen(&mut self, r: &PenReport) {
            self.reports.lock().unwrap().push(*r);
        }
        fn screen_size(&self) -> (i32, i32) {
            self.screen
        }
    }
}
