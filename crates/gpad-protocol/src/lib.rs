//! gpad-protocol v0.5.0 — definisi paket biner Gpad "Cintiq mode".
//!
//! Transport: Android Open Accessory (AOA) over USB bulk endpoints.
//! Semua pesan dibingkai length-prefixed agar dua kanal logis (video
//! downstream, input upstream) bisa dimultipleks di atas satu pasang
//! endpoint bulk:
//!
//! ```text
//! [0..4]  u32 LE: panjang frame TOTAL termasuk 4 byte header ini
//! [4]     u8:     channel (CH_*)
//! [5..]    payload (panjang = frame_len - 5)
//! ```
//!
//! Byte order: little-endian di kedua sisi.
//!
//! v0.5.0 menghapus total jalur TCP/ADB-reverse v0.4.x (GPAD_PORT,
//! handshake "GPAD" v1, mode absolut/relatif, GMOD). PenPacket 24-byte
//! dipakai ulang untuk kanal input — koordinat x/y kini dinormalisasi
//! terhadap SELURUH desktop PC (0.0..1.0), karena HP menampilkan mirror
//! penuh desktop (letterboxed).

/// Versi protokol v0.5.0.
pub const PROTOCOL_VERSION: u16 = 2;
/// Magic "GPD5" di awal handshake.
pub const HANDSHAKE_MAGIC: u32 = 0x3544_5047;

/// Panjang maksimum satu frame (8 MiB) — bingkai video keyframe aman.
pub const MAX_FRAME_LEN: usize = 8 * 1024 * 1024;
/// Panjang header frame: 4 byte len + 1 byte channel.
pub const FRAME_HEADER_LEN: usize = 5;

// --- channel ---
/// Kontrol: handshake & balasan.
pub const CH_CONTROL: u8 = 0x01;
/// Video downstream (PC -> HP): frame H.264.
pub const CH_VIDEO: u8 = 0x02;
/// Input upstream (HP -> PC): PenPacket 24-byte.
pub const CH_INPUT: u8 = 0x03;
/// Heartbeat dua arah (tiap 1 detik) + echo untuk ukur RTT.
pub const CH_HEARTBEAT: u8 = 0x04;
/// Statistik PC -> HP (1 Hz): fps, RTT, bitrate.
pub const CH_STATS: u8 = 0x05;

// --- jenis event pena (field `kind` di PenPacket) ---
pub const KIND_DOWN: u8 = 0;
pub const KIND_MOVE: u8 = 1;
pub const KIND_UP: u8 = 2;
pub const KIND_HOVER: u8 = 3;

// --- flags (field `flags`, bitmask) ---
/// Tombol utama stylus ditekan.
pub const FLAG_BARREL_BUTTON: u8 = 0x01;
/// Ujung penghapus.
pub const FLAG_ERASER: u8 = 0x02;
/// Sumbernya jari, bukan stylus aktif.
pub const FLAG_FINGER: u8 = 0x04;

// --- capability (handshake `caps`, bitmask) ---
pub const CAP_PRESSURE: u16 = 0x0001;
pub const CAP_TILT: u16 = 0x0002;
pub const CAP_HOVER: u16 = 0x0004;
pub const CAP_BUTTON: u16 = 0x0008;
/// HP bisa decode H.264 via MediaCodec.
pub const CAP_VIDEO: u16 = 0x0010;

/// Panjang paket handshake HP -> PC.
pub const HANDSHAKE_LEN: usize = 16;
/// Panjang paket balasan PC -> HP.
pub const HANDSHAKE_REPLY_LEN: usize = 20;
/// Panjang paket pena.
pub const PACKET_LEN: usize = 24;
/// Panjang header video per frame (di dalam payload CH_VIDEO).
pub const VIDEO_HEADER_LEN: usize = 15;
/// Panjang payload heartbeat (u64 timestamp).
pub const HEARTBEAT_LEN: usize = 8;
/// Panjang payload stats.
pub const STATS_LEN: usize = 5;

/// Bit flag video: keyframe (IDR, berisi SPS/PPS).
pub const VIDEO_FLAG_KEYFRAME: u8 = 0x01;

// ---------------------------------------------------------------------------
// Framing
// ---------------------------------------------------------------------------

/// Encode satu frame: [len u32 LE][channel u8][payload].
pub fn encode_frame(channel: u8, payload: &[u8]) -> Vec<u8> {
    let total = FRAME_HEADER_LEN + payload.len();
    assert!(total <= MAX_FRAME_LEN, "frame terlalu besar");
    let mut b = Vec::with_capacity(total);
    b.extend_from_slice(&(total as u32).to_le_bytes());
    b.push(channel);
    b.extend_from_slice(payload);
    b
}

