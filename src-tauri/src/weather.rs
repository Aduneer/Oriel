//! The weather widget: today's forecast from Open-Meteo (open-meteo.com, free and without an
//! account), and the place search in Settings.
//!
//! What's sent: the location's coordinates, rounded to two decimals (about a kilometre), in the
//! forecast request; and, when someone searches for a place, the words they typed. Temperatures
//! are always fetched in °C; the page converts to °F if that's what's chosen.

use crate::feed::Watch;
use crate::net;
use crate::settings::Settings;
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;

const EVERY: Duration = Duration::from_secs(30 * 60);
const MAX_REPLY: usize = 256 << 10;
const TIMEOUT: Duration = Duration::from_secs(15);
/// Hours ahead the page gets, from the current one.
const HOURS: usize = 36;
const MAX_QUERY: usize = 80;

#[derive(Clone, PartialEq)]
pub struct Config {
    lat: f64,
    lon: f64,
}

/// Where to get the weather for: the place chosen in Settings, if the widget is on a page.
pub fn config(s: &Settings) -> Option<Config> {
    let p = s.weather_place.as_ref().filter(|_| s.shows("weather"))?;
    Some(Config { lat: p.lat, lon: p.lon })
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Forecast {
    now: Now,
    hours: Vec<Hour>,
    days: Vec<Day>,
}

/// Times are as they are at the place itself ("2026-10-19T14:00"), not where this PC is.
#[derive(Serialize, Clone, Debug, PartialEq)]
struct Now {
    time: String,
    temp: f64,
    feels: Option<f64>,
    humidity: Option<f64>,
    /// km/h
    wind: Option<f64>,
    /// WMO weather code (0 clear, 3 overcast, 61 rain, 95 thunderstorm, ...)
    code: u8,
    day: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
struct Hour {
    time: String,
    temp: f64,
    code: u8,
    day: bool,
    /// Chance of rain or snow, %
    rain: Option<u8>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
struct Day {
    date: String,
    code: u8,
    hi: f64,
    lo: f64,
    rain: Option<u8>,
    /// "07:32"
    sunrise: Option<String>,
    sunset: Option<String>,
}

pub type WeatherWatch = Watch<Config, Forecast>;

pub fn start() -> WeatherWatch {
    Watch::start("weather", EVERY, Duration::from_secs(3600), fetch, |_, _| None)
}

fn fetch(c: &Config, _prev: Option<&Forecast>) -> (Option<Forecast>, Option<String>) {
    match net::get(&forecast_url(c), "application/json", MAX_REPLY, TIMEOUT) {
        Ok(body) => match parse(&String::from_utf8_lossy(&body)) {
            Some(f) => (Some(f), None),
            None => (None, Some("Unexpected reply".into())),
        },
        Err(e) => (None, Some(e.text())),
    }
}

fn forecast_url(c: &Config) -> String {
    format!(
        "https://api.open-meteo.com/v1/forecast?latitude={:.2}&longitude={:.2}\
         &current=temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,is_day,wind_speed_10m\
         &hourly=temperature_2m,weather_code,is_day,precipitation_probability\
         &daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max,sunrise,sunset\
         &timezone=auto&forecast_days=7",
        c.lat, c.lon
    )
}

fn column(v: &Value, key: &str) -> Vec<Value> {
    v.get(key).and_then(Value::as_array).cloned().unwrap_or_default()
}

fn time_of_day(iso: &str) -> Option<String> {
    iso.split_once('T').map(|(_, t)| t.chars().take(5).collect())
}

fn parse(json: &str) -> Option<Forecast> {
    let v: Value = serde_json::from_str(json).ok()?;
    let cur = v.get("current")?;
    let num = |x: Option<&Value>| x.and_then(Value::as_f64).filter(|n| n.is_finite());
    let code = |x: Option<&Value>| num(x).map(|c| c as u8).unwrap_or(0);
    let pct = |x: Option<&Value>| num(x).map(|p| p.clamp(0.0, 100.0) as u8);
    let now = Now {
        time: cur.get("time")?.as_str()?.to_string(),
        temp: num(cur.get("temperature_2m"))?,
        feels: num(cur.get("apparent_temperature")),
        humidity: num(cur.get("relative_humidity_2m")),
        wind: num(cur.get("wind_speed_10m")),
        code: code(cur.get("weather_code")),
        day: num(cur.get("is_day")).is_none_or(|d| d > 0.0),
    };

    let h = v.get("hourly")?;
    let (times, temps, codes, is_day, rain) = (column(h, "time"), column(h, "temperature_2m"), column(h, "weather_code"), column(h, "is_day"), column(h, "precipitation_probability"));
    // From the hour we're in. The times are all "YYYY-MM-DDTHH:MM", so text order is time order.
    let this_hour: String = now.time.chars().take(13).collect();
    let hours: Vec<Hour> = times
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            let time = t.as_str()?;
            (time.get(..13)? >= this_hour.as_str()).then_some(())?;
            Some(Hour {
                time: time.to_string(),
                temp: num(temps.get(i))?,
                code: code(codes.get(i)),
                day: num(is_day.get(i)).is_none_or(|d| d > 0.0),
                rain: pct(rain.get(i)),
            })
        })
        .take(HOURS)
        .collect();

    let d = v.get("daily")?;
    let (dates, dcodes, his, los, drain, rises, sets) = (
        column(d, "time"),
        column(d, "weather_code"),
        column(d, "temperature_2m_max"),
        column(d, "temperature_2m_min"),
        column(d, "precipitation_probability_max"),
        column(d, "sunrise"),
        column(d, "sunset"),
    );
    let days: Vec<Day> = dates
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            Some(Day {
                date: t.as_str()?.to_string(),
                code: code(dcodes.get(i)),
                hi: num(his.get(i))?,
                lo: num(los.get(i))?,
                rain: pct(drain.get(i)),
                sunrise: rises.get(i).and_then(Value::as_str).and_then(time_of_day),
                sunset: sets.get(i).and_then(Value::as_str).and_then(time_of_day),
            })
        })
        .take(7)
        .collect();
    (!hours.is_empty() && !days.is_empty()).then_some(Forecast { now, hours, days })
}

