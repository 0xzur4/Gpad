# Gpad — HP jadi pen tablet via USB (Fase 1)

Mengubah ponsel/tablet Android menjadi alas gambar untuk PC Windows,
terhubung lewat kabel USB agar jeda sekecil mungkin.

## Status: Fase 4 (ukur & perkecil jeda)

- [x] Fase 1–3 selesai (lihat di bawah).
- [x] Fase 4 — perbaikan nyata:
  - Android: thread jaringan prioritas `THREAD_PRIORITY_URGENT_DISPLAY`;
    jalur panas TANPA alokasi (pool array 24-byte + encode manual LE,
    tanpa `ByteBuffer` per titik, tanpa tekanan GC); tulis TCP per batch
    (1 flush untuk ≤16 paket, bukan 1 flush per paket).
  - Daemon: pembacaan blokir TANPA timeout (tanpa bangun palsu);
    thread bernama (`gpad-hotkey`, `gpad-tray`, `gpad-profiles`);
    statistik latensi proses per paket tiap 2000 paket (avg/p95/maks).
- [x] Angka terukur (jujur, bukan tebakan) — komponen "proses daemon",
      10.000 paket, VM Linux + mock injector:
      **avg <1µs, p95 <1µs, maks 42–91µs**.
      Ini BUKAN angka Windows/injeksi nyata — angka itu hanya bisa diukur
      di hardware zura (lihat metode di bawah).
- [ ] Pengukuran end-to-end di hardware asli (metode kamera 240fps):
      rekam sentuh stylus → garis muncul di Krita; hitung frame di antaranya
      (1 frame ≈ 4.2ms). Pisahkan komponen: sentuh Android (bandingkan
      `t_us` paket vs waktu tulis di log debug), jaringan USB (dominan kecil),
      proses daemon (statistik di atas, di Windows mencakup
      `InjectSyntheticPointerInput`), render aplikasi (sisa dari total kamera).

## Status: Fase 3 (kenyamanan pakai)

- [x] Fase 1–2: protokol, daemon pen injection Windows Ink, aplikasi Android.
- [x] Fase 3:
  - Mode absolut (area: `screen` | `monitor:N` | `custom`, opsi `keep_aspect`)
    dan mode relatif (sensitivitas, tanpa akselerasi, reset acuan saat angkat).
  - Ganti mode 4 cara: tombol di HP, tombol barrel stylus, hotkey global
    (`ctrl+alt+g`, bisa diubah/dimatikan), menu tray. Daemon membalas paket
    `GMOD` agar indikator mode di HP selalu sinkron.
  - System tray Windows (ikon + tooltip + menu mode/keluar).
  - `gpad.toml`: semua pengaturan + profil per aplikasi (mis. Photoshop
    otomatis absolut, Krita sensitivitas 1.2). Lihat `gpad.example.toml`.
  - Kurva tekanan pindah ke config (env `GPAD_PRESSURE_GAMMA` tetap menimpa).
  - Diagnostik ADB: deteksi kabel charge-only vs perangkat belum dipercaya.
- [x] Verifikasi: 14 test lolos; target `x86_64-pc-windows-gnu` compile bersih
      tanpa warning (semua modul); smoke test socket: handshake, hover/down/
      move/up, ganti mode via paket + balasan GMOD dua arah.
- [ ] Uji di hardware asli (Windows + A56): bangun `.exe` (`cargo build --release`),
      bangun APK di Android Studio, `adb reverse`, uji di Krita.

## Batasan hardware jujur (Samsung A56)

A56 **tidak punya stylus aktif** (S Pen). Artinya di Fase 1–2:
- x/y/hover-jari: BISA diuji penuh.
- pressure: Android memberi nilai tekanan jari (bervariasi, tidak presisi).
- tilt presisi & tombol barrel: TIDAK bisa diuji — field protokolnya sudah
  ada, tapi validasi perilaku nyata butuh perangkat ber-stylus.

## Cara build

### Daemon (di PC Windows)

```powershell
# butuh Rust: https://rustup.rs/
cd gpad
cargo build --release
# -> target\release\gpad-daemon.exe
```

### Aplikasi Android (di PC, pakai Android Studio)

1. Buka folder `gpad/android` di Android Studio.
2. Biarkan Gradle sync (butuh internet sekali).
3. Run ▶ ke HP (atau Build > Build APK).

## Cara pakai (Fase 1)

1. Di PC: jalankan `gpad-daemon.exe`.
2. Colok HP dengan **kabel DATA** (bukan kabel charge-only).
3. Aktifkan USB debugging di HP, izinkan PC.
4. Di PC: `adb reverse tcp:51337 tcp:51337`
   (daemon juga mencoba otomatis saat start).
5. Buka aplikasi Gpad di HP — status "Tersambung".
6. Sentuh/geser di HP → kursor PC mengikuti (mode absolut seluruh layar).

## Troubleshooting

| Gejala | Penyebab umum |
|---|---|
| Status "PC tidak terjangkau" | Kabel charge-only; USB debugging mati; `adb reverse` belum jalan; daemon belum start |
| Kursor tidak gerak tapi tersambung | Cek log daemon di terminal |
| Garis patah-patah | Titik historis tidak terkirim (cek log); naikkan prioritas thread di HP |

## Peta jalan

- **Fase 2**: tekanan & tilt → suntik sebagai input pena Windows Ink
  (CreateSyntheticPointerDevice), uji ketebalan garis di Krita.
- **Fase 3**: area terpilih, mode relatif, kurva tekanan, tray, profil.
- **Fase 4**: ukur jeda dengan kamera high-speed, bandingkan vs Weylus.
- **Fase 5**: macOS/Linux, iOS via usbmuxd, installer.