/// Hasil parse header frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    /// Panjang total frame termasuk header.
    pub len: usize,
    pub channel: u8,
}

/// Parse 5 byte header. None jika panjang tidak valid.
pub fn decode_frame_header(buf: &[u8]) -> Option<FrameHeader> {
    if buf.len() < FRAME_HEADER_LEN {
        return None;
    }
    let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if len < FRAME_HEADER_LEN || len > MAX_FRAME_LEN {
        return None;
    }
    Some(FrameHeader { len, channel: buf[4] })
}

/// Validasi channel dikenal.
pub fn channel_known(ch: u8) -> bool {
    matches!(ch, CH_CONTROL | CH_VIDEO | CH_INPUT | CH_HEARTBEAT | CH_STATS)
}

// ---------------------------------------------------------------------------
// Handshake
// ---------------------------------------------------------------------------

/// Handshake HP -> PC, 16 byte:
/// magic "GPD5" | version u16 | width u16 | height u16 | caps u16 | reserved u16.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handshake {
    pub version: u16,
    /// Lebar layar HP, piksel.
    pub width: u16,
    /// Tinggi layar HP, piksel.
    pub height: u16,
    pub caps: u16,
}

impl Handshake {
    pub fn encode(&self) -> [u8; HANDSHAKE_LEN] {
        let mut b = [0u8; HANDSHAKE_LEN];
        b[0..4].copy_from_slice(&HANDSHAKE_MAGIC.to_le_bytes());
        b[4..6].copy_from_slice(&self.version.to_le_bytes());
        b[6..8].copy_from_slice(&self.width.to_le_bytes());
        b[8..10].copy_from_slice(&self.height.to_le_bytes());
        b[10..12].copy_from_slice(&self.caps.to_le_bytes());
        b
    }

    pub fn decode(buf: &[u8; HANDSHAKE_LEN]) -> Option<Self> {
        if u32::from_le_bytes(buf[0..4].try_into().ok()?) != HANDSHAKE_MAGIC {
            return None;
        }
        Some(Self {
            version: u16::from_le_bytes(buf[4..6].try_into().ok()?),
            width: u16::from_le_bytes(buf[6..8].try_into().ok()?),
            height: u16::from_le_bytes(buf[8..10].try_into().ok()?),
            caps: u16::from_le_bytes(buf[10..12].try_into().ok()?),
        })
    }
}

/// Balasan PC -> HP, 20 byte:
/// magic "GPD5" | version u16 | desk_w u16 | desk_h u16 |
/// vid_w u16 | vid_h u16 | fps u8 | reserved[3].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HandshakeReply {
    pub version: u16,
    pub desk_w: u16,
    pub desk_h: u16,
    /// Dimensi stream video ter-encode (setelah scaling).
    pub vid_w: u16,
    pub vid_h: u16,
    pub fps: u8,
}

impl HandshakeReply {
    pub fn encode(&self) -> [u8; HANDSHAKE_REPLY_LEN] {
        let mut b = [0u8; HANDSHAKE_REPLY_LEN];
        b[0..4].copy_from_slice(&HANDSHAKE_MAGIC.to_le_bytes());
        b[4..6].copy_from_slice(&self.version.to_le_bytes());
        b[6..8].copy_from_slice(&self.desk_w.to_le_bytes());
        b[8..10].copy_from_slice(&self.desk_h.to_le_bytes());
        b[10..12].copy_from_slice(&self.vid_w.to_le_bytes());
        b[12..14].copy_from_slice(&self.vid_h.to_le_bytes());
        b[14] = self.fps;
        b
    }

    pub fn decode(buf: &[u8; HANDSHAKE_REPLY_LEN]) -> Option<Self> {
        if u32::from_le_bytes(buf[0..4].try_into().ok()?) != HANDSHAKE_MAGIC {
            return None;
        }
        Some(Self {
            version: u16::from_le_bytes(buf[4..6].try_into().ok()?),
            desk_w: u16::from_le_bytes(buf[6..8].try_into().ok()?),
            desk_h: u16::from_le_bytes(buf[8..10].try_into().ok()?),
            vid_w: u16::from_le_bytes(buf[10..12].try_into().ok()?),
            vid_h: u16::from_le_bytes(buf[12..14].try_into().ok()?),
            fps: buf[14],
        })
    }
}

