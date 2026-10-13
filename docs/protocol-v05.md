# Gpad Protocol v0.5.0 ("Cintiq mode")

Transport: **Android Open Accessory (AOA)** over USB. The PC initiates
accessory mode via USB control transfers, then the Android phone (acting as
the USB accessory) and the PC talk over bulk endpoints. **No ADB, no TCP.**

Wire framing is identical on both sides:

```
Frame = [u32 LE total_len][u8 channel][payload]
```

- `total_len` includes the 5-byte header itself.
- Max frame: 8 MiB.
- Channels are multiplexed over ONE bulk endpoint pair (simpler than two
  pairs; ordering between video/input is preserved).

## Channels

| ID   | Name       | Direction | Contents |
| ---- | ---------- | --------- | -------- |
| 0x01 | CONTROL    | both      | Handshake / HandshakeReply |
| 0x02 | VIDEO      | PC -> HP  | VideoHeader + H.264 Annex-B |
| 0x03 | INPUT      | HP -> PC  | PenPacket (24 bytes) |
| 0x04 | HEARTBEAT  | both      | u64 LE timestamp (µs) |
| 0x05 | STATS      | PC -> HP  | fps, rtt, bitrate |

## Handshake (channel CONTROL)

1. Phone -> PC, 16 bytes: `"GPD5"` magic (0x35445047), u16 version (=2),
   u16 phone screen w, u16 phone screen h, u16 caps, u16 reserved.
   Caps bits: `0x0001` pressure, `0x0004` hover, `0x0010` video-receive.
2. PC -> phone, 20 bytes: magic, u16 version, u16 desktop w, u16 desktop h,
   u16 video w, u16 video h, u8 fps, 3 bytes reserved.
3. Any mismatch (magic/version) aborts the session.

## Video (channel VIDEO)

Each frame: **VideoHeader (15 bytes)** + H.264 Annex-B byte stream.

```
VideoHeader = [u32 LE seq][u64 LE capture_ts_us][u8 flags]
```

- `flags` bit 0 = keyframe (IDR + SPS/PPS). The phone waits for the first
  keyframe, extracts SPS (NAL type 7) and PPS (NAL type 8) to configure
  MediaCodec, then decodes each access unit.
- `capture_ts_us` is the PC's capture timestamp — used by the phone to
  estimate glass-to-glass latency (display time minus capture time).
- Encoder: Cisco OpenH264, baseline, 3 Mbps CBR-ish, 30 fps,
  forced IDR every 60 frames, SPS/PPS prepended to every IDR.
- Resolution: desktop scaled to fit within 1920x1080, even dimensions,
  letterboxed on the phone to preserve aspect.

## Input (channel INPUT)

**PenPacket (24 bytes, little-endian):**

```
[u8 kind][u8 flags][u16 seq][u32 t_us][f32 x][f32 y][f32 pressure][i16 tilt_x][i16 tilt_y]
```

- `kind`: 0=down, 1=move, 2=up, 3=hover.
- `x`, `y`: **0.0..1.0 normalized to the FULL PC desktop** (not the phone
  screen, not the video stream). The phone maps the touch point from the
  displayed (letterboxed) video rect to desktop coordinates.
- `flags` bit 2 (`0x04`) = finger touch.
- Pressure: the Samsung A56 has no active stylus, so the phone sends a
  constant emulated pressure of 0.65. The PC daemon's `[pressure] gamma`
  config still applies its curve to this value.

## Heartbeat & reconnect (channel HEARTBEAT)

- Each side sends its current timestamp (µs) every 1 s.
- Each side echoes heartbeats it didn't originate.
- If a sent heartbeat isn't echoed within **3.5 s**, the session is dead:
  both sides tear down and reconnect with exponential backoff
  (1, 2, 4, 8, 16, 30 s, capped at 30 s).
- This replaces the v0.4.x TCP heartbeat; the same 3.5 s budget applies.

## Stats (channel STATS, PC -> phone, 1/s)

5 bytes: `[u8 fps][u16 LE rtt_ms][u16 LE bitrate_kbps]`.
Shown on the phone status line and the PC tray tooltip.

## AOA bootstrap (PC side, `gpad-daemon`)

1. Find a USB device that is NOT yet in accessory mode: any device whose
   product ID isn't 0x2D00/0x2D01. (Heuristic: first device; the user has
   one phone plugged in.)
2. Control transfers on endpoint 0:
   - `GET_PROTOCOL` (request 51): expect protocol version ≥ 1.
   - `SEND_IDENT` (request 52) × 6: manufacturer "HiSaTech", model "Gpad",
     description "Gpad pen display", version "0.5.0",
     URI "https://github.com/0xzur4/Gpad", serial "GPAD0001".
   - `START_ACCESSORY` (request 53): phone re-enumerates as 18D1:2D00
     (accessory) or 18D1:2D01 (accessory+ADB).
3. Wait up to ~10 s for re-enumeration, open bulk IN/OUT endpoints
   (first bulk pair found), start the framed session above.

**Known Windows caveat:** the phone's stock Samsung/MTP driver claims the
device, so libusb/rusb can't always issue the control transfers. One-time
fix: Zadig → install WinUSB for the phone's composite device. See
`docs/driver-setup.md`. This is the only manual step in v0.5.0.

## File map (implementation)

- Rust: `crates/gpad-protocol/src/lib.rs` (framing, handshake, PenPacket,
  backoff, letterbox math — unit tested), `crates/gpad-daemon/src/usb.rs`
  (AOA bootstrap + framed session), `src/video.rs` (DXGI + OpenH264).
- Kotlin: `Proto.kt` (same constants — keep in sync!),
  `UsbLink.kt` (accessory I/O + heartbeat), `VideoDecoder.kt` (MediaCodec),
  `TouchView.kt` (touch → desktop mapping).
