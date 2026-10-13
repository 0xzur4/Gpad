//! gpad-daemon — Fase 3 (kenyamanan pakai).
//!
//! - Mode absolut (area: layar / monitor:N / custom, opsi keep_aspect)
//!   dan mode relatif (sensitivitas, tanpa akselerasi).
//! - Ganti mode: tombol di HP, tombol barrel stylus, hotkey global,
//!   menu tray. Daemon -> HP: paket balasan mode ("GMOD").
//! - Konfigurasi TOML (gpad.toml) + profil per aplikasi (Windows).
//! - Pen injection Windows Ink seperti Fase 2.

mod config;
#[cfg(windows)]
mod foreground;
#[cfg(windows)]
mod hotkey;
mod inject;
mod mapping;
#[cfg(windows)]
mod tray;

use config::Config;
use gpad_protocol::{
    encode_mode_reply, Handshake, PenPacket, FLAG_BARREL_BUTTON, FLAG_ERASER, GPAD_PORT,
    HANDSHAKE_LEN, KIND_DOWN, KIND_HOVER, KIND_MODE_SWITCH, KIND_MOVE, KIND_UP, MODE_ABSOLUTE,
    MODE_RELATIVE, PACKET_LEN, PROTOCOL_VERSION,
};
use inject::{Injector, PenReport};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::time::Instant;

#[cfg(windows)]
use inject::win::WinInjector;
#[cfg(not(windows))]
use inject::mock::MockInjector;

/// Mode aktif bersama: ditulis oleh hotkey/tray/HP/barrel/profil, dibaca per paket.
static MODE: AtomicU8 = AtomicU8::new(MODE_ABSOLUTE);
/// Override sensitivitas profil (bits f32; 0 = pakai config).
static SENS_OVERRIDE: AtomicU32 = AtomicU32::new(0);
/// Override gamma profil (bits f32; 0 = pakai config).
static GAMMA_OVERRIDE: AtomicU32 = AtomicU32::new(0);

fn mode_name(m: u8) -> &'static str {
    if m == MODE_ABSOLUTE {
        "absolut"
    } else {
        "relatif"
    }
}

fn main() {
    println!(
        "Gpad daemon v{} — Fase 3 (mode absolut/relatif, tray, profil)",
        env!("CARGO_PKG_VERSION")
    );
    let config = Config::load();
    MODE.store(
        if config.general.mode == "relative" {
            MODE_RELATIVE
        } else {
            MODE_ABSOLUTE
        },
        Ordering::Relaxed,
    );
    eprintln!("mode awal: {}", mode_name(MODE.load(Ordering::Relaxed)));

    try_adb_reverse();

    #[cfg(windows)]
    {
        let hk = config.switching.hotkey.clone();
        if !hk.trim().is_empty() {
            std::thread::Builder::new()
                .name("gpad-hotkey".into())
                .spawn(move || hotkey::run(&hk))
                .expect("spawn hotkey");
        }
        std::thread::Builder::new()
            .name("gpad-tray".into())
            .spawn(tray::run)
            .expect("spawn tray");
        let profiles: Vec<config::Profile> = config.profile.values().cloned().collect();
        if !profiles.is_empty() {
            std::thread::Builder::new()
                .name("gpad-profiles".into())
                .spawn(move || foreground::run(profiles))
                .expect("spawn profiles");
        }
    }

    let addr = format!("127.0.0.1:{GPAD_PORT}");
    let listener = TcpListener::bind(&addr).expect("gagal bind localhost");
    println!("Mendengarkan di {addr} (hanya localhost).");

    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                println!("Perangkat tersambung.");
                handle_client(s, &config);
                println!("Perangkat terputus — menunggu koneksi berikutnya.");
            }
            Err(e) => eprintln!("accept gagal: {e}"),
        }
    }
}