// ---------------------------------------------------------------------------
// Video
// ---------------------------------------------------------------------------

/// Header frame video (15 byte) di awal payload CH_VIDEO:
/// seq u32 | capture_ts_us u64 | flags u8 | reserved[2], lalu NAL Annex-B.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoHeader {
    pub seq: u32,
    /// Waktu capture di PC, mikrodetik (monotonic). Dipakai estimasi latensi.
    pub capture_ts_us: u64,
    pub flags: u8,
}

impl VideoHeader {
    pub fn encode(&self) -> [u8; VIDEO_HEADER_LEN] {
        let mut b = [0u8; VIDEO_HEADER_LEN];
        b[0..4].copy_from_slice(&self.seq.to_le_bytes());
        b[4..12].copy_from_slice(&self.capture_ts_us.to_le_bytes());
        b[12] = self.flags;
        b
    }

    pub fn decode(buf: &[u8]) -> Option<Self> {
        if buf.len() < VIDEO_HEADER_LEN {
            return None;
        }
        Some(Self {
            seq: u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]),
            capture_ts_us: u64::from_le_bytes([
                buf[4], buf[5], buf[6], buf[7], buf[8], buf[9], buf[10], buf[11],
            ]),
            flags: buf[12],
        })
    }

    pub fn is_keyframe(&self) -> bool {
        self.flags & VIDEO_FLAG_KEYFRAME != 0
    }
}

// ---------------------------------------------------------------------------
// Heartbeat & stats
// ---------------------------------------------------------------------------

/// Encode heartbeat: timestamp pengirim (mikrodetik, monotonic).
/// Penerima meng-echo payload yang sama kembali di CH_HEARTBEAT.
pub fn encode_heartbeat(ts_us: u64) -> [u8; HEARTBEAT_LEN] {
    ts_us.to_le_bytes()
}

pub fn decode_heartbeat(buf: &[u8]) -> Option<u64> {
    if buf.len() < HEARTBEAT_LEN {
        return None;
    }
    Some(u64::from_le_bytes([
        buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
    ]))
}

/// Stats PC -> HP (5 byte): fps u8 | rtt_ms u16 | bitrate_kbps u16.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stats {
    pub fps: u8,
    pub rtt_ms: u16,
    pub bitrate_kbps: u16,
}

impl Stats {
    pub fn encode(&self) -> [u8; STATS_LEN] {
        let mut b = [0u8; STATS_LEN];
        b[0] = self.fps;
        b[1..3].copy_from_slice(&self.rtt_ms.to_le_bytes());
        b[3..5].copy_from_slice(&self.bitrate_kbps.to_le_bytes());
        b
    }

    pub fn decode(buf: &[u8]) -> Option<Self> {
        if buf.len() < STATS_LEN {
            return None;
        }
        Some(Self {
            fps: buf[0],
            rtt_ms: u16::from_le_bytes([buf[1], buf[2]]),
            bitrate_kbps: u16::from_le_bytes([buf[3], buf[4]]),
        })
    }
}

// ---------------------------------------------------------------------------
// Pemetaan koordinat: sentuh di rect video (letterbox) -> desktop 0.0..1.0
// ---------------------------------------------------------------------------

/// Persegi tampilan video di layar HP (piksel).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Petakan titik sentuh (piksel layar HP) ke koordinat desktop 0.0..1.0.
/// None jika sentuhan di luar rect video (area letterbox) — diabaikan.
pub fn map_touch_to_desktop(tx: f32, ty: f32, rect: VideoRect) -> Option<(f32, f32)> {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return None;
    }
    let nx = (tx - rect.x) / rect.w;
    let ny = (ty - rect.y) / rect.h;
    if !(0.0..=1.0).contains(&nx) || !(0.0..=1.0).contains(&ny) {
        return None;
    }
    Some((nx, ny))
}

/// Hitung rect letterbox: video vw×vh pas di layar sw×sh (aspect dijaga).
pub fn letterbox_rect(sw: f32, sh: f32, vw: f32, vh: f32) -> VideoRect {
    if sw <= 0.0 || sh <= 0.0 || vw <= 0.0 || vh <= 0.0 {
        return VideoRect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
    }
    let scale = (sw / vw).min(sh / vh);
    let w = vw * scale;
    let h = vh * scale;
    VideoRect {
        x: (sw - w) / 2.0,
        y: (sh - h) / 2.0,
        w,
        h,
    }
}

