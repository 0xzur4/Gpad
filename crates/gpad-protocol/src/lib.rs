//! gpad-protocol — definisi paket biner Gpad.
//!
//! Semua paket berukuran TETAP (tanpa JSON) agar jalur panas bebas alokasi
//! dan parsing O(1):
//!   - Handshake: 16 byte, dikirim aplikasi Android tepat setelah connect.
//!   - PenPacket: 24 byte per event stylus/sentuh.
//!
//! Byte order: little-endian di kedua sisi (to_le_bytes / from_le_bytes).

/// Port TCP lokal yang dipakai ADB reverse: `adb reverse tcp:51337 tcp:51337`.
pub const GPAD_PORT: u16 = 51337;
/// Magic "GPAD" di awal handshake.
pub const HANDSHAKE_MAGIC: u32 = 0x4441_5047;
/// Versi protokol saat ini.
pub const PROTOCOL_VERSION: u16 = 1;

/// Panjang paket handshake dalam byte.
pub const HANDSHAKE_LEN: usize = 16;
/// Panjang paket pena dalam byte.
pub const PACKET_LEN: usize = 24;

// --- jenis event (field `kind`) ---
pub const KIND_DOWN: u8 = 0;
pub const KIND_MOVE: u8 = 1;
pub const KIND_UP: u8 = 2;
pub const KIND_HOVER: u8 = 3;
/// HP -> PC: minta ganti mode. flags bit0: 0 = absolut, 1 = relatif (set eksplisit).
pub const KIND_MODE_SWITCH: u8 = 4;

// --- mode (dipakai di reply daemon -> HP) ---
pub const MODE_ABSOLUTE: u8 = 0;
pub const MODE_RELATIVE: u8 = 1;
/// Magic balasan mode daemon -> HP: "GMOD" + mode u8 + 3 byte reserved.
pub const MODE_REPLY_MAGIC: u32 = 0x444F_4D47;
pub const MODE_REPLY_LEN: usize = 8;

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

/// Handshake 16 byte: magic | version | caps | width | height | reserved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handshake {
    pub version: u16,
    /// Kemampuan perangkat (CAP_*).
    pub caps: u16,
    /// Lebar area gambar di perangkat, piksel.
    pub width: u16,
    /// Tinggi area gambar di perangkat, piksel.
    pub height: u16,
}

impl Handshake {
    pub fn encode(&self) -> [u8; HANDSHAKE_LEN] {
        let mut b = [0u8; HANDSHAKE_LEN];
        b[0..4].copy_from_slice(&HANDSHAKE_MAGIC.to_le_bytes());
        b[4..6].copy_from_slice(&self.version.to_le_bytes());
        b[6..8].copy_from_slice(&self.caps.to_le_bytes());
        b[8..10].copy_from_slice(&self.width.to_le_bytes());
        b[10..12].copy_from_slice(&self.height.to_le_bytes());
        // b[12..16] reserved = 0
        b
    }

    pub fn decode(buf: &[u8; HANDSHAKE_LEN]) -> Option<Self> {
        if u32::from_le_bytes(buf[0..4].try_into().ok()?) != HANDSHAKE_MAGIC {
            return None;
        }
        Some(Self {
            version: u16::from_le_bytes(buf[4..6].try_into().ok()?),
            caps: u16::from_le_bytes(buf[6..8].try_into().ok()?),
            width: u16::from_le_bytes(buf[8..10].try_into().ok()?),
            height: u16::from_le_bytes(buf[10..12].try_into().ok()?),
        })
    }
}

/// Balasan mode daemon -> HP: 8 byte "GMOD" | mode u8 | 3 reserved.
pub fn encode_mode_reply(mode: u8) -> [u8; MODE_REPLY_LEN] {
    let mut b = [0u8; MODE_REPLY_LEN];
    b[0..4].copy_from_slice(&MODE_REPLY_MAGIC.to_le_bytes());
    b[4] = mode;
    b
}

/// Decode balasan mode. None jika bukan paket mode yang valid.
pub fn decode_mode_reply(buf: &[u8]) -> Option<u8> {
    if buf.len() < MODE_REPLY_LEN {
        return None;
    }
    if u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) != MODE_REPLY_MAGIC {
        return None;
    }
    match buf[4] {
        MODE_ABSOLUTE | MODE_RELATIVE => Some(buf[4]),
        _ => None,
    }
}
/// Satu event pena: 24 byte, repr(C) agar layout deterministik.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenPacket {
    pub kind: u8,     // KIND_*
    pub flags: u8,    // FLAG_*
    pub seq: u16,     // nomor urut, deteksi paket hilang
    pub t_us: u32,    // waktu event, mikrodetik (monotonic perangkat)
    pub x: f32,       // 0.0..1.0 relatif ke area gambar
    pub y: f32,       // 0.0..1.0
    pub pressure: f32, // 0.0..1.0
    pub tilt_x: i16,  // derajat, -90..90
    pub tilt_y: i16,  // derajat, -90..90
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
    fn packet_roundtrip() {
        let p = PenPacket {
            kind: KIND_MOVE,
            flags: FLAG_BARREL_BUTTON,
            seq: 4242,
            t_us: 1_234_567,
            x: 0.25,
            y: 0.75,
            pressure: 0.5,
            tilt_x: -12,
            tilt_y: 30,
        };
        let b = p.encode();
        assert_eq!(b.len(), PACKET_LEN);
        assert_eq!(PenPacket::decode(&b), p);
    }

    #[test]
    fn handshake_roundtrip_and_bad_magic() {
        let h = Handshake {
            version: PROTOCOL_VERSION,
            caps: CAP_PRESSURE | CAP_HOVER,
            width: 1080,
            height: 2400,
        };
        let b = h.encode();
        assert_eq!(Handshake::decode(&b), Some(h));
        let mut bad = b;
        bad[0] ^= 0xFF;
        assert_eq!(Handshake::decode(&bad), None);
    }

    #[test]
    fn seq_gap_detects_loss() {
        let mut p = PenPacket {
            kind: KIND_MOVE,
            flags: 0,
            seq: 11,
            t_us: 0,
            x: 0.0,
            y: 0.0,
            pressure: 0.0,
            tilt_x: 0,
            tilt_y: 0,
        };
        assert_eq!(p.seq_gap(10), 0); // 10 -> 11: tidak ada yang hilang
        p.seq = 15;
        assert_eq!(p.seq_gap(11), 3); // 12,13,14 hilang
        p.seq = 0;
        assert_eq!(p.seq_gap(65535), 0); // 65535 -> 0: tidak ada yang hilang
        p.seq = 1;
        assert_eq!(p.seq_gap(65535), 1); // 65535 -> 1: paket 0 hilang
    }
}