/// Diagnostik ADB: reverse + cek status perangkat (kabel/otorisasi).
fn try_adb_reverse() {
    let arg = format!("tcp:{GPAD_PORT}");
    let rev = std::process::Command::new("adb")
        .args(["reverse", &arg, &arg])
        .output();
    match rev {
        Ok(o) if o.status.success() => {
            println!("adb reverse aktif.");
            // Cek daftar perangkat untuk panduan koneksi.
            if let Ok(dev) = std::process::Command::new("adb").arg("devices").output() {
                let out = String::from_utf8_lossy(&dev.stdout);
                let lines: Vec<&str> = out.lines().skip(1).filter(|l| !l.trim().is_empty()).collect();
                if lines.is_empty() {
                    println!("panduan: tidak ada perangkat — cek KABEL DATA (bukan charge-only) dan USB debugging.");
                } else if lines.iter().any(|l| l.contains("unauthorized")) {
                    println!("panduan: perangkat belum dipercaya — setujui dialog RSA di HP.");
                } else {
                    println!("panduan: {} perangkat terdeteksi.", lines.len());
                }
            }
        }
        _ => println!(
            "adb tidak jalan — instal platform-tools lalu: adb reverse tcp:{GPAD_PORT} tcp:{GPAD_PORT}"
        ),
    }
}

fn handle_client(stream: TcpStream, config: &Config) {
    if let Err(e) = stream.set_nodelay(true) {
        eprintln!("set_nodelay gagal: {e}");
        return;
    }
    // Fase 4: pembacaan BLOKIR tanpa timeout — tidak ada bangun palsu tiap
    // 30 detik; putus koneksi terdeteksi dari penutupan socket oleh HP.

    #[cfg(windows)]
    let injector = WinInjector::new();
    #[cfg(not(windows))]
    let injector = MockInjector::with_screen(1920, 1080);

    // --- handshake ---
    let peer = stream
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "?".into());
    let mut hb = [0u8; HANDSHAKE_LEN];
    let mut stream = stream;
    if let Err(e) = stream.read_exact(&mut hb) {
        eprintln!("gagal baca handshake dari {peer}: {e}");
        return;
    }
    let phone_aspect = match Handshake::decode(&hb) {
        Some(h) if h.version == PROTOCOL_VERSION => {
            println!(
                "Handshake OK: protokol v{}, layar HP {}x{}, caps=0x{:04x}",
                h.version, h.width, h.height, h.caps
            );
            if h.height > 0 {
                h.width as f32 / h.height as f32
            } else {
                0.0
            }
        }
        Some(h) => {
            eprintln!("versi protokol tak cocok: {}", h.version);
            return;
        }
        None => {
            eprintln!("magic handshake salah — bukan klien Gpad?");
            return;
        }
    };

    let reply = stream.try_clone().ok();
    let mut app = App::new(injector, config.clone(), phone_aspect, reply);
    let mut stats = ProcStats::new();

    let mut pb = [0u8; PACKET_LEN];
    loop {
        if stream.read_exact(&mut pb).is_err() {
            break;
        }
        // Fase 4: ukur waktu proses daemon per paket (decode + mapping + inject).
        let t0 = Instant::now();
        let pkt = PenPacket::decode(&pb);
        app.on_packet(&pkt);
        stats.record(t0.elapsed());
    }
}

/// Fase 4: statistik latensi pemrosesan daemon per paket.
/// Dilaporkan tiap 2000 paket: rata-rata, p95, maks (mikrodetik).
/// Ini komponen "jeda proses daemon" — di Windows mencakup InjectSyntheticPointerInput.
struct ProcStats {
    durs: Vec<std::time::Duration>,
}

impl ProcStats {
    fn new() -> Self {
        Self { durs: Vec::with_capacity(2000) }
    }

    fn record(&mut self, d: std::time::Duration) {
        self.durs.push(d);
        if self.durs.len() >= 2000 {
            self.durs.sort_unstable();
            let n = self.durs.len();
            let sum: std::time::Duration = self.durs.iter().sum();
            let avg = sum / n as u32;
            let p95 = self.durs[((n as f64 * 0.95) as usize).min(n - 1)];
            let max = self.durs[n - 1];
            eprintln!(
                "latensi proses daemon (n={n}): avg={}us p95={}us max={}us",
                avg.as_micros(),
                p95.as_micros(),
                max.as_micros()
            );
            self.durs.clear();
        }
    }
}

/// Keadaan per koneksi.
struct App<I: Injector> {
    injector: I,
    config: Config,
    phone_aspect: f32,
    area: mapping::Area,
    relative: Option<mapping::RelativeState>,
    last_seq: Option<u16>,
    down: bool,
    last_mode: u8,
    last_replied: Option<u8>,
    reply: Option<TcpStream>,
}

