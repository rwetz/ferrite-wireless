//! The dial: stations at frequencies on an FM-style band, and what the radio
//! hears at any point on it. Between stations there's only static; near one,
//! the station fades in through it. All pure, so it's all tested.

/// The band, in MHz, and the dial's resolution.
pub const LOW: f32 = 87.5;
pub const HIGH: f32 = 108.0;
pub const STEP: f32 = 0.1;

/// Within this of a station, reception is perfect.
const CLEAR: f32 = 0.1;
/// Beyond `CLEAR + FADE`, the station is gone. With presets every
/// [`SPACING`] there's still a strip of static between neighbours.
const FADE: f32 = 0.2;
/// How far apart the presets sit.
pub const SPACING: f32 = 0.5;
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

/// SomaFM's year-round channels (listener-supported, ad-free), in dial
/// order: ambient at the bottom of the band, through electronica, lounge
/// and soul, to rock, pop and the odd ones at the top. Holiday channels
/// and the occasional live/specials streams are left off: they're silent
/// most of the year. `(name, genre, stream id)`.
const SOMA: [(&str, &str, &str); 37] = [
    ("Groove Salad", "ambient downtempo", "groovesalad"),
    ("Groove Salad Classic", "early-2000s chill", "gsclassic"),
    ("Drone Zone", "atmospheric ambient", "dronezone"),
    ("Deep Space One", "deep ambient, space music", "deepspaceone"),
    ("Space Station Soma", "spaced-out ambient, mid-tempo electronica", "spacestation"),
    ("Mission Control", "ambient with NASA radio", "missioncontrol"),
    ("Synphaera", "modern space and ambient electronic", "synphaera"),
    ("The Dark Zone", "dark ambient", "darkzone"),
    ("n5MD Radio", "ambient, post-rock, IDM", "n5md"),
    ("Lush", "mellow vocals, electronica", "lush"),
    ("Fluid", "instrumental hip-hop, lo-fi beats", "fluid"),
    ("Beat Blender", "deep house, downtempo", "beatblender"),
    ("Cliqhop IDM", "intelligent dance music", "cliqhop"),
    ("Dub Step Beyond", "dubstep, deep bass", "dubstep"),
    ("The Trip", "progressive house, trance", "thetrip"),
    ("Digitalis", "analog and digital rock", "digitalis"),
    ("Vaporwaves", "vaporwave", "vaporwaves"),
    ("PopTron", "electropop, indie dance", "poptron"),
    ("DEF CON Radio", "music for hacking", "defcon"),
    ("Suburbs of Goa", "desi-influenced Asian world beats", "suburbsofgoa"),
    ("Secret Agent", "lounge, spy jazz", "secretagent"),
    ("Illinois Street Lounge", "classic bachelor-pad exotica", "illstreet"),
    ("Sonic Universe", "transcending jazz", "sonicuniverse"),
    ("Seven Inch Soul", "vintage soul 45s", "7soul"),
    ("Heavyweight Reggae", "reggae, ska, rocksteady", "reggae"),
    ("Boot Liquor", "Americana roots", "bootliquor"),
    ("Folk Forward", "indie folk, alt-folk", "folkfwd"),
    ("ThistleRadio", "Celtic roots", "thistle"),
    ("Left Coast 70s", "mellow 70s album rock", "seventies"),
    ("Underground 80s", "UK synthpop, new wave", "u80s"),
    ("Indie Pop Rocks!", "indie pop", "indiepop"),
    ("BAGeL Radio", "alternative rock", "bagel"),
    ("Covers", "songs you know, by artists you don't", "covers"),
    ("Metal Detector", "heavy metal", "metal"),
    ("Black Rock FM", "Burning Man's radio", "brfm"),
    ("SF 10-33", "ambient with San Francisco radio", "sf1033"),
    ("SF Police Scanner", "the city's scanner, over ambient", "scanner"),
];

/// Where the first preset sits.
const FIRST: f32 = 88.1;

/// The preset band: every year-round SomaFM channel, [`SPACING`] apart.
pub fn presets() -> Vec<Station> {
    SOMA.iter().enumerate().map(|(i, (name, genre, id))| soma(snap(FIRST + i as f32 * SPACING), name, genre, id)).collect()
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

/// The narrowest dial face that still gives every preset its own mark
/// (two columns per [`SPACING`], and one to spare).
pub const MIN_SCALE_COLS: usize = ((HIGH - LOW) / SPACING) as usize * 2 + 3;

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
        let t = tune(88.6, &st);
        assert_eq!(t.station, Some(1));
        assert_eq!(t.signal, 1.);
        assert_eq!(t.locked(), Some(1));
        assert_eq!(t.hiss(), 0.);
    }

    #[test]
    fn signal_fades_through_static() {
        let st = presets();
        let near = tune(88.75, &st).signal;
        let far = tune(88.82, &st).signal;
        assert!(near < 1. && near > far, "{near} {far}");
        // Past the last preset the band is empty.
        assert_eq!(tune(107.5, &st), Tuning { station: None, signal: 0. }, "off the presets there's only static");
        // Halfway between two presets the signal is too weak to lock.
        assert_eq!(tune(88.85, &st).locked(), None, "between stations there's static");
    }

    #[test]
    fn weak_signal_does_not_lock() {
        let st = presets();
        // Midway between two presets: still hears one, too faint to lock.
        let t = tune(88.85, &st);
        assert!(t.station.is_some() && t.signal < LOCK);
        assert_eq!(t.locked(), None);
    }

    #[test]
    fn seeks_and_wraps() {
        let st = presets();
        let last = st.len() - 1;
        assert_eq!(seek(88.0, &st, true), Some(0));
        assert_eq!(seek(88.1, &st, true), Some(1), "from on a station, seek moves off it");
        assert_eq!(seek(107.0, &st, true), Some(0), "wraps to the bottom");
        assert_eq!(seek(88.1, &st, false), Some(last), "wraps to the top");
        assert_eq!(seek(89.3, &st, false), Some(2));
        assert_eq!(seek(95.0, &[], true), None);
    }

    #[test]
    fn every_soma_channel_has_its_own_spot() {
        let st = presets();
        assert_eq!(st.len(), SOMA.len());
        assert!(st.iter().all(|s| (LOW..=HIGH).contains(&s.freq)));
        let ids: std::collections::HashSet<_> = SOMA.iter().map(|c| c.2).collect();
        assert_eq!(ids.len(), SOMA.len(), "no channel twice");
        for w in st.windows(2) {
            assert!((w[1].freq - w[0].freq - SPACING).abs() < 0.01, "{} → {}", w[0].freq, w[1].freq);
        }
        // Each one comes in clear on its own frequency.
        for (i, s) in st.iter().enumerate() {
            assert_eq!(tune(s.freq, &st).locked(), Some(i), "{}", s.name);
        }
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
        let [top, needle] = scale(88.1, &st, MIN_SCALE_COLS);
        assert_eq!(top.chars().count(), MIN_SCALE_COLS);
        assert_eq!(top.matches('|').count(), st.len());
        let at = |s: &str, c: char| s.chars().position(|x| x == c);
        assert_eq!(at(&needle, '^'), top.chars().position(|c| c == '|'), "the needle sits on the first station");
    }
}
