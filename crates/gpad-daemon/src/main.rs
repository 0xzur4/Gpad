//! gpad-daemon v0.5.0 "Cintiq mode".
//!
//! Arsitektur: HP sebagai pen display.
//! - Transport: Android Open Accessory (AOA) over USB bulk (rusb).
//!   TANPA ADB, TANPA `adb reverse`, TANPA TCP.
//! - Video downstream: DXGI -> scale ≤1080p -> OpenH264 -> USB ->
//!   MediaCodec di HP. 30 fps target.
//! - Input upstream: sentuh HP -> koordinat desktop 0.0..1.0 ->
//!   Windows Ink synthetic pointer (dipakai ulang dari v0.4.x).
//! - Heartbeat 1 detik dua arah + echo RTT; putus >3.5 detik ->
//!   reconnect dengan backoff eksponensial.
//!
//! Tanpa console secara default (tray app); `--console` untuk diagnostik.
//!
//! Loop sesi single-threaded (sengaja): DXGI handle & tray icon bukan Send,
//! dan multiplexing I/O USB memang paling sederhana dalam satu loop.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod config;
mod inject;
mod usb;
mod vidutil;
#[cfg(windows)]
mod tray;
#[cfg(windows)]
mod video;

use config::Config;
use gpad_protocol::{
    decode_heartbeat, encode_heartbeat, Handshake, HandshakeReply, PenPacket, Stats, CH_CONTROL,
    CH_HEARTBEAT, CH_INPUT, CH_STATS, CH_VIDEO, HANDSHAKE_LEN, PACKET_LEN, PROTOCOL_VERSION,
    KIND_DOWN, KIND_HOVER, KIND_MOVE, KIND_UP,
};
use inject::{Injector, PenReport};
use std::sync::atomic::{AtomicU16, AtomicU8, Ordering};
use std::time::{Duration, Instant};

#[cfg(windows)]
use inject::win::WinInjector;
#[cfg(not(windows))]
use inject::mock::MockInjector;

/// Status koneksi global untuk tray: 0=idle, 1=menghubungkan, 2=terhubung.
static STATUS: AtomicU8 = AtomicU8::new(0);
static FPS_OUT: AtomicU8 = AtomicU8::new(0);
static RTT_MS: AtomicU16 = AtomicU16::new(0);

const HB_INTERVAL: Duration = Duration::from_secs(1);
/// Bila echo heartbeat tak kembali dalam ini, anggap mati.
const HB_DEAD_AFTER: Duration = Duration::from_millis(3500);
/// Timeout poll baca USB per iterasi loop (tetap responsif untuk video).
const POLL_TIMEOUT: Duration = Duration::from_millis(10);

fn main() {
    #[allow(unused_variables)]
    let want_console = std::env::args().any(|a| a == "--console");
    #[cfg(windows)]
    if want_console {
        use windows::Win32::System::Console::AllocConsole;
        unsafe {
            let _ = AllocConsole();
        }
    }
    eprintln!(
        "Gpad daemon v{} — Cintiq mode (AOA USB, mirror + pen)",
        env!("CARGO_PKG_VERSION")
    );
    let config = Config::load();

    #[cfg(windows)]
    std::thread::Builder::new()
        .name("gpad-tray".into())
        .spawn(tray::run)
        .expect("spawn tray");

    session_loop(&config);
}

/// Loop abadi: sambung -> sesi -> putus -> backoff -> sambung lagi.
fn session_loop(config: &Config) {
    let mut attempt = 0u32;
    loop {
        STATUS.store(1, Ordering::Relaxed);
        match usb::connect() {
            Ok(link) => {
                eprintln!("USB tersambung.");
                attempt = 0;
                run_session(link, config);
                eprintln!("sesi berakhir — reconnect…");
            }
            Err(e) => {
                eprintln!("sambung gagal: {e}");
            }
        }
        STATUS.store(0, Ordering::Relaxed);
        FPS_OUT.store(0, Ordering::Relaxed);
        let d = gpad_protocol::reconnect_delay(attempt);
        attempt = attempt.saturating_add(1);
        eprintln!("coba lagi dalam {}s…", d.as_secs());
        std::thread::sleep(d);
    }
}

