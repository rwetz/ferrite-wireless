//! The dial: stations at frequencies on an FM-style band, and what the radio
//! hears at any point on it. Between stations there's only static; near one,
//! the station fades in through it. All pure, so it's all tested.

/// The band, in MHz, and the dial's resolution.
pub const LOW: f32 = 87.5;
pub const HIGH: f32 = 108.0;
pub const STEP: f32 = 0.1;

/// Within this of a station, reception is perfect.
const CLEAR: f32 = 0.15;
/// Beyond `CLEAR + FADE`, the station is gone.
const FADE: f32 = 0.6;
/// Below this signal the radio doesn't lock on (no stream is opened).
pub const LOCK: f32 = 0.3;

#[derive(Clone, Debug, PartialEq)]
pub struct Station {
    pub freq: f32,
    pub name: String,
    /// What it plays, for the marquee.
    pub genre: String,
    pub url: String,
}

fn soma(freq: f32, name: &str, genre: &str, id: &str) -> Station {
    Station { freq, name: name.into(), genre: genre.into(), url: format!("https://ice1.somafm.com/{id}-128-mp3") }
}

/// The preset band: listener-supported, ad-free SomaFM channels.
pub fn presets() -> Vec<Station> {
    vec![
        soma(88.9, "Groove Salad", "ambient downtempo", "groovesalad"),
        soma(91.3, "Fluid", "instrumental hip-hop, lo-fi beats", "fluid"),
        soma(93.7, "Drone Zone", "atmospheric ambient", "dronezone"),
        soma(96.1, "Lush", "mellow vocals, electronica", "lush"),
        soma(98.5, "Deep Space One", "deep ambient, space music", "deepspaceone"),
        soma(101.1, "Secret Agent", "lounge, spy jazz", "secretagent"),
        soma(103.3, "Vaporwaves", "vaporwave", "vaporwaves"),
        soma(105.7, "DEF CON Radio", "music for hacking", "defcon"),
    ]
}

/// A user station from the settings file: `94.1 | Name | genre | url`
/// (the genre may be left out: `94.1 | Name | url`).
pub fn parse_station(line: &str) -> Option<Station> {
    let parts: Vec<&str> = line.split('|').map(str::trim).collect();
    let (freq, name, genre, url) = match parts.as_slice() {
        [f, n, u] => (f, n, "", u),
        [f, n, g, u] => (f, n, *g, u),
        _ => return None,
    };
    let freq: f32 = freq.parse().ok()?;
    let url_ok = url.starts_with("http://") || url.starts_with("https://");
    ((LOW..=HIGH).contains(&freq) && !name.is_empty() && url_ok).then(|| Station { freq: snap(freq), name: name.to_string(), genre: genre.to_string(), url: url.to_string() })
}

pub fn format_station(s: &Station) -> String {
    format!("{:.1} | {} | {} | {}", s.freq, s.name, s.genre, s.url)
}

/// Round to the dial's 0.1 MHz and keep it on the band.
pub fn snap(freq: f32) -> f32 {
    ((freq.clamp(LOW, HIGH) / STEP).round() * STEP * 10.).round() / 10.
}

/// What the radio hears at `freq`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuning {
    /// The nearest station, if there's any signal from it at all.
    pub station: Option<usize>,
    /// 0 (only static) to 1 (clear).
    pub signal: f32,
}

impl Tuning {
    /// The station the radio locks onto: close enough to be worth a stream.
    pub fn locked(&self) -> Option<usize> {
        self.station.filter(|_| self.signal >= LOCK)
    }

    /// How much static is mixed in, 0–1.
    pub fn hiss(&self) -> f32 {
        1. - self.signal
    }
}

pub fn tune(freq: f32, stations: &[Station]) -> Tuning {
    let nearest = stations.iter().enumerate().map(|(i, s)| (i, (s.freq - freq).abs())).min_by(|a, b| a.1.total_cmp(&b.1));
    match nearest {
        Some((i, d)) => {
            let signal = (1. - (d - CLEAR) / FADE).clamp(0., 1.);
            Tuning { station: (signal > 0.).then_some(i), signal }
        }
        None => Tuning { station: None, signal: 0. },
    }
}