impl<I: Injector> App<I> {
    fn new(injector: I, config: Config, phone_aspect: f32, reply: Option<TcpStream>) -> Self {
        let (sw, sh) = injector.screen_size();
        let area = mapping::resolve_area(
            &config.absolute.area,
            sw,
            sh,
            (
                config.absolute.x,
                config.absolute.y,
                config.absolute.w,
                config.absolute.h,
            ),
        );
        println!(
            "area absolut: {}x{}+{}+{} (keep_aspect={})",
            area.w, area.h, area.x, area.y, config.absolute.keep_aspect
        );
        let mode = MODE.load(Ordering::Relaxed);
        Self {
            injector,
            config,
            phone_aspect,
            area,
            relative: None,
            last_seq: None,
            down: false,
            last_mode: mode,
            last_replied: None,
            reply,
        }
    }

    fn sensitivity(&self) -> f32 {
        let bits = SENS_OVERRIDE.load(Ordering::Relaxed);
        if bits != 0 {
            f32::from_bits(bits)
        } else {
            self.config.relative.sensitivity
        }
        .clamp(0.1, 5.0)
    }

    fn gamma(&self) -> f32 {
        let bits = GAMMA_OVERRIDE.load(Ordering::Relaxed);
        if bits != 0 {
            f32::from_bits(bits).clamp(0.25, 4.0)
        } else {
            self.config.gamma()
        }
    }

    /// Kirim balasan mode ke HP jika berubah.
    fn maybe_send_mode(&mut self) {
        let m = MODE.load(Ordering::Relaxed);
        if self.last_replied == Some(m) {
            return;
        }
        if let Some(w) = self.reply.as_mut() {
            let _ = w.write_all(&encode_mode_reply(m));
        }
        self.last_replied = Some(m);
    }

