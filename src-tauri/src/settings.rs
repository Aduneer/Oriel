//! User settings, stored as JSON in %APPDATA%\Oriel\settings.json.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// "dark" | "light" | "auto"
    pub theme: String,
    /// Color palette id, e.g. "oriel" or "nord" (the palettes live in ui/themes.css)
    pub palette: String,
    /// "auto" (a secondary screen if there is one) or a monitor number from `monitors`
    pub monitor: String,
    pub fullscreen: bool,
    pub interval_ms: u64,
    /// Seconds of history on the graphs
    pub graph_seconds: u32,
    /// Dashboards to switch between, each with its own widgets. Most people only need the one.
    pub pages: Vec<Page>,
    /// Move on to the next page every this many seconds, or 0 to stay on one
    pub rotate_seconds: u32,
    pub limits: Limits,
    /// Nudge the layout a pixel or two now and then so static UI can't burn into OLED panels
    pub burn_in_shift: bool,
    /// Let the dashboard draw with the GPU. Off by default: software drawing uses ~130 MB less
    /// memory for a little more CPU, and a page that redraws a few graphs a second doesn't need it.
    pub gpu: bool,
    pub nvml: bool,
    pub lhm_url: String,
    /// What the weather widget shows the forecast for, chosen with the place search in Settings.
    /// Its coordinates are what Open-Meteo is asked about. None until someone picks a place.
    pub weather_place: Option<Place>,
    /// "auto" (°F in the United States and a few other countries, °C elsewhere), "c" or "f"
    pub weather_units: String,
    /// The calendar widget's .ics links. Secrets, in the sense that anyone who has one can read the
    /// calendar: they're only used for fetching, and never shown anywhere but in Settings.
    pub calendars: Vec<CalendarFeed>,
    /// How many days ahead the calendar widget looks
    pub calendar_days: u32,
}

/// A place, for the weather.
#[derive(Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(default)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

/// One calendar for the calendar widget.
#[derive(Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(default)]
pub struct CalendarFeed {
    /// What the widget calls it; empty for "Calendar 1" and so on
    pub name: String,
    /// A theme color's name (see `CALENDAR_COLORS`), or empty for the widget's pick
    pub color: String,
    /// https:// (webcal:// is saved as that); empty if what was typed isn't a link that can be fetched
    pub url: String,
}

pub const MAX_CALENDARS: usize = 5;
pub const MAX_CALENDAR_NAME: usize = 40;
pub const MAX_PLACE_NAME: usize = 60;
/// Colors a calendar can have: tokens from themes.css, so they suit every theme.
pub const CALENDAR_COLORS: &[&str] = &["cpu", "gpu", "mem", "s1", "s2"];

/// One dashboard: the name on its tab, and rows of widget ids, top to bottom.
#[derive(Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(default)]
pub struct Page {
    pub name: String,
    pub layout: Vec<Vec<String>>,
}

/// One per number key, 1 to 9.
pub const MAX_PAGES: usize = 9;
/// In characters, so a name fits on its tab.
pub const MAX_PAGE_NAME: usize = 32;

/// [warn, critical] thresholds
#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct Limits {
    pub cpu_temp: [f64; 2],
    pub gpu_temp: [f64; 2],
    pub mem: [f64; 2],
    pub drive: [f64; 2],
}

impl Default for Limits {
    fn default() -> Self {
        Limits { cpu_temp: [90.0, 97.0], gpu_temp: [83.0, 90.0], mem: [85.0, 95.0], drive: [90.0, 95.0] }
    }
}

impl Default for Settings {
    fn default() -> Self {
        let row = |ids: &[&str]| ids.iter().map(|s| s.to_string()).collect();
        Settings {
            theme: "dark".into(),
            palette: "oriel".into(),
            monitor: "auto".into(),
            fullscreen: true,
            interval_ms: 1000,
            graph_seconds: 300,
            pages: vec![Page {
                name: "Dashboard".into(),
                layout: vec![row(&["cpu", "gpu", "memory"]), row(&["network", "disk", "processes"])],
            }],
            rotate_seconds: 0,
            limits: Limits::default(),
            burn_in_shift: true,
            gpu: false,
            nvml: true,
            lhm_url: "http://127.0.0.1:8085/data.json".into(),
            weather_place: None,
            weather_units: "auto".into(),
            calendars: vec![],
            calendar_days: 7,
        }
    }
}

/// A name for a list or a tab: no control characters, no edge spaces, at most `max` characters.
fn tidy(name: &str, max: usize) -> String {
    name.chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().to_string()
}

pub fn path() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("Oriel").join("settings.json")
}

impl Settings {
    pub fn load() -> Self {
        std::fs::read_to_string(path()).ok().and_then(|s| Settings::parse(&s)).unwrap_or_default()
    }