// --- place search --------------------------------------------------------------

/// A search result: what Settings shows, and what's kept if it's chosen.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Place {
    name: String,
    /// "Illinois, United States": to tell places of the same name apart.
    detail: String,
    lat: f64,
    lon: f64,
}

/// Looks a place up by name. Called only when someone searches.
pub fn search(query: &str) -> Result<Vec<Place>, String> {
    let query: String = query.chars().filter(|c| !c.is_control()).take(MAX_QUERY).collect();
    let query = query.trim();
    if query.chars().count() < 2 {
        return Ok(vec![]);
    }
    let url = format!("https://geocoding-api.open-meteo.com/v1/search?name={}&count=8&language=en&format=json", encode(query));
    let body = net::get(&url, "application/json", MAX_REPLY, TIMEOUT).map_err(|e| e.text())?;
    parse_places(&String::from_utf8_lossy(&body)).ok_or_else(|| "Unexpected reply".to_string())
}

fn parse_places(json: &str) -> Option<Vec<Place>> {
    let v: Value = serde_json::from_str(json).ok()?;
    // No matches: there's no "results" at all.
    let Some(results) = v.get("results").and_then(Value::as_array) else { return Some(vec![]) };
    let text = |r: &Value, k: &str| r.get(k).and_then(Value::as_str).map(|s| s.chars().filter(|c| !c.is_control()).take(60).collect::<String>()).filter(|s| !s.is_empty());
    Some(
        results
            .iter()
            .filter_map(|r| {
                let (lat, lon) = (r.get("latitude")?.as_f64()?, r.get("longitude")?.as_f64()?);
                ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)).then_some(())?;
                Some(Place {
                    name: text(r, "name")?,
                    detail: [text(r, "admin1"), text(r, "country")].into_iter().flatten().collect::<Vec<_>>().join(", "),
                    lat: (lat * 100.0).round() / 100.0,
                    lon: (lon * 100.0).round() / 100.0,
                })
            })
            .collect(),
    )
}