    fn on_packet(&mut self, p: &PenPacket) {
        if let Some(prev) = self.last_seq {
            let gap = p.seq_gap(prev);
            if gap > 0 {
                eprintln!("peringatan: {gap} paket hilang (seq {prev} -> {})", p.seq);
            }
        }
        self.last_seq = Some(p.seq);

        // --- ganti mode eksplisit dari HP ---
        if p.kind == KIND_MODE_SWITCH {
            let m = if p.flags & 1 == 1 { MODE_RELATIVE } else { MODE_ABSOLUTE };
            MODE.store(m, Ordering::Relaxed);
            eprintln!("mode -> {} (via HP)", mode_name(m));
            self.maybe_send_mode();
            return;
        }

        // --- baca mode terkini (bisa diubah hotkey/tray/barrel/profil) ---
        let mode = MODE.load(Ordering::Relaxed);
        if mode != self.last_mode {
            eprintln!("mode -> {}", mode_name(mode));
            self.last_mode = mode;
            if mode == MODE_RELATIVE {
                let (cx, cy) = self.injector.cursor_pos();
                self.relative = Some(mapping::RelativeState::new(cx, cy, self.area));
            } else {
                self.relative = None;
            }
        }
        self.maybe_send_mode();

        // --- hitung posisi piksel sesuai mode ---
        let (x, y) = if mode == MODE_RELATIVE {
            if self.relative.is_none() {
                let (cx, cy) = self.injector.cursor_pos();
                self.relative = Some(mapping::RelativeState::new(cx, cy, self.area));
            }
            let sens = self.sensitivity();
            let rel = self.relative.as_mut().expect("baru dibuat");
            let is_down = p.kind == KIND_DOWN;
            let is_up = p.kind == KIND_UP;
            if is_down {
                self.down = true;
            }
            let pos = rel.event(is_down, is_up, p.x, p.y, sens);
            if is_up {
                self.down = false;
            }
            pos
        } else {
            mapping::absolute(
                p.x,
                p.y,
                self.area,
                self.config.absolute.keep_aspect,
                self.phone_aspect,
            )
        };

        let contact = match p.kind {
            KIND_DOWN => {
                self.down = true;
                // Tombol barrel saat DOWN = ganti mode (Fase 3).
                if self.config.switching.barrel_toggles_mode && p.flags & FLAG_BARREL_BUTTON != 0
                {
                    let cur = MODE.load(Ordering::Relaxed);
                    let next = if cur == MODE_ABSOLUTE { MODE_RELATIVE } else { MODE_ABSOLUTE };
                    MODE.store(next, Ordering::Relaxed);
                    eprintln!("mode -> {} (via barrel)", mode_name(next));
                }
                true
            }
            KIND_MOVE => self.down,
            KIND_UP | KIND_HOVER => {
                self.down = false;
                false
            }
            k => {
                eprintln!("kind tak dikenal: {k}");
                return;
            }
        };

        self.injector.pen(&PenReport {
            x,
            y,
            pressure01: (p.pressure.clamp(0.0, 1.0).powf(self.gamma())),
            tilt_x: p.tilt_x as i32,
            tilt_y: p.tilt_y as i32,
            in_range: true,
            contact,
            barrel: p.flags & FLAG_BARREL_BUTTON != 0,
            eraser: p.flags & FLAG_ERASER != 0,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpad_protocol::{decode_mode_reply, CAP_HOVER, CAP_PRESSURE, MODE_REPLY_LEN};

    fn pkt(kind: u8, seq: u16, x: f32, y: f32) -> PenPacket {
        PenPacket {
            kind,
            flags: 0,
            seq,
            t_us: 0,
            x,
            y,
            pressure: 0.5,
            tilt_x: 0,
            tilt_y: 0,
        }
    }

    fn app() -> App<MockInjector> {
        MODE.store(MODE_ABSOLUTE, Ordering::Relaxed);
        // Aspek HP 16:9 sama dengan area -> keep_aspect tidak mengubah apa-apa.
        App::new(MockInjector::with_screen(1920, 1080), Config::default(), 16.0 / 9.0, None)
    }

    #[test]
    fn packet_drives_pen_absolute() {
        let mut app = app();
        let reports = app.injector.reports.clone();
        app.on_packet(&pkt(KIND_DOWN, 1, 0.0, 0.0));
        app.on_packet(&pkt(KIND_MOVE, 2, 0.5, 0.5));
        app.on_packet(&pkt(KIND_HOVER, 3, 0.75, 0.75));
        app.on_packet(&pkt(KIND_UP, 4, 1.0, 1.0));
        let rs = reports.lock().unwrap();
        assert_eq!(rs.len(), 4);
        assert_eq!((rs[0].x, rs[0].y), (0, 0));
        assert!(rs[0].contact && rs[0].in_range);
        assert_eq!((rs[1].x, rs[1].y), (960, 540));
        assert!(rs[1].contact);
        assert!(!rs[2].contact && rs[2].in_range);
        assert!(!rs[3].contact);
        assert!((rs[1].pressure01 - 0.5).abs() < 1e-6);
    }

    #[test]
    fn mode_switch_packet_changes_mode() {
        let mut app = app();
        assert_eq!(MODE.load(Ordering::Relaxed), MODE_ABSOLUTE);
        let mut p = pkt(KIND_MODE_SWITCH, 1, 0.0, 0.0);
        p.flags = 1; // -> relatif
        app.on_packet(&p);
        assert_eq!(MODE.load(Ordering::Relaxed), MODE_RELATIVE);
        // paket gerak berikutnya memakai mode relatif (tidak lompat)
        let reports = app.injector.reports.clone();
        app.on_packet(&pkt(KIND_DOWN, 2, 0.5, 0.5));
        app.on_packet(&pkt(KIND_MOVE, 3, 0.6, 0.5));
        let rs = reports.lock().unwrap();
        // mulai dari tengah layar (960,540), geser 0.1*1920 = +192
        assert_eq!((rs[0].x, rs[0].y), (960, 540));
        assert_eq!((rs[1].x, rs[1].y), (1152, 540));
        MODE.store(MODE_ABSOLUTE, Ordering::Relaxed); // bersih-bersih
    }

    #[test]
    fn decode_mode_reply_ok() {
        assert_eq!(decode_mode_reply(&encode_mode_reply(MODE_RELATIVE)), Some(MODE_RELATIVE));
        assert_eq!(decode_mode_reply(&[0u8; MODE_REPLY_LEN]), None);
    }

    #[test]
    fn handshake_caps_survive() {
        let h = Handshake {
            version: PROTOCOL_VERSION,
            caps: CAP_PRESSURE | CAP_HOVER,
            width: 1080,
            height: 2400,
        };
        let decoded = Handshake::decode(&h.encode()).unwrap();
        assert_eq!(decoded.caps, CAP_PRESSURE | CAP_HOVER);
    }
}
