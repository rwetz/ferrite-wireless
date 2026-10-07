//! The radio's settings, saved as plain `key = value` lines in
//! `<config>/ferrite/wireless.conf`. Your own stations go in as
//! `station = 94.1 | Name | genre | url` lines; any at all replace the
//! preset band.

use std::path::PathBuf;

use crate::dial::{self, Station};

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub scheme: String,
    /// `dark`, `light` or `system`.
    pub appearance: String,
    pub fps: u32,
    /// Where the dial was left, MHz.
    pub freq: f32,
    /// 0–1.
    pub volume: f32,
    /// Your stations; empty means the presets.
    pub stations: Vec<Station>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { scheme: "ferrite".into(), appearance: "dark".into(), fps: 240, freq: 91.3, volume: 0.6, stations: Vec::new() }
    }
}

impl Settings {
    pub fn parse(src: &str) -> Self {
        let mut s = Self::default();
        for line in src.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "scheme" if !v.is_empty() => s.scheme = v.into(),
                "appearance" if matches!(v, "dark" | "light" | "system") => s.appearance = v.into(),
                "fps" => s.fps = v.parse().map(|f: u32| f.clamp(12, 240)).unwrap_or(s.fps),
                "freq" => s.freq = v.parse().map(dial::snap).unwrap_or(s.freq),
                "volume" => s.volume = v.parse().map(|f: f32| f.clamp(0., 1.)).unwrap_or(s.volume),
                "station" => s.stations.extend(dial::parse_station(v)),
                _ => {}
            }
        }
        s
    }

    pub fn serialize(&self) -> String {
        let mut out = format!(
            "# Wireless settings.\n# Add your own stations as `station = 94.1 | Name | genre | url`;\n# any station lines replace the preset band.\n\
             scheme = {}\nappearance = {}\nfps = {}\nfreq = {:.1}\nvolume = {:.2}\n",
            self.scheme, self.appearance, self.fps, self.freq, self.volume,
        );
        for st in &self.stations {
            out.push_str(&format!("station = {}\n", dial::format_station(st)));
        }
        out
    }

    /// The band: your stations, or the presets.
    pub fn band(&self) -> Vec<Station> {
        if self.stations.is_empty() { dial::presets() } else { self.stations.clone() }
    }

    pub fn path() -> Option<PathBuf> {
        let base = if cfg!(windows) {
            std::env::var_os("APPDATA").map(PathBuf::from)
        } else if cfg!(target_os = "macos") {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
        } else {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        };
        base.map(|b| b.join("ferrite").join("wireless.conf"))
    }

    pub fn load() -> Self {
        Self::path().and_then(|p| std::fs::read_to_string(p).ok()).map(|s| Self::parse(&s)).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("no config directory")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, self.serialize()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let s = Settings {
            scheme: "bruise".into(),
            appearance: "light".into(),
            fps: 25,
            freq: 101.1,
            volume: 0.35,
            stations: vec![dial::parse_station("94.1 | KEXP | eclectic | https://kexp.streamguys1.com/kexp160.aac").unwrap()],
        };
        assert_eq!(Settings::parse(&s.serialize()), s);
    }

    #[test]
    fn presets_unless_you_add_stations() {
        assert_eq!(Settings::default().band(), dial::presets());
        let s = Settings::parse("station = 90.0 | Mine | http://example.com/s\nstation = junk\n");
        assert_eq!(s.band().len(), 1);
    }

    #[test]
    fn junk_falls_back_to_defaults() {
        let s = Settings::parse("appearance = purple\nvolume = 9\nfreq = 300\n");
        assert_eq!(s.appearance, "dark");
        assert_eq!(s.volume, 1.);
        assert_eq!(s.freq, dial::HIGH);
    }
}