/// Percent-encodes everything but letters, digits and `-._~`.
fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{Page, Place as Chosen};
    use serde_json::json;

    /// A reply shaped like Open-Meteo's: 3 days of hours from midnight, 7 days, made-up numbers.
    fn reply(now: &str) -> String {
        let hours: Vec<String> = (0..72).map(|i| format!("2026-10-{:02}T{:02}:00", 19 + i / 24, i % 24)).collect();
        let n = hours.len();
        json!({
            "latitude": 52.52, "longitude": 13.42, "timezone": "Europe/Berlin", "utc_offset_seconds": 7200,
            "current": {"time": now, "interval": 900, "temperature_2m": 11.4, "apparent_temperature": 9.8,
                        "relative_humidity_2m": 71, "weather_code": 3, "is_day": 1, "wind_speed_10m": 14.2},
            "hourly": {
                "time": hours,
                "temperature_2m": (0..n).map(|i| 8.0 + (i % 24) as f64 / 3.0).collect::<Vec<_>>(),
                "weather_code": (0..n).map(|i| if i % 24 > 15 { 61 } else { 3 }).collect::<Vec<_>>(),
                "is_day": (0..n).map(|i| i32::from((7..18).contains(&(i % 24)))).collect::<Vec<_>>(),
                "precipitation_probability": (0..n).map(|i| if i % 5 == 0 { Value::Null } else { json!((i * 3) % 120) }).collect::<Vec<_>>(),
            },
            "daily": {
                "time": (19..26).map(|d| format!("2026-10-{d}")).collect::<Vec<_>>(),
                "weather_code": [3, 61, 61, 80, 2, 0, 1],
                "temperature_2m_max": [12.1, 13.0, 10.5, 9.9, 11.0, 14.2, 15.0],
                "temperature_2m_min": [6.3, 7.1, 5.0, 4.4, 3.9, 5.5, 6.0],
                "precipitation_probability_max": [20, 80, 90, 70, 10, 0, 5],
                "sunrise": (19..26).map(|d| format!("2026-10-{d}T07:3{}", d % 10)).collect::<Vec<_>>(),
                "sunset": (19..26).map(|d| format!("2026-10-{d}T17:1{}", d % 10)).collect::<Vec<_>>(),
            },
        })
        .to_string()
    }

    #[test]
    fn reads_the_forecast() {
        let f = parse(&reply("2026-10-19T14:15")).expect("didn't parse");
        assert_eq!(f.now.temp, 11.4);
        assert_eq!(f.now.feels, Some(9.8));
        assert_eq!(f.now.humidity, Some(71.0));
        assert_eq!(f.now.wind, Some(14.2));
        assert_eq!((f.now.code, f.now.day), (3, true));
        // Hours from 14:00 on, at most HOURS of them, and the chance of rain as a plain percentage.
        assert_eq!(f.hours.len(), HOURS);
        assert_eq!(f.hours[0].time, "2026-10-19T14:00");
        assert_eq!(f.hours[1].time, "2026-10-19T15:00");
        assert_eq!(f.hours[10].time, "2026-10-20T00:00");
        assert_eq!(f.hours[0].rain, Some(42)); // i = 14: 14 * 3
        assert_eq!(f.hours[1].rain, None); // i = 15: null
        assert!(f.hours.iter().all(|h| h.rain.is_none_or(|r| r <= 100)), "clamped to a percentage");
        assert!(f.hours[4].code == 61 && !f.hours[4].day);
        // Seven days, with the sun times cut to the time of day.
        assert_eq!(f.days.len(), 7);
        assert_eq!((f.days[0].hi, f.days[0].lo, f.days[0].code), (12.1, 6.3, 3));
        assert_eq!(f.days[1].rain, Some(80));
        assert_eq!(f.days[0].sunrise.as_deref(), Some("07:39"));
        assert_eq!(f.days[0].sunset.as_deref(), Some("17:19"));
    }

    #[test]
    fn a_reply_that_isnt_a_forecast_is_refused() {
        assert!(parse("").is_none());
        assert!(parse("<html>Bad gateway</html>").is_none());
        assert!(parse(r#"{"error": true, "reason": "Latitude must be in range of -90 to 90"}"#).is_none());
        assert!(parse(r#"{"current": {"time": "2026-10-19T14:15"}, "hourly": {}, "daily": {}}"#).is_none());
        // A missing temperature is no forecast at all, rather than 0 °C.
        let mut v: Value = serde_json::from_str(&reply("2026-10-19T14:15")).unwrap();
        v["current"].as_object_mut().unwrap().remove("temperature_2m");
        assert!(parse(&v.to_string()).is_none());
    }

    #[test]
    fn night_and_missing_fields_dont_break_it() {
        let mut v: Value = serde_json::from_str(&reply("2026-10-19T23:40")).unwrap();
        v["current"]["is_day"] = json!(0);
        v["current"].as_object_mut().unwrap().remove("apparent_temperature");
        let f = parse(&v.to_string()).unwrap();
        assert!(!f.now.day);
        assert_eq!(f.now.feels, None);
        assert_eq!(f.hours[0].time, "2026-10-19T23:00");
    }

    #[test]
    fn the_request_names_only_the_rounded_coordinates() {
        let url = forecast_url(&Config { lat: 52.52, lon: -13.41 });
        assert!(url.starts_with("https://api.open-meteo.com/v1/forecast?latitude=52.52&longitude=-13.41&"), "{url}");
        assert!(url.contains("timezone=auto"));
        assert!(!url.contains("temperature_unit"), "always °C; the page converts");
        let precise = forecast_url(&Config { lat: 52.123456, lon: 13.987654 });
        assert!(precise.contains("latitude=52.12&longitude=13.99&"), "{precise}");
    }

    fn settings(on: bool, place: Option<Chosen>) -> Settings {
        Settings {
            pages: vec![Page { name: "P".into(), layout: vec![if on { vec!["weather".to_string()] } else { vec![] }], ..Page::default() }],
            weather_place: place,
            ..Settings::default()
        }
    }

    #[test]
    fn nothing_is_fetched_without_the_widget_and_a_place() {
        let berlin = Chosen { name: "Berlin".into(), lat: 52.52, lon: 13.41 };
        assert!(config(&settings(false, Some(berlin.clone()))).is_none());
        assert!(config(&settings(true, None)).is_none());
        let c = config(&settings(true, Some(berlin))).unwrap();
        assert_eq!((c.lat, c.lon), (52.52, 13.41));
    }

    #[test]
    fn place_results() {
        let reply = json!({"results": [
            {"id": 1, "name": "Springfield", "latitude": 39.80172, "longitude": -89.64371, "country": "United States", "admin1": "Illinois"},
            {"id": 2, "name": "Springfield", "latitude": 42.10148, "longitude": -72.58981, "country": "United States", "admin1": "Massachusetts"},
            {"id": 3, "name": "Nowhere", "latitude": 123.0, "longitude": 10.0, "country": "Atlantis"},
            {"id": 4, "name": "Noland", "latitude": 5.0},
            {"id": 5, "name": "Smallton", "latitude": -12.5, "longitude": 130.25}
        ], "generationtime_ms": 0.7}).to_string();
        let places = parse_places(&reply).unwrap();
        assert_eq!(places.len(), 3, "out of range and incomplete results are dropped");
        assert_eq!(places[0], Place { name: "Springfield".into(), detail: "Illinois, United States".into(), lat: 39.8, lon: -89.64 });
        assert_eq!(places[1].detail, "Massachusetts, United States");
        assert_eq!(places[2].detail, "");
        // No matches is a reply with no "results".
        assert_eq!(parse_places(r#"{"generationtime_ms": 0.4}"#), Some(vec![]));
        assert_eq!(parse_places("nope"), None);
    }

    #[test]
    fn searches_are_encoded_and_short_ones_are_not_sent() {
        assert_eq!(encode("São Paulo"), "S%C3%A3o%20Paulo");
        assert_eq!(encode("a&b=c/d?e#f"), "a%26b%3Dc%2Fd%3Fe%23f");
        assert_eq!(encode("Saint-Denis_1.~"), "Saint-Denis_1.~");
        assert_eq!(search(""), Ok(vec![]));
        assert_eq!(search(" a "), Ok(vec![]));
    }
}