/// The next station up (or down) the dial from `freq`, wrapping around.
pub fn seek(freq: f32, stations: &[Station], up: bool) -> Option<usize> {
    let mut order: Vec<usize> = (0..stations.len()).collect();
    order.sort_by(|a, b| stations[*a].freq.total_cmp(&stations[*b].freq));
    let eps = STEP / 2.;
    if up {
        order.iter().copied().find(|&i| stations[i].freq > freq + eps).or(order.first().copied())
    } else {
        order.iter().rev().copied().find(|&i| stations[i].freq < freq - eps).or(order.last().copied())
    }
}

/// The dial face as text, `cols` wide, in two rows: ticks every MHz with the
/// stations marked `|`, and the needle `^` under them.
pub fn scale(freq: f32, stations: &[Station], cols: usize) -> [String; 2] {
    let col = |f: f32| (((f - LOW) / (HIGH - LOW)) * (cols - 1) as f32).round() as usize;
    let mut top: Vec<char> = vec!['.'; cols];
    let mut mhz = LOW.ceil() as i32;
    while (mhz as f32) <= HIGH {
        top[col(mhz as f32)] = if mhz % 2 == 0 { ':' } else { '.' };
        mhz += 1;
    }
    for s in stations {
        top[col(s.freq)] = '|';
    }
    let mut needle: Vec<char> = vec![' '; cols];
    needle[col(freq.clamp(LOW, HIGH))] = '^';
    [top.into_iter().collect(), needle.into_iter().collect()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_a_station_is_clear() {
        let st = presets();
        let t = tune(91.3, &st);
        assert_eq!(t.station, Some(1));
        assert_eq!(t.signal, 1.);
        assert_eq!(t.locked(), Some(1));
        assert_eq!(t.hiss(), 0.);
    }

    #[test]
    fn signal_fades_through_static() {
        let st = presets();
        let near = tune(91.6, &st).signal;
        let far = tune(92.0, &st).signal;
        assert!(near < 1. && near > far, "{near} {far}");
        assert_eq!(tune(92.5, &st), Tuning { station: None, signal: 0. }, "between stations there's only static");
        assert_eq!(tune(92.5, &st).locked(), None);
    }

    #[test]
    fn weak_signal_does_not_lock() {
        let st = presets();
        let t = tune(91.3 + 0.62, &st);
        assert!(t.station.is_some() && t.signal < LOCK);
        assert_eq!(t.locked(), None);
    }

    #[test]
    fn seeks_and_wraps() {
        let st = presets();
        assert_eq!(seek(88.0, &st, true), Some(0));
        assert_eq!(seek(88.9, &st, true), Some(1), "from on a station, seek moves off it");
        assert_eq!(seek(107.0, &st, true), Some(0), "wraps to the bottom");
        assert_eq!(seek(88.9, &st, false), Some(7), "wraps to the top");
        assert_eq!(seek(95.0, &st, false), Some(2));
        assert_eq!(seek(95.0, &[], true), None);
    }

    #[test]
    fn snaps_to_the_dial() {
        assert_eq!(snap(94.149), 94.1);
        assert_eq!(snap(94.15), 94.2);
        assert_eq!(snap(50.), LOW);
        assert_eq!(snap(200.), HIGH);
    }

    #[test]
    fn user_stations_round_trip() {
        let s = parse_station("94.1 | KEXP | eclectic | https://kexp.streamguys1.com/kexp160.aac").unwrap();
        assert_eq!(s.name, "KEXP");
        assert_eq!(parse_station(&format_station(&s)), Some(s));
        assert!(parse_station("94.1 | No genre | http://example.com/stream").is_some());
        assert_eq!(parse_station("150 | Off band | http://x"), None);
        assert_eq!(parse_station("94.1 | Not a url | ftp://x"), None);
        assert_eq!(parse_station("94.1"), None);
    }

    #[test]
    fn the_scale_marks_stations_and_needle() {
        let st = presets();
        let [top, needle] = scale(88.9, &st, 64);
        assert_eq!(top.chars().count(), 64);
        assert_eq!(top.matches('|').count(), st.len());
        let at = |s: &str, c: char| s.chars().position(|x| x == c);
        assert_eq!(at(&needle, '^'), top.chars().position(|c| c == '|'), "the needle sits on the first station");
    }
}