    /// Read a settings file, including one from before pages.
    fn parse(text: &str) -> Option<Self> {
        // Notepad and PowerShell can save with a byte-order mark, which serde_json rejects.
        let mut v: serde_json::Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
        // Before pages there was a single `layout`: it becomes the first page.
        if let Some(obj) = v.as_object_mut() {
            if let Some(layout) = obj.remove("layout") {
                obj.entry("pages").or_insert_with(|| serde_json::json!([{ "name": "Dashboard", "layout": layout }]));
            }
        }
        serde_json::from_value(v).ok()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let p = path();
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Write then rename, so a crash mid-write can't leave a corrupt file.
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, p)
    }

    /// Whether a widget is on any page, on screen or not.
    pub fn shows(&self, widget: &str) -> bool {
        self.pages.iter().flat_map(|p| p.layout.iter().flatten()).any(|id| id == widget)
    }

    pub fn clamp(mut self) -> Self {
        self.interval_ms = self.interval_ms.clamp(250, 10_000);
        self.graph_seconds = self.graph_seconds.clamp(30, 600);
        if self.rotate_seconds != 0 {
            self.rotate_seconds = self.rotate_seconds.clamp(5, 3600);
        }
        self.pages.truncate(MAX_PAGES);
        if self.pages.is_empty() {
            self.pages = Settings::default().pages;
        }
        for (i, page) in self.pages.iter_mut().enumerate() {
            let name: String = page.name.trim().chars().filter(|c| !c.is_control()).take(MAX_PAGE_NAME).collect();
            page.name = match name.trim_end() {
                "" => format!("Page {}", i + 1),
                name => name.to_string(),
            };
        }
        // It ends up in an HTML attribute, so keep it to a plain id.
        let id_ok = |s: &str| !s.is_empty() && s.len() <= 32 && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !id_ok(&self.palette) {
            self.palette = "oriel".into();
        }
        // The LHM reader sends this address in a plain HTTP request; no other schemes, spaces or line breaks.
        let url = &self.lhm_url;
        if !url.is_empty() && !(url.starts_with("http://") && url.len() <= 256 && !url.chars().any(|c| c.is_whitespace() || c.is_control())) {
            self.lhm_url = Settings::default().lhm_url;
        }

        // Weather: a place Open-Meteo could be asked about, to the hundredth of a degree (about a kilometre).
        self.weather_place = self.weather_place.take().and_then(|p| {
            let name = tidy(&p.name, MAX_PLACE_NAME);
            let ok = !name.is_empty() && (-90.0..=90.0).contains(&p.lat) && (-180.0..=180.0).contains(&p.lon);
            ok.then(|| Place { name, lat: (p.lat * 100.0).round() / 100.0, lon: (p.lon * 100.0).round() / 100.0 })
        });
        if !["auto", "c", "f"].contains(&self.weather_units.as_str()) {
            self.weather_units = "auto".into();
        }

        // Calendars: a link is fetched, so it has to be https:// (webcal:// is how calendar apps write
        // that). A row whose link isn't one is kept, empty, so it doesn't vanish while it's being typed.
        self.calendars.truncate(MAX_CALENDARS);
        for c in &mut self.calendars {
            c.name = tidy(&c.name, MAX_CALENDAR_NAME);
            if !CALENDAR_COLORS.contains(&c.color.as_str()) {
                c.color.clear();
            }
            c.url = crate::net::https_url(&c.url).unwrap_or_default();
        }
        self.calendar_days = self.calendar_days.clamp(1, 14);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_layout_becomes_one_page() {
        // A file from before pages, saved by Notepad with a byte-order mark.
        let old = "\u{feff}{\"theme\": \"light\", \"layout\": [[\"cpu\", \"media\"], [\"processes\"]]}";
        let s = Settings::parse(old).expect("the old file didn't load").clamp();
        assert_eq!(s.theme, "light");
        assert_eq!(s.pages.len(), 1);
        assert_eq!(s.pages[0].name, "Dashboard");
        assert_eq!(s.pages[0].layout, [vec!["cpu", "media"], vec!["processes"]]);
        assert!(s.shows("media"));
        assert_eq!(s.rotate_seconds, 0);

        // Saved again, it has pages and no layout, and reads back the same.
        let saved = serde_json::to_value(&s).unwrap();
        assert!(saved.get("layout").is_none());
        assert!(Settings::parse(&saved.to_string()).unwrap() == s);
    }

    #[test]
    fn pages_win_over_a_leftover_layout() {
        let both = r#"{"pages": [{"name": "Media", "layout": [["media"]]}], "layout": [["cpu"]]}"#;
        let s = Settings::parse(both).unwrap();
        assert_eq!(s.pages.len(), 1);
        assert_eq!(s.pages[0].name, "Media");
        assert!(!s.shows("cpu"));
    }

    #[test]
    fn shows_looks_at_every_page() {
        let s = Settings::parse(r#"{"pages": [{"name": "Resources", "layout": [["cpu"]]}, {"name": "Media", "layout": [["media"]]}]}"#).unwrap();
        assert!(s.shows("cpu") && s.shows("media"));
        assert!(!s.shows("processes"));
    }

    #[test]
    fn a_file_from_before_weather_and_calendar_still_loads() {
        let s = Settings::parse(r#"{"theme": "light", "pages": [{"name": "Main", "layout": [["cpu"]]}]}"#).unwrap().clamp();
        assert!(s.weather_place.is_none() && s.calendars.is_empty());
        assert_eq!((s.weather_units.as_str(), s.calendar_days), ("auto", 7));
    }

    #[test]
    fn weather_place_and_units_are_clamped() {
        let with = |name: &str, lat: f64, lon: f64| Settings { weather_place: Some(Place { name: name.into(), lat, lon }), ..Settings::default() }.clamp().weather_place;
        let p = with("  Berlin\n", 52.523456, 13.404954).unwrap();
        assert_eq!((p.name.as_str(), p.lat, p.lon), ("Berlin", 52.52, 13.4));
        assert!(with("Nowhere", 91.0, 10.0).is_none());
        assert!(with("Nowhere", 10.0, -180.5).is_none());
        assert!(with("Nowhere", f64::NAN, 10.0).is_none());
        assert!(with("  ", 10.0, 10.0).is_none());
        assert_eq!(with(&"x".repeat(100), 1.0, 1.0).unwrap().name.chars().count(), MAX_PLACE_NAME);
        let units = |u: &str| Settings { weather_units: u.into(), ..Settings::default() }.clamp().weather_units;
        assert_eq!((units("f").as_str(), units("c").as_str(), units("kelvin").as_str()), ("f", "c", "auto"));
    }

    #[test]
    fn calendar_links_are_checked_and_tidied() {
        let feed = |name: &str, color: &str, url: &str| CalendarFeed { name: name.into(), color: color.into(), url: url.into() };
        let s = Settings {
            calendars: vec![
                feed("  Home\t", "s2", "  webcal://calendar.example.test/home.ics "),
                feed(&"n".repeat(80), "magenta", "https://calendar.example.test/work.ics?key=abc"),
                feed("Plain http", "", "http://calendar.example.test/x.ics"),
                feed("Not a link", "cpu", "calendar.example.test/x.ics"),
                feed("Has a space", "", "https://calendar.example.test/a b.ics"),
                feed("Sixth", "", "https://calendar.example.test/sixth.ics"),
                feed("Seventh", "", "https://calendar.example.test/seventh.ics"),
            ],
            calendar_days: 90,
            ..Settings::default()
        }
        .clamp();
        assert_eq!(s.calendars.len(), MAX_CALENDARS);
        assert_eq!((s.calendars[0].name.as_str(), s.calendars[0].color.as_str()), ("Home", "s2"));
        assert_eq!(s.calendars[0].url, "https://calendar.example.test/home.ics");
        assert_eq!(s.calendars[1].name.chars().count(), MAX_CALENDAR_NAME);
        assert_eq!(s.calendars[1].color, "", "only theme colors");
        assert_eq!(s.calendars[1].url, "https://calendar.example.test/work.ics?key=abc");
        // What can't be fetched is emptied, but the row stays.
        assert_eq!(s.calendars[2].url, "");
        assert_eq!(s.calendars[3].url, "");
        assert_eq!(s.calendars[4].url, "");
        assert_eq!(s.calendar_days, 14);
        assert_eq!(Settings { calendar_days: 0, ..Settings::default() }.clamp().calendar_days, 1);

        // Saved and read back, they're the same (the link included, for the settings panel).
        let again = Settings::parse(&serde_json::to_string(&s).unwrap()).unwrap().clamp();
        assert!(again == s);
    }

    #[test]
    fn pages_are_clamped() {
        let mut s = Settings {
            pages: (0..12).map(|i| Page { name: format!("P{i}"), layout: vec![] }).collect(),
            ..Settings::default()
        };
        s.pages[1].name = "  \t ".into();
        s.pages[2].name = "x".repeat(40);
        s.pages[3].name = "  Media\n ".into();
        s.rotate_seconds = 1;
        let s = s.clamp();
        assert_eq!(s.pages.len(), MAX_PAGES);
        assert_eq!(s.pages[1].name, "Page 2");
        assert_eq!(s.pages[2].name.chars().count(), MAX_PAGE_NAME);
        assert_eq!(s.pages[3].name, "Media");
        assert_eq!(s.rotate_seconds, 5);

        let none = Settings { pages: vec![], ..Settings::default() }.clamp();
        assert!(none.pages == Settings::default().pages);
    }
}
