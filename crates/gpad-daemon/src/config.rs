//! config.rs — konfigurasi TOML Gpad v0.5.0.
//!
//! File `gpad.toml` di folder yang sama dengan exe (atau %APPDATA%/Gpad).
//! Tidak ada file = semua default. Contoh: gpad.example.toml.
//! v0.5.0: single mode (Cintiq) — hanya pengaturan video & tekanan.

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub video: Video,
    pub pressure: Pressure,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Video {
    /// Frame per detik target (1..60).
    pub fps: u32,
    /// Bitrate kilobit per detik (500..8000).
    pub bitrate_kbps: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Pressure {
    /// Kurva tekanan (env GPAD_PRESSURE_GAMMA menimpa ini).
    pub gamma: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            video: Video::default(),
            pressure: Pressure::default(),
        }
    }
}
impl Default for Video {
    fn default() -> Self {
        Self { fps: 30, bitrate_kbps: 3000 }
    }
}
impl Default for Pressure {
    fn default() -> Self {
        Self { gamma: 1.0 }
    }
}

impl Config {
    /// Cari gpad.toml di folder exe, lalu %APPDATA%/Gpad. Tidak ada = default.
    pub fn load() -> Self {
        for p in Self::candidates() {
            if let Ok(text) = std::fs::read_to_string(&p) {
                match toml::from_str::<Config>(&text) {
                    Ok(c) => {
                        eprintln!("config dimuat: {}", p.display());
                        return c;
                    }
                    Err(e) => eprintln!("config {} rusak: {e} — pakai default", p.display()),
                }
            }
        }
        eprintln!("gpad.toml tidak ditemukan — pakai default");
        Self::default()
    }

    fn candidates() -> Vec<PathBuf> {
        let mut v = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                v.push(dir.join("gpad.toml"));
            }
        }
        v.push(PathBuf::from("gpad.toml")); // folder kerja (dev)
        #[cfg(windows)]
        if let Ok(appdata) = std::env::var("APPDATA") {
            v.push(PathBuf::from(appdata).join("Gpad").join("gpad.toml"));
        }
        v
    }

    /// gamma efektif: env menimpa file.
    pub fn gamma(&self) -> f32 {
        std::env::var("GPAD_PRESSURE_GAMMA")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|&g: &f32| g > 0.0 && g <= 4.0)
            .unwrap_or(self.pressure.gamma)
            .clamp(0.25, 4.0)
    }

    pub fn fps(&self) -> u32 {
        self.video.fps.clamp(1, 60)
    }

    pub fn bitrate_bps(&self) -> u32 {
        (self.video.bitrate_kbps.clamp(500, 8000)) * 1000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_sane() {
        let c = Config::default();
        assert_eq!(c.fps(), 30);
        assert_eq!(c.bitrate_bps(), 3_000_000);
        assert!((c.gamma() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn parses_example() {
        let toml_text = r#"
[video]
fps = 24
bitrate_kbps = 2000
[pressure]
gamma = 1.8
"#;
        let c: Config = toml::from_str(toml_text).unwrap();
        assert_eq!(c.fps(), 24);
        assert_eq!(c.bitrate_bps(), 2_000_000);
        assert!((c.gamma() - 1.8).abs() < 1e-6);
    }

    #[test]
    fn clamps_ranges() {
        let c: Config = toml::from_str("[video]\nfps=999\nbitrate_kbps=1").unwrap();
        assert_eq!(c.fps(), 60);
        assert_eq!(c.bitrate_bps(), 500_000);
    }
}