/// Satu sesi: handshake, lalu loop I/O sampai fatal.
fn run_session(mut link: usb::UsbLink, config: &Config) {
    // --- handshake: HP mengirim duluan ---
    let (ch, payload) = match read_frame_deadline(&mut link, Duration::from_secs(10)) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("handshake gagal dibaca: {e}");
            return;
        }
    };
    if ch != CH_CONTROL || payload.len() != HANDSHAKE_LEN {
        eprintln!("handshake: channel/panjang salah (ch={ch} len={})", payload.len());
        return;
    }
    let mut hb = [0u8; HANDSHAKE_LEN];
    hb.copy_from_slice(&payload);
    let hs = match Handshake::decode(&hb) {
        Some(h) if h.version == PROTOCOL_VERSION => h,
        Some(h) => {
            eprintln!("versi protokol HP tak cocok: {}", h.version);
            return;
        }
        None => {
            eprintln!("magic handshake salah");
            return;
        }
    };
    eprintln!("Handshake OK: HP {}x{} caps=0x{:04x}", hs.width, hs.height, hs.caps);

    #[cfg(windows)]
    let (desk_w, desk_h) = desktop_size();
    #[cfg(not(windows))]
    let (desk_w, desk_h) = (1920, 1080);

    #[cfg(windows)]
    let mut pipeline = match video::VideoPipeline::new(
        desk_w as usize,
        desk_h as usize,
        config.bitrate_bps(),
    ) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return;
        }
    };
    #[cfg(windows)]
    let (vid_w, vid_h) = pipeline.enc_size();
    #[cfg(not(windows))]
    let (vid_w, vid_h) = (1280, 720);

    let reply = HandshakeReply {
        version: PROTOCOL_VERSION,
        desk_w: desk_w.min(65535) as u16,
        desk_h: desk_h.min(65535) as u16,
        vid_w: vid_w.min(65535) as u16,
        vid_h: vid_h.min(65535) as u16,
        fps: config.video.fps.min(255) as u8,
    };
    if link.write_frame(CH_CONTROL, &reply.encode()).is_err() {
        eprintln!("gagal kirim balasan handshake");
        return;
    }

    #[cfg(windows)]
    let mut injector = WinInjector::new();
    #[cfg(not(windows))]
    let mut injector = MockInjector::with_screen(desk_w as i32, desk_h as i32);
    let gamma = config.gamma();

    STATUS.store(2, Ordering::Relaxed);

    let video_interval = Duration::from_secs_f64(1.0 / config.fps().max(1).min(60) as f64);
    let mut next_video = Instant::now();
    let mut next_hb = Instant::now();
    let mut next_stats = Instant::now();
    let mut pending_hb: Option<u64> = None;
    let mut last_hb_echo = Instant::now();
    let mut last_seq: Option<u16> = None;
    let mut frames_sent = 0u64;
    let mut bytes_sent = 0u64;
    let mut stat_window = Instant::now();

    loop {
        let now = Instant::now();

        // 1. Video (Windows): pump bila jadwalnya tiba.
        #[cfg(windows)]
        if now >= next_video {
            next_video = now + video_interval;
            match pipeline.pump() {
                Ok(Some(payload)) => {
                    if link.write_frame(CH_VIDEO, &payload).is_err() {
                        eprintln!("tulis video gagal — koneksi putus");
                        return;
                    }
                    frames_sent += 1;
                    bytes_sent += payload.len() as u64;
                }
                Ok(None) => {}
                Err(e) => {
                    eprintln!("video error: {e}");
                    return;
                }
            }
        }

        // 2. Heartbeat tiap detik.
        if now >= next_hb {
            next_hb = now + HB_INTERVAL;
            let ts = monotonic_us();
            if link.write_frame(CH_HEARTBEAT, &encode_heartbeat(ts)).is_err() {
                eprintln!("tulis heartbeat gagal");
                return;
            }
            pending_hb = Some(ts);
        }

        // 3. Stats tiap detik.
        if now >= next_stats {
            next_stats = now + Duration::from_secs(1);
            let el = stat_window.elapsed().as_secs_f64().max(0.001);
            let fps = (frames_sent as f64 / el).round().clamp(0.0, 255.0) as u8;
            let kbps = (bytes_sent as f64 * 8.0 / el / 1000.0).round().clamp(0.0, 65535.0) as u16;
            FPS_OUT.store(fps, Ordering::Relaxed);
            let stats = Stats {
                fps,
                rtt_ms: RTT_MS.load(Ordering::Relaxed),
                bitrate_kbps: kbps,
            };
            if link.write_frame(CH_STATS, &stats.encode()).is_err() {
                eprintln!("tulis stats gagal");
                return;
            }
            frames_sent = 0;
            bytes_sent = 0;
            stat_window = Instant::now();
        }

        // 4. Mati bila echo tak kembali.
        if pending_hb.is_some() && now.duration_since(last_hb_echo) > HB_DEAD_AFTER {
            eprintln!("heartbeat tak di-echo >3.5s — koneksi mati");
            return;
        }

        // 5. Baca input/heartbeat (poll singkat agar loop tetap jalan).
        match link.poll_frame(POLL_TIMEOUT) {
            Ok(Some((CH_INPUT, payload))) => {
                if payload.len() == PACKET_LEN {
                    let mut b = [0u8; PACKET_LEN];
                    b.copy_from_slice(&payload);
                    let pkt = PenPacket::decode(&b);
                    on_pen_packet(&mut injector, &pkt, &mut last_seq, gamma, desk_w, desk_h);
                }
            }
            Ok(Some((CH_HEARTBEAT, payload))) => {
                if let Some(ts) = decode_heartbeat(&payload) {
                    if pending_hb == Some(ts) {
                        let rtt = monotonic_us().saturating_sub(ts);
                        RTT_MS.store((rtt / 1000).min(65535) as u16, Ordering::Relaxed);
                        last_hb_echo = Instant::now();
                        pending_hb = None;
                    } else {
                        // Heartbeat dari HP — echo kembali.
                        let _ = link.write_frame(CH_HEARTBEAT, &encode_heartbeat(ts));
                    }
                }
            }
            Ok(Some((_ch, _))) => {
                // CH_VIDEO tak diharapkan dari HP; lainnya diabaikan.
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("baca USB gagal: {e}");
                return;
            }
        }
    }
}