// ---------------------------------------------------------------------------
// Reconnect backoff
// ---------------------------------------------------------------------------

/// Backoff eksponensial untuk reconnect: 1s, 2s, 4s, ... maks 30s.
pub fn reconnect_delay(attempt: u32) -> std::time::Duration {
    let secs = 1u64 << attempt.min(5); // 1,2,4,8,16,32 -> clamp 30
    std::time::Duration::from_secs(secs.min(30))
}

// ---------------------------------------------------------------------------
// PenPacket (dipakai ulang dari v0.4.x, kini via CH_INPUT)
// ---------------------------------------------------------------------------

/// Satu event pena: 24 byte, repr(C) agar layout deterministik.
/// x/y: 0.0..1.0 relatif terhadap SELURUH desktop PC (bukan area HP).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenPacket {
    pub kind: u8,      // KIND_*
    pub flags: u8,     // FLAG_*
    pub seq: u16,      // nomor urut, deteksi paket hilang
    pub t_us: u32,     // waktu event, mikrodetik (monotonic perangkat)
    pub x: f32,        // 0.0..1.0 relatif ke desktop PC
    pub y: f32,        // 0.0..1.0
    pub pressure: f32, // 0.0..1.0 (emulasi, lihat README)
    pub tilt_x: i16,   // derajat, -90..90
    pub tilt_y: i16,   // derajat, -90..90
}

const _: () = assert!(std::mem::size_of::<PenPacket>() == PACKET_LEN);

impl PenPacket {
    pub fn encode(&self) -> [u8; PACKET_LEN] {
        let mut b = [0u8; PACKET_LEN];
        b[0] = self.kind;
        b[1] = self.flags;
        b[2..4].copy_from_slice(&self.seq.to_le_bytes());
        b[4..8].copy_from_slice(&self.t_us.to_le_bytes());
        b[8..12].copy_from_slice(&self.x.to_le_bytes());
        b[12..16].copy_from_slice(&self.y.to_le_bytes());
        b[16..20].copy_from_slice(&self.pressure.to_le_bytes());
        b[20..22].copy_from_slice(&self.tilt_x.to_le_bytes());
        b[22..24].copy_from_slice(&self.tilt_y.to_le_bytes());
        b
    }

