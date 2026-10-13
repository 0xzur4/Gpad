# Gpad — Turn Your Phone into a Pen Display over USB

Turn an Android phone/tablet into a pen display for a Windows PC.
The PC screen is mirrored to the phone over a USB cable for minimal latency;
you draw on the phone while seeing the screen, and the PC shows it too.

## Status: v0.5.0 "Cintiq mode" (built, needs hardware testing)

v0.5.0 replaces the previous blind-tablet approach (v0.4.x, ADB reverse
transport) with a pen-display architecture:

- **Transport: USB Accessory Mode (AOA)** — no ADB, no platform-tools,
  no `adb reverse`. Plug the cable, tap one "Connect" button.
  Length-prefixed packets over USB bulk endpoints, heartbeat every second,
  auto-reconnect with backoff. Protocol spec: `docs/protocol-v05.md`.
- **Video downstream**: Windows DXGI desktop duplication → scaled to ≤1080p →
  H.264 encode (OpenH264, statically linked — no DLL to bundle) → USB →
  Android MediaCodec hardware decode → displayed on the phone.
  Target 30 fps at ~3 Mbps.
- **Input upstream**: touch on the phone → mapped from the displayed video
  rect to absolute PC desktop coordinates → injected as pen input
  (Windows Ink synthetic pointer device, reused from v0.4.x).
- Mirrors the full primary display (letterboxed on the phone).
- PC daemon: tray icon with status / fps / RTT; `--console` flag for
  diagnostics; runs without a console window by default.
- Android app: single "Hubungkan" button, status line (connecting /
  connected / fps / RTT), full-screen landscape video.

Expected glass-to-glass latency: ~50–150 ms (honest estimate from the
component budget: capture + encode + USB + decode; real measurement
requires the hardware setup).

Built and unit-tested on 2026-10-13 (27 Rust tests pass, APK compiles and
is signed). Not yet tested end-to-end on real hardware — the AOA USB
handshake, the WinUSB/Zadig step, and the MediaCodec pipeline all need a
real PC + phone run.

## Honest hardware limitations (Samsung A56)

The A56 has **no active stylus** (no S Pen). That means:

- x/y finger position: fully testable.
- pressure: the app sends a **constant emulated pressure of 0.65** for
  finger touches (Android finger pressure is too noisy to use directly);
  the daemon's `[pressure] gamma` config curve still applies.
- precise tilt & barrel button: protocol fields exist, but real behavior
  needs stylus hardware.

## Building

### Daemon (on the Windows PC)

```powershell
# needs Rust: https://rustup.rs/
cd gpad
cargo build --release
# -> target\release\gpad-daemon.exe
```

### Android app (on a PC with Android Studio)

1. Open the `gpad/android` folder in Android Studio.
2. Let Gradle sync (needs internet once).
3. Run ▶ to the phone (or Build > Build APK).

(The v0.5.0 APK was also built successfully with plain SDK command-line
tools — aapt2 + kotlinc + d8 + apksigner — so Android Studio is
convenient but not required.)

## Usage (v0.5.0)

1. On the PC: run `gpad-daemon.exe`.
2. Connect the phone with a **data cable** (not charge-only).
3. One-time: install the WinUSB driver for the phone via Zadig
   (see `docs/driver-setup.md`).
4. Open the Gpad app on the phone, tap **Connect**.
5. The PC screen appears on the phone; touch/drag draws on the PC.

## Troubleshooting

| Symptom | Common cause |
|---|---|
| "PC unreachable" | Charge-only cable; WinUSB driver not installed; daemon not running |
| Connected but cursor doesn't move | Check the daemon log (run with `--console`) |
| Choppy lines | Touch batching issue (check app log); USB contention |

## Roadmap

- **v0.5.0**: Cintiq mode (this release) — AOA transport, H.264 mirror,
  touch-to-pen, one-button connect.
- **v0.5.1**: per-window/region mirror selection, pressure curve tuning.
- **v0.6**: macOS/Linux daemon, iOS via usbmuxd, installer.

## History

- v0.4.x: blind-tablet mode over `adb reverse` TCP (superseded).