/// Baca satu frame dengan deadline total.
fn read_frame_deadline(
    link: &mut usb::UsbLink,
    deadline: Duration,
) -> Result<(u8, Vec<u8>), usb::UsbError> {
    let start = Instant::now();
    loop {
        match link.poll_frame(Duration::from_millis(200))? {
            Some(f) => return Ok(f),
            None => {
                if start.elapsed() > deadline {
                    return Err(usb::UsbError::Timeout("handshake"));
                }
            }
        }
    }
}

fn on_pen_packet<I: Injector>(
    injector: &mut I,
    p: &PenPacket,
    last_seq: &mut Option<u16>,
    gamma: f32,
    desk_w: i32,
    desk_h: i32,
) {
    if let Some(prev) = *last_seq {
        let gap = p.seq_gap(prev);
        if gap > 0 {
            eprintln!("peringatan: {gap} paket input hilang");
        }
    }
    *last_seq = Some(p.seq);

    // x/y sudah dinormalisasi terhadap desktop penuh oleh HP.
    let x = (p.x.clamp(0.0, 1.0) * desk_w as f32).round() as i32;
    let y = (p.y.clamp(0.0, 1.0) * desk_h as f32).round() as i32;
    let contact = match p.kind {
        KIND_DOWN | KIND_MOVE => true,
        KIND_UP | KIND_HOVER => false,
        _ => return,
    };
    injector.pen(&PenReport {
        x: x.clamp(0, desk_w - 1),
        y: y.clamp(0, desk_h - 1),
        pressure01: p.pressure.clamp(0.0, 1.0).powf(gamma),
        tilt_x: p.tilt_x as i32,
        tilt_y: p.tilt_y as i32,
        in_range: true,
        contact,
        barrel: p.flags & gpad_protocol::FLAG_BARREL_BUTTON != 0,
        eraser: p.flags & gpad_protocol::FLAG_ERASER != 0,
    });
}

#[cfg(windows)]
fn desktop_size() -> (i32, i32) {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
    unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
}

fn monotonic_us() -> u64 {
    use std::sync::OnceLock;
    static T0: OnceLock<Instant> = OnceLock::new();
    let t0 = T0.get_or_init(Instant::now);
    t0.elapsed().as_micros() as u64
}

// Re-ekspor untuk tray.
pub(crate) fn status_snapshot() -> (u8, u8, u16) {
    (
        STATUS.load(Ordering::Relaxed),
        FPS_OUT.load(Ordering::Relaxed),
        RTT_MS.load(Ordering::Relaxed),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpad_protocol::FLAG_FINGER;

    fn pkt(kind: u8, seq: u16, x: f32, y: f32) -> PenPacket {
        PenPacket {
            kind,
            flags: FLAG_FINGER,
            seq,
            t_us: 0,
            x,
            y,
            pressure: 0.65,
            tilt_x: 0,
            tilt_y: 0,
        }
    }

    #[test]
    fn pen_packet_maps_to_desktop_pixels() {
        let mut inj = MockInjector::with_screen(1920, 1080);
        let mut last = None;
        on_pen_packet(&mut inj, &pkt(KIND_DOWN, 1, 0.5, 0.5), &mut last, 1.0, 1920, 1080);
        let rs = inj.take_reports();
        assert_eq!(rs.len(), 1);
        assert_eq!((rs[0].x, rs[0].y), (960, 540));
        assert!(rs[0].contact);
        assert!((rs[0].pressure01 - 0.65).abs() < 1e-6);
    }

    #[test]
    fn pen_packet_clamps_edges() {
        let mut inj = MockInjector::with_screen(1920, 1080);
        let mut last = None;
        on_pen_packet(&mut inj, &pkt(KIND_DOWN, 1, 1.5, -0.5), &mut last, 1.0, 1920, 1080);
        let rs = inj.take_reports();
        assert_eq!((rs[0].x, rs[0].y), (1919, 0));
    }

    #[test]
    fn hover_has_no_contact() {
        let mut inj = MockInjector::with_screen(1920, 1080);
        let mut last = None;
        on_pen_packet(&mut inj, &pkt(KIND_HOVER, 1, 0.1, 0.1), &mut last, 1.0, 1920, 1080);
        let rs = inj.take_reports();
        assert!(!rs[0].contact);
        assert!(rs[0].in_range);
    }

    #[test]
    fn heartbeat_encode_decode_for_rtt() {
        let ts = monotonic_us();
        let b = encode_heartbeat(ts);
        assert_eq!(decode_heartbeat(&b), Some(ts));
    }

    #[test]
    fn reconnect_delay_increases() {
        use std::time::Duration;
        assert_eq!(gpad_protocol::reconnect_delay(0), Duration::from_secs(1));
        assert_eq!(gpad_protocol::reconnect_delay(3), Duration::from_secs(8));
    }
}
