//! usb.rs — transport Android Open Accessory (AOA) via rusb.
//!
//! Alur koneksi:
//!  1. Cari perangkat Android mode normal (bukan 18D1) yang menjawab
//!     GET_PROTOCOL. BUTUH driver WinUSB terikat ke perangkat (lihat
//!     docs/driver-setup.md) — tanpa itu `open()` gagal dan kita lapor
//!     jelas, bukan hang.
//!  2. Kirim string identifikasi (produsen/model/deskripsi/versi/uri/serial),
//!     lalu START_ACCESSORY. Perangkat lepas dari bus & enumerasi ulang
//!     sebagai VID 0x18D1 PID 0x2D00/0x2D01.
//!  3. Buka perangkat accessory, claim interface 0, pakai bulk IN/OUT.
//!
//! Di atas endpoint bulk, pesan dibingkai oleh gpad_protocol::framing
//! (length-prefixed). Bulk read bisa short — reader loop sampai frame
//! lengkap atau timeout.

use gpad_protocol::{channel_known, decode_frame_header, encode_frame, FRAME_HEADER_LEN};
use rusb::{Context, Device, DeviceHandle, DeviceList, Direction, TransferType};
use std::time::Duration;

// --- AOA ---
const AOA_VID: u16 = 0x18D1;
const AOA_PIDS: [u16; 2] = [0x2D00, 0x2D01]; // accessory, accessory+adb
const REQ_GET_PROTOCOL: u8 = 51;
const REQ_SEND_IDENT: u8 = 52;
const REQ_START_ACCESSORY: u8 = 53;
const RT_IN_VENDOR: u8 = 0xC0; // IN | vendor | device
const RT_OUT_VENDOR: u8 = 0x40; // OUT | vendor | device

const IDENT_MANUFACTURER: &str = "HiSaTech";
const IDENT_MODEL: &str = "Gpad";
const IDENT_DESCRIPTION: &str = "Gpad pen display";
const IDENT_VERSION: &str = "0.5.0";
const IDENT_URI: &str = "https://github.com/0xzur4/Gpad";
const IDENT_SERIAL: &str = "gpad-001";

const CTRL_TIMEOUT: Duration = Duration::from_millis(1000);
const BULK_TIMEOUT: Duration = Duration::from_millis(1000);
/// Tunggu enumerasi ulang accessory setelah START_ACCESSORY.
const REENUM_WAIT: Duration = Duration::from_secs(12);

#[derive(Debug)]
pub enum UsbError {
    Rusb(rusb::Error),
    NoDevice,
    Timeout(&'static str),
    Frame(&'static str),
}

impl std::fmt::Display for UsbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UsbError::Rusb(e) => write!(f, "USB: {e}"),
            UsbError::NoDevice => write!(f, "perangkat Android tidak ditemukan"),
            UsbError::Timeout(s) => write!(f, "timeout: {s}"),
            UsbError::Frame(s) => write!(f, "frame rusak: {s}"),
        }
    }
}

impl From<rusb::Error> for UsbError {
    fn from(e: rusb::Error) -> Self {
        UsbError::Rusb(e)
    }
}

/// Koneksi bulk ke HP dalam mode accessory.
pub struct UsbLink {
    handle: DeviceHandle<Context>,
    ep_in: u8,
    ep_out: u8,
    /// Buffer baca parsial antar poll (bulk bisa short-read).
    rbuf: Vec<u8>,
}

