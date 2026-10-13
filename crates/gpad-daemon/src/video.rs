//! video.rs — pipeline video downstream (Windows only).
//!
//! DXGI desktop duplication -> scale ke ≤1080p -> BGRA→RGB→YUV420 ->
//! OpenH264 (statik, tanpa DLL) -> NAL Annex-B per frame.
//!
//! Strategi keyframe: IDR paksa tiap ~2 detik + IDR pertama; SPS/PPS
//! ber-ID naik (IncreasingId) agar decoder bisa deteksi frame hilang.
//! Decoder HP (MediaCodec) dikonfigurasi dari SPS/PPS frame IDR pertama.

use crate::vidutil::{fit_dims, scale_bgra_to_rgb_bilinear};
use dxgcap::{CaptureError, DXGIManager};
use gpad_protocol::{VideoHeader, VIDEO_FLAG_KEYFRAME};
use openh264::encoder::{
    BitRate, Encoder, EncoderConfig, FrameRate, RateControlMode, SpsPpsStrategy, UsageType,
};
use openh264::formats::{RgbSliceU8, YUVBuffer};
use openh264::nal_units;
use openh264::OpenH264API;
use std::time::{Duration, Instant};

/// IDR paksa tiap N frame (recovery bila paket hilang).
const FORCED_IDR_EVERY: u32 = 60;

pub struct VideoPipeline {
    cap: DXGIManager,
    encoder: Encoder,
    yuv: YUVBuffer,
    enc_w: usize,
    enc_h: usize,
    desk_w: usize,
    desk_h: usize,
    rgb: Vec<u8>,
    annexb: Vec<u8>,
    seq: u32,
    frames_encoded: u64,
    bytes_encoded: u64,
    last_stat: Instant,
}

impl VideoPipeline {
    pub fn new(desk_w: usize, desk_h: usize, bitrate_bps: u32) -> Result<Self, String> {
        let (enc_w, enc_h) = fit_dims(desk_w, desk_h);
        let mut cap = DXGIManager::new(50).map_err(|e| format!("DXGI init gagal: {e}"))?;
        cap.set_capture_source_index(0); // display utama
        let config = EncoderConfig::new()
            .usage_type(UsageType::ScreenContentRealTime)
            .bitrate(BitRate::from_bps(bitrate_bps))
            .max_frame_rate(FrameRate::from_hz(30.0))
            .rate_control_mode(RateControlMode::Bitrate)
            .sps_pps_strategy(SpsPpsStrategy::IncreasingId);
        let api = OpenH264API::from_source();
        let mut encoder =
            Encoder::with_api_config(api, config).map_err(|e| format!("encoder gagal: {e:?}"))?;
        encoder.force_intra_frame(); // frame pertama = IDR
        eprintln!("video: desktop {desk_w}x{desk_h} -> encode {enc_w}x{enc_h}, {bitrate_bps}bps");
        Ok(Self {
            cap,
            encoder,
            yuv: YUVBuffer::new(enc_w, enc_h),
            enc_w,
            enc_h,
            desk_w,
            desk_h,
            rgb: vec![0u8; enc_w * enc_h * 3],
            annexb: Vec::with_capacity(256 * 1024),
            seq: 0,
            frames_encoded: 0,
            bytes_encoded: 0,
            last_stat: Instant::now(),
        })
    }

    pub fn enc_size(&self) -> (usize, usize) {
        (self.enc_w, self.enc_h)
    }

    /// Capture + encode satu frame. Ok(None) = tidak ada frame baru
    /// (timeout) atau frame di-skip encoder. Ok(Some(payload)) =
    /// payload CH_VIDEO siap kirim (sudah termasuk VideoHeader).
    pub fn pump(&mut self) -> Result<Option<Vec<u8>>, String> {
        let (bgra, (cw, ch)) = match self.cap.capture_frame_components() {
            Ok(f) => f,
            Err(CaptureError::Timeout) => return Ok(None),
            Err(CaptureError::AccessLost) => {
                // Coba pulihkan duplikasi (mis. ganti mode fullscreen).
                let _ = self.cap.acquire_output_duplication();
                return Ok(None);
            }
            Err(e) => return Err(format!("capture gagal: {e:?}")),
        };
        if cw != self.desk_w || ch != self.desk_h {
            eprintln!("peringatan: resolusi desktop berubah {cw}x{ch}");
        }
        // Scale BGRA -> RGB, lalu RGB -> YUV420.
        scale_bgra_to_rgb_bilinear(&bgra, cw, ch, &mut self.rgb, self.enc_w, self.enc_h);
        let (w, h) = (self.enc_w, self.enc_h);
        self.yuv.read_rgb8(RgbSliceU8::new(&self.rgb, (w, h)));

        if self.seq % FORCED_IDR_EVERY == 0 {
            self.encoder.force_intra_frame();
        }
        let bitstream = self
            .encoder
            .encode(&self.yuv)
            .map_err(|e| format!("encode gagal: {e:?}"))?;

        // to_vec() sudah Annex-B; normalisasi ke start code 4-byte dan
        // deteksi keyframe (NAL type 5 = IDR).
        let raw = bitstream.to_vec();
        self.annexb.clear();
        let mut keyframe = false;
        for nal in nal_units(&raw) {
            let mut i = 0;
            while i < nal.len() && nal[i] == 0 {
                i += 1;
            }
            if i < nal.len() && nal[i] == 1 {
                i += 1;
            }
            if i < nal.len() && nal[i] & 0x1F == 5 {
                keyframe = true;
            }
            self.annexb.extend_from_slice(&[0, 0, 0, 1]);
            self.annexb.extend_from_slice(&nal[i.min(nal.len())..]);
        }
        if self.annexb.is_empty() {
            return Ok(None); // frame di-skip encoder (rate control)
        }
        let hdr = VideoHeader {
            seq: self.seq,
            capture_ts_us: monotonic_us(),
            flags: if keyframe { VIDEO_FLAG_KEYFRAME } else { 0 },
        };
        self.seq = self.seq.wrapping_add(1);
        self.frames_encoded += 1;
        self.bytes_encoded += self.annexb.len() as u64;

        let mut payload = Vec::with_capacity(15 + self.annexb.len());
        payload.extend_from_slice(&hdr.encode());
        payload.extend_from_slice(&self.annexb);

        if self.last_stat.elapsed() >= Duration::from_secs(5) {
            let el = self.last_stat.elapsed().as_secs_f64();
            let fps = self.frames_encoded as f64 / el;
            let kbps = self.bytes_encoded as f64 * 8.0 / el / 1000.0;
            eprintln!("video: {fps:.1} fps, {kbps:.0} kbps");
            self.frames_encoded = 0;
            self.bytes_encoded = 0;
            self.last_stat = Instant::now();
        }
        Ok(Some(payload))
    }
}

/// Mikrodetik monotonik sejak program mulai.
fn monotonic_us() -> u64 {
    use std::sync::OnceLock;
    static T0: OnceLock<Instant> = OnceLock::new();
    let t0 = T0.get_or_init(Instant::now);
    t0.elapsed().as_micros() as u64
}