    pub fn decode(buf: &[u8; PACKET_LEN]) -> Self {
        Self {
            kind: buf[0],
            flags: buf[1],
            seq: u16::from_le_bytes([buf[2], buf[3]]),
            t_us: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
            x: f32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
            y: f32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]),
            pressure: f32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]),
            tilt_x: i16::from_le_bytes([buf[20], buf[21]]),
            tilt_y: i16::from_le_bytes([buf[22], buf[23]]),
        }
    }

    /// Selisih nomor urut dengan paket sebelumnya (wrapping u16).
    /// >1 berarti ada paket yang hilang di tengah.
    pub fn seq_gap(&self, prev_seq: u16) -> u16 {
        self.seq.wrapping_sub(prev_seq).wrapping_sub(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip() {
        let payload = vec![1u8, 2, 3, 4, 5];
        let f = encode_frame(CH_VIDEO, &payload);
        assert_eq!(f.len(), FRAME_HEADER_LEN + 5);
        let h = decode_frame_header(&f).unwrap();
        assert_eq!(h.len, f.len());
        assert_eq!(h.channel, CH_VIDEO);
        assert_eq!(&f[FRAME_HEADER_LEN..], &payload[..]);
    }

    #[test]
    fn frame_rejects_bad_len() {
        assert!(decode_frame_header(&[0u8; 4]).is_none()); // terlalu pendek
        let mut bad = [0u8; 5];
        bad[0..4].copy_from_slice(&4u32.to_le_bytes()); // len < header
        assert!(decode_frame_header(&bad).is_none());
        let mut big = [0u8; 5];
        big[0..4].copy_from_slice(&((MAX_FRAME_LEN + 1) as u32).to_le_bytes());
        assert!(decode_frame_header(&big).is_none());
        // len = 0
        assert!(decode_frame_header(&[0u8; 5]).is_none());
    }

    #[test]
    fn channels_known() {
        for ch in [CH_CONTROL, CH_VIDEO, CH_INPUT, CH_HEARTBEAT, CH_STATS] {
            assert!(channel_known(ch));
        }
        assert!(!channel_known(0x00));
        assert!(!channel_known(0xFF));
    }

    #[test]
    fn handshake_roundtrip_and_bad_magic() {
        let h = Handshake {
            version: PROTOCOL_VERSION,
            width: 1080,
            height: 2400,
            caps: CAP_PRESSURE | CAP_VIDEO,
        };
        let b = h.encode();
        assert_eq!(Handshake::decode(&b), Some(h));
        let mut bad = b;
        bad[0] ^= 0xFF;
        assert_eq!(Handshake::decode(&bad), None);
    }

    #[test]
    fn handshake_reply_roundtrip() {
        let r = HandshakeReply {
            version: PROTOCOL_VERSION,
            desk_w: 1920,
            desk_h: 1080,
            vid_w: 1280,
            vid_h: 720,
            fps: 30,
        };
        let b = r.encode();
        assert_eq!(b.len(), HANDSHAKE_REPLY_LEN);
        assert_eq!(HandshakeReply::decode(&b), Some(r));
    }

    #[test]
    fn video_header_keyframe_flag() {
        let h = VideoHeader { seq: 7, capture_ts_us: 123456, flags: VIDEO_FLAG_KEYFRAME };
        let b = h.encode();
        let d = VideoHeader::decode(&b).unwrap();
        assert_eq!(d, h);
        assert!(d.is_keyframe());
        let h2 = VideoHeader { seq: 8, capture_ts_us: 0, flags: 0 };
        assert!(!VideoHeader::decode(&h2.encode()).unwrap().is_keyframe());
        assert!(VideoHeader::decode(&[0u8; 4]).is_none());
    }

    #[test]
    fn heartbeat_echo_roundtrip() {
        let ts = 9_999_999u64;
        let b = encode_heartbeat(ts);
        assert_eq!(decode_heartbeat(&b), Some(ts));
        assert_eq!(decode_heartbeat(&[0u8; 4]), None);
    }

    #[test]
    fn stats_roundtrip() {
        let s = Stats { fps: 30, rtt_ms: 12, bitrate_kbps: 2500 };
        assert_eq!(Stats::decode(&s.encode()), Some(s));
    }

    #[test]
    fn letterbox_16x9_on_tall_phone() {
        // Desktop 1920x1080 di layar HP 1080x2400 -> video 1080x607.5, centered.
        let r = letterbox_rect(1080.0, 2400.0, 1920.0, 1080.0);
        assert!((r.w - 1080.0).abs() < 0.01);
        assert!((r.h - 607.5).abs() < 0.01);
        assert!((r.x - 0.0).abs() < 0.01);
        assert!((r.y - (2400.0 - 607.5) / 2.0).abs() < 0.01);
    }

    #[test]
    fn touch_mapping_inside_and_outside() {
        let r = letterbox_rect(1080.0, 2400.0, 1920.0, 1080.0);
        // Tengah video -> (0.5, 0.5) desktop.
        let (x, y) = map_touch_to_desktop(r.x + r.w / 2.0, r.y + r.h / 2.0, r).unwrap();
        assert!((x - 0.5).abs() < 1e-6 && (y - 0.5).abs() < 1e-6);
        // Pojok kiri atas video -> (0,0).
        let (x, y) = map_touch_to_desktop(r.x, r.y, r).unwrap();
        assert!((x - 0.0).abs() < 1e-6 && (y - 0.0).abs() < 1e-6);
        // Di area letterbox (atas) -> None.
        assert!(map_touch_to_desktop(540.0, 100.0, r).is_none());
        // Di luar kanan -> None.
        assert!(map_touch_to_desktop(r.x + r.w + 10.0, r.y + 10.0, r).is_none());
    }

    #[test]
    fn reconnect_backoff_sequence() {
        use std::time::Duration;
        assert_eq!(reconnect_delay(0), Duration::from_secs(1));
        assert_eq!(reconnect_delay(1), Duration::from_secs(2));
        assert_eq!(reconnect_delay(2), Duration::from_secs(4));
        assert_eq!(reconnect_delay(5), Duration::from_secs(30)); // clamp
        assert_eq!(reconnect_delay(99), Duration::from_secs(30));
    }

    #[test]
    fn pen_packet_roundtrip() {
        let p = PenPacket {
            kind: KIND_MOVE,
            flags: FLAG_FINGER,
            seq: 4242,
            t_us: 1_234_567,
            x: 0.25,
            y: 0.75,
            pressure: 0.65,
            tilt_x: 0,
            tilt_y: 0,
        };
        assert_eq!(PenPacket::decode(&p.encode()), p);
        assert_eq!(p.seq_gap(4241), 0);
    }
}