impl UsbLink {
    /// Coba baca satu frame; Ok(None) bila belum ada data lengkap dalam
    /// `timeout`. Data parsial disimpan di rbuf — tidak hilang.
    pub fn poll_frame(&mut self, timeout: Duration) -> Result<Option<(u8, Vec<u8>)>, UsbError> {
        if !self.fill(FRAME_HEADER_LEN, timeout)? {
            return Ok(None);
        }
        let fh = decode_frame_header(&self.rbuf[..FRAME_HEADER_LEN])
            .ok_or(UsbError::Frame("header invalid"))?;
        if !channel_known(fh.channel) {
            // Desync defensif (seharusnya tak terjadi): buang 1 byte.
            self.rbuf.drain(..1);
            return Ok(None);
        }
        if !self.fill(fh.len, Duration::from_millis(0))? {
            return Ok(None);
        }
        let channel = fh.channel;
        let payload = self.rbuf[FRAME_HEADER_LEN..fh.len].to_vec();
        self.rbuf.drain(..fh.len);
        Ok(Some((channel, payload)))
    }

    /// Isi rbuf hingga `need` byte. Ok(true) = tercapai.
    fn fill(&mut self, need: usize, timeout: Duration) -> Result<bool, UsbError> {
        let mut first = true;
        while self.rbuf.len() < need {
            let t = if first { timeout } else { Duration::from_millis(0) };
            first = false;
            let mut tmp = [0u8; 4096];
            match self.handle.read_bulk(self.ep_in, &mut tmp, t) {
                Ok(0) => return Ok(false),
                Ok(n) => self.rbuf.extend_from_slice(&tmp[..n]),
                Err(rusb::Error::Timeout) => return Ok(false),
                Err(e) => return Err(UsbError::Rusb(e)),
            }
        }
        Ok(true)
    }

    /// Tulis satu frame (diframing di sini agar pemanggil cukup kirim payload).
    pub fn write_frame(&mut self, channel: u8, payload: &[u8]) -> Result<(), UsbError> {
        let frame = encode_frame(channel, payload);
        let mut off = 0;
        while off < frame.len() {
            match self.handle.write_bulk(self.ep_out, &frame[off..], BULK_TIMEOUT) {
                Ok(n) => off += n,
                Err(e) => return Err(UsbError::Rusb(e)),
            }
        }
        Ok(())
    }
}

