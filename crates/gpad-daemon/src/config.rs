//! config.rs — konfigurasi TOML Gpad (Fase 3).
//!
//! File `gpad.toml` di folder yang sama dengan exe (atau %APPDATA%/Gpad).
//! Tidak ada file = semua default. Contoh lengkap ada di gpad.example.toml.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: General,
    pub absolute: Absolute,
    pub relative: Relative,
    pub pressure: Pressure,
    pub switching: Switching,
    /// Profil per aplikasi: nama -> Profile { process, mode, ... }.
    pub profile: HashMap<String, Profile>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct General {
    /// "absolute" | "relative"
    pub mode: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Absolute {
    /// "screen" | "monitor:N" | "custom"
    pub area: String,
    /// Persegi custom dalam piksel virtual screen (dipakai jika area="custom").
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// Kunci rasio aspek area (lingkaran tetap lingkaran).
    pub keep_aspect: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Relative {
    /// Pengali gerak (1.0 = 1:1 terhadap lebar area).
    pub sensitivity: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Pressure {
    /// Kurva tekanan (env GPAD_PRESSURE_GAMMA menimpa ini).
    pub gamma: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Switching {
    /// Tombol barrel saat DOWN = ganti mode.
    pub barrel_toggles_mode: bool,
    /// Pintasan global, mis. "ctrl+alt+g". Kosong = mati.
    pub hotkey: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Profile {
    /// Nama exe proses foreground, mis. "photoshop.exe".
    pub process: String,
    /// mode yang dipakai saat profil aktif ("absolute"/"relative"/"").
    pub mode: String,
    pub sensitivity: Option<f32>,
    pub gamma: Option<f32>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: General::default(),
            absolute: Absolute::default(),
            relative: Relative::default(),
            pressure: Pressure::default(),
            switching: Switching::default(),
            profile: HashMap::new(),
        }
    }
}
impl Default for General {
    fn default() -> Self {
        Self { mode: "absolute".into() }
    }
}
impl Default for Absolute {
    fn default() -> Self {
        Self {
            area: "screen".into(),
            x: 0, y: 0, w: 0, h: 0,
            keep_aspect: true,
        }
    }
}
impl Default for Relative {
    fn default() -> Self {
        Self { sensitivity: 1.0 }
    }
}
impl Default for Pressure {
    fn default() -> Self {
        Self { gamma: 1.0 }
    }
}
impl Default for Switching {
    fn default() -> Self {
        Self {
            barrel_toggles_mode: true,
            hotkey: "ctrl+alt+g".into(),
        }
    }
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            process: String::new(),
            mode: String::new(),
            sensitivity: None,
            gamma: None,
        }
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

    /// gamma efektif: env menimpa file (kompatibel Fase 2).
    pub fn gamma(&self) -> f32 {
        std::env::var("GPAD_PRESSURE_GAMMA")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|&g: &f32| g > 0.0 && g <= 4.0)
            .unwrap_or(self.pressure.gamma)
            .clamp(0.25, 4.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_sane() {
        let c = Config::default();
        assert_eq!(c.general.mode, "absolute");
        assert_eq!(c.absolute.area, "screen");
        assert!(c.absolute.keep_aspect);
        assert!((c.relative.sensitivity - 1.0).abs() < 1e-6);
        assert!(c.switching.barrel_toggles_mode);
    }

    #[test]
    fn parses_example() {
        let toml_text = r#"
[general]
mode = "relative"
[absolute]
area = "custom"
x = 100
y = 100
w = 1600
h = 900
keep_aspect = false
[relative]
sensitivity = 1.5
[pressure]
gamma = 1.8
[switching]
barrel_toggles_mode = false
hotkey = ""
[profile.photoshop]
process = "photoshop.exe"
mode = "absolute"
sensitivity = 1.2
"#;
        let c: Config = toml::from_str(toml_text).unwrap();
        assert_eq!(c.general.mode, "relative");
        assert_eq!(c.absolute.w, 1600);
        assert!(!c.switching.barrel_toggles_mode);
        assert_eq!(c.profile["photoshop"].process, "photoshop.exe");
    }
}
