//! Radio Browser discovery. Search only returns streams this decoder supports.
use serde_json::Value;
use std::sync::OnceLock;

#[derive(Clone, Debug)]
pub struct Found {
    pub uuid: String,
    pub name: String,
    pub country: String,
    pub tags: String,
    pub codec: String,
    pub url: String,
}
fn clean(s: &str) -> String {
    s.replace(['\n', '\r', '|'], " ").trim().to_string()
}
pub fn parse(json: &str) -> Result<Vec<Found>, String> {
    let data: Value = serde_json::from_str(json).map_err(|_| "Invalid station response")?;
    let array = data.as_array().ok_or("Missing station results")?;
    Ok(array
        .iter()
        .filter_map(|v| {
            let codec = v["codec"].as_str()?.to_uppercase();
            if !matches!(codec.as_str(), "MP3" | "AAC" | "AAC+" | "OGG" | "VORBIS")
                || v["hls"].as_u64() == Some(1)
                || v["lastcheckok"].as_u64() == Some(0)
            {
                return None;
            }
            let url = v["url_resolved"].as_str()?;
            if !(url.starts_with("https://") || url.starts_with("http://"))
                || url.contains(['\r', '\n'])
            {
                return None;
            }
            Some(Found {
                uuid: v["stationuuid"].as_str()?.into(),
                name: clean(v["name"].as_str()?),
                country: clean(v["countrycode"].as_str().unwrap_or_default()),
                tags: clean(v["tags"].as_str().unwrap_or_default()),
                codec,
                url: url.into(),
            })
        })
        .take(30)
        .collect())
}
fn get(url: &str, query: Option<&str>) -> Result<String, String> {
    let mut request = ureq::get(url).header(
        "User-Agent",
        concat!("Ferrite-Wireless/", env!("CARGO_PKG_VERSION")),
    );
    if let Some(query) = query {
        request = request
            .query("name", query)
            .query("limit", "100")
            .query("hidebroken", "true")
            .query("order", "clickcount")
            .query("reverse", "true");
    }
    let mut response = request
        .config()
        .timeout_global(Some(std::time::Duration::from_secs(10)))
        .build()
        .call()
        .map_err(|e| format!("Station service: {e}"))?;
    response
        .body_mut()
        .with_config()
        .limit(2 * 1024 * 1024)
        .read_to_string()
        .map_err(|_| "Couldn't read station results".into())
}
fn mirrors() -> &'static Vec<String> {
    static HOSTS: OnceLock<Vec<String>> = OnceLock::new();
    HOSTS.get_or_init(|| {
        get("https://all.api.radio-browser.info/json/servers", None)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| v.as_array().cloned())
            .map(|array| {
                array
                    .iter()
                    .filter_map(|s| s["name"].as_str())
                    .filter(|name| {
                        name.ends_with(".api.radio-browser.info")
                            && name
                                .chars()
                                .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-')
                    })
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .filter(|hosts| !hosts.is_empty())
            .unwrap_or_else(|| {
                vec![
                    "de1.api.radio-browser.info".into(),
                    "nl1.api.radio-browser.info".into(),
                ]
            })
    })
}
pub fn search(query: &str) -> Result<Vec<Found>, String> {
    let hosts = mirrors();
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as usize;
    let mut error = "Station directory unavailable".to_string();
    for i in 0..hosts.len().min(2) {
        let url = format!(
            "https://{}/json/stations/search",
            hosts[(seed + i) % hosts.len()]
        );
        match get(&url, Some(query)).and_then(|s| parse(&s)) {
            Ok(found) => return Ok(found),
            Err(e) => error = e,
        }
    }
    Err(error)
}
pub fn register_click(uuid: String) {
    if uuid.is_empty() || !uuid.chars().all(|ch| ch.is_ascii_hexdigit() || ch == '-') {
        return;
    }
    std::thread::spawn(move || {
        let _ = get(&format!("https://{}/json/url/{uuid}", mirrors()[0]), None);
    });
}
pub fn free_frequency(band: &[crate::dial::Station]) -> Option<f32> {
    (0..=40)
        .map(|i| crate::dial::snap(87.6 + i as f32 * 0.5))
        .find(|freq| {
            band.iter()
                .all(|station| (station.freq - freq).abs() >= 0.4)
        })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn skips_hls_unsupported_codecs_and_unsafe_streams() {
        let json = r#"[{"name":"Jazz | FM","stationuuid":"abc","codec":"MP3","url_resolved":"https://example.com/live","hls":0},{"name":"Skip","stationuuid":"def","codec":"OPUS","url_resolved":"https://example.com/opus"}]"#;
        let results = parse(json).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Jazz   FM");
        assert!(free_frequency(&crate::dial::presets()).is_some());
        assert!(parse("{}").is_err());
    }
}