/// Top-level: dapatkan UsbLink ke HP (switch ke accessory bila perlu).
pub fn connect() -> Result<UsbLink, UsbError> {
    // 1. Sudah dalam mode accessory? (sesi sebelumnya / user mencolok ulang cepat)
    let devices = DeviceList::new_with_context(Context::new()?)?;
    if let Some(link) = try_open_accessory(&devices)? {
        eprintln!("perangkat sudah dalam mode accessory");
        return Ok(link);
    }
    // 2. Cari perangkat normal yang mendukung AOA, lalu switch.
    let dev = find_aoa_capable(&devices)?.ok_or(UsbError::NoDevice)?;
    eprintln!("perangkat AOA ditemukan, switching ke accessory mode…");
    switch_to_accessory(&dev)?;
    // Perangkat lepas dari bus; tunggu enumerasi ulang.
    let start = std::time::Instant::now();
    while start.elapsed() < REENUM_WAIT {
        let devices = DeviceList::new_with_context(Context::new()?)?;
        if let Some(link) = try_open_accessory(&devices)? {
            return Ok(link);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(UsbError::Timeout("enumerasi ulang accessory"))
}

/// Buka perangkat accessory (18D1:2D00/2D01) bila ada.
fn try_open_accessory(devices: &DeviceList<Context>) -> Result<Option<UsbLink>, UsbError> {
    for dev in devices.iter() {
        let desc = match dev.device_descriptor() {
            Ok(d) => d,
            Err(_) => continue,
        };
        if desc.vendor_id() != AOA_VID || !AOA_PIDS.contains(&desc.product_id()) {
            continue;
        }
        match open_bulk_link(&dev) {
            Ok(link) => return Ok(Some(link)),
            Err(e) => {
                eprintln!("gagal buka accessory {e} — cek driver WinUSB (docs/driver-setup.md)");
                return Err(e);
            }
        }
    }
    Ok(None)
}

/// Cari perangkat mode normal yang menjawab GET_PROTOCOL (1/2).
/// Perangkat yang tidak bisa di-open (driver) dilewati dengan pesan jelas.
fn find_aoa_capable(devices: &DeviceList<Context>) -> Result<Option<Device<Context>>, UsbError> {
    let mut skipped_driver = 0;
    for dev in devices.iter() {
        let desc = match dev.device_descriptor() {
            Ok(d) => d,
            Err(_) => continue,
        };
        if desc.vendor_id() == AOA_VID {
            continue; // sudah accessory — ditangani try_open_accessory
        }
        if desc.class_code() == 0x09 {
            continue; // hub
        }
        let handle = match dev.open() {
            Ok(h) => h,
            Err(_) => {
                skipped_driver += 1;
                continue;
            }
        };
        let mut buf = [0u8; 2];
        match handle.read_control(
            RT_IN_VENDOR,
            REQ_GET_PROTOCOL,
            0,
            0,
            &mut buf,
            Duration::from_millis(500),
        ) {
            Ok(2) => {
                let ver = u16::from_le_bytes(buf);
                if ver == 1 || ver == 2 {
                    eprintln!(
                        "AOA v{ver} didukung oleh {:04x}:{:04x}",
                        desc.vendor_id(),
                        desc.product_id()
                    );
                    return Ok(Some(dev));
                }
            }
            _ => {} // bukan perangkat AOA / stall — lewati
        }
    }
    if skipped_driver > 0 {
        eprintln!(
            "catatan: {skipped_driver} perangkat USB tidak bisa di-open \
             (butuh driver WinUSB via Zadig — lihat docs/driver-setup.md)"
        );
    }
    Ok(None)
}

/// Kirim string identifikasi lalu START_ACCESSORY.
fn switch_to_accessory(dev: &Device<Context>) -> Result<(), UsbError> {
    let handle = dev.open()?;
    let idents = [
        IDENT_MANUFACTURER,
        IDENT_MODEL,
        IDENT_DESCRIPTION,
        IDENT_VERSION,
        IDENT_URI,
        IDENT_SERIAL,
    ];
    for (i, s) in idents.iter().enumerate() {
        let mut data = s.as_bytes().to_vec();
        data.push(0); // NUL-terminated
        handle.write_control(RT_OUT_VENDOR, REQ_SEND_IDENT, 0, i as u16, &data, CTRL_TIMEOUT)?;
    }
    handle.write_control(RT_OUT_VENDOR, REQ_START_ACCESSORY, 0, 0, &[], CTRL_TIMEOUT)?;
    eprintln!("START_ACCESSORY terkirim");
    Ok(())
}

/// Claim interface bulk accessory & temukan endpoint IN/OUT.
fn open_bulk_link(dev: &Device<Context>) -> Result<UsbLink, UsbError> {
    let handle = dev.open()?;
    let cfg = dev.active_config_descriptor()?;
    let mut ep_in = None;
    let mut ep_out = None;
    let mut if_num = 0;
    'outer: for iface in cfg.interfaces() {
        for idesc in iface.descriptors() {
            if_num = idesc.interface_number();
            for ep in idesc.endpoint_descriptors() {
                if ep.transfer_type() != TransferType::Bulk {
                    continue;
                }
                match ep.direction() {
                    Direction::In => ep_in = Some(ep.address()),
                    Direction::Out => ep_out = Some(ep.address()),
                }
            }
            if ep_in.is_some() && ep_out.is_some() {
                break 'outer;
            }
        }
    }
    let (ep_in, ep_out) = match (ep_in, ep_out) {
        (Some(i), Some(o)) => (i, o),
        _ => return Err(UsbError::Frame("endpoint bulk tidak lengkap")),
    };
    // Detach kernel driver bila perlu (Linux); di Windows no-op.
    let _ = handle.detach_kernel_driver(if_num);
    handle.claim_interface(if_num)?;
    eprintln!("bulk IN=0x{ep_in:02x} OUT=0x{ep_out:02x} (iface {if_num})");
    Ok(UsbLink { handle, ep_in, ep_out, rbuf: Vec::new() })
}
