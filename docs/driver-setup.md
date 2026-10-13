# Driver Setup — Gpad v0.5.0 (satu kali saja)

> Agar PC bisa mengaktifkan **mode accessory** di HP Samsung, Windows harus
> mengizinkan aplikasi Gpad berbicara langsung ke HP lewat USB. Caranya:
> pasang driver **WinUSB** satu kali memakai aplikasi gratis **Zadig**.

## Yang dibutuhkan

- Kabel **USB DATA** (bukan kabel cas saja) dari HP ke PC.
- HP Samsung A56 (atau Android lain yang mendukung USB accessory).
- Aplikasi **Zadig** (gratis, portable): https://zadig.akeo.ie/

## Langkah-langkah

1. **Colok HP ke PC** dengan kabel USB. Di HP, pilih mode USB apa saja
   (mis. "Transfer file / MTP") — tidak penting, yang penting terdeteksi.

2. **Jalankan Zadig sebagai Administrator**
   (klik kanan `zadig-2.x.exe` → *Run as administrator*).

3. Di Zadig, buka menu **Options → List All Devices**.

4. Pada daftar dropdown, cari perangkat HP Samsung.
   Biasanya bernama **"SAMSUNG Mobile USB Composite Device"** atau
   **"SAMSUNG_Android"** dengan USB ID seperti `04E8:6860`.
   ⚠️ **Pilih yang benar.** Jangan pilih mouse/keyboard/dongle lain.

5. Di kolom kanan (driver target), pilih **WinUSB**.

6. Klik **Install Driver** (atau *Replace Driver*). Tunggu sampai selesai.

7. **Cabut dan colok ulang** kabel USB.

8. Jalankan `gpad-daemon.exe`. Di HP, buka aplikasi **Gpad** lalu tap
   **Hubungkan**. Status harusnya berubah menjadi
   "Tersambung — menggambar!".

## Catatan penting (baca dulu)

- **Smart Switch / Samsung USB Driver bisa terganggu.** Mengganti driver
  composite Samsung ke WinUSB berarti Windows tidak lagi memakai driver
  resmi Samsung untuk perangkat itu — Samsung Smart Switch mungkin tidak
  mengenali HP lagi.
- **Cara kembali seperti semula:** buka *Device Manager* → cari perangkat
  Samsung → klik kanan → *Uninstall device* → **centang "Delete the driver
  software"** → cabut-colok USB. Windows akan memakai driver Samsung
  aslinya lagi.
- Langkah ini **cukup sekali** — tidak perlu diulang tiap pakai Gpad.
- Bila nanti HP berganti, ulangi langkah di atas untuk perangkat baru.

## Bila masih gagal

1. Pastikan Zadig dijalankan sebagai Administrator.
2. Pastikan kabel adalah kabel data (coba kabel lain).
3. Pastikan hanya SATU HP yang tercolok saat daemon dijalankan.
4. Jalankan `gpad-daemon.exe --console` untuk melihat pesan error —
   kirim pesan itu ke pengembang (asisten) untuk dibantu.

## Kenapa ini diperlukan? (penjelasan singkat)

Android punya mode khusus bernama **Accessory Mode**: HP "menyerah" dan
membiarkan PC yang mengatur koneksi USB. Untuk memerintahkannya, PC harus
mengirim perintah USB tingkat rendah — dan itu hanya bisa dilakukan bila
Windows memakai driver generik (WinUSB), bukan driver MTP/Samsung.
Setelah perintah terkirim, HP muncul sebagai perangkat baru
(`18D1:2D00`) dan Gpad langsung jalan tanpa ADB sama sekali.
