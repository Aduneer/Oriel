//! The calendar widget: fetches the .ics links in Settings and works out what's coming up.
//!
//! The links are secrets (many calendars' "secret address" is the only thing protecting them),
//! so they live only in the settings file and in this module's requests: they're never logged,
//! never sent to the page, and never put in an error message. The page only gets the events,
//! and whether each calendar could be fetched.

use crate::feed::{now, Watch};
use crate::ics::{self, Event};
use crate::net::{self, Fail};
use crate::settings::Settings;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;

/// How often the links are fetched.
const EVERY: Duration = Duration::from_secs(15 * 60);
/// How often "what's coming up" is worked out again from what's been fetched.
const REBUILD_EVERY: Duration = Duration::from_secs(10 * 60);
const MAX_FILE: usize = 5 << 20;
const TIMEOUT: Duration = Duration::from_secs(30);
/// The most events sent to the page.
const MAX_SHOWN: usize = 300;

/// What to fetch and show. Deliberately has no `Debug`, so a link can't end up in a log by
/// accident. Names and colors aren't here: changing one isn't a reason to fetch again, and the
/// page has them in the settings.
#[derive(Clone, PartialEq)]
pub struct Config {
    /// https:// links only: see `net::https_url`. One per calendar, in the settings' order.
    links: Vec<String>,
    days: u32,
}

/// The widget is off, or has no usable link: nothing is fetched.
pub fn config(s: &Settings) -> Option<Config> {
    if !s.shows("calendar") {
        return None;
    }
    let links: Vec<String> = s.calendars.iter().filter_map(|c| net::https_url(&c.url)).collect();
    (!links.is_empty()).then_some(Config { links, days: s.calendar_days })
}

#[derive(Clone)]
struct FeedData {
    events: Arc<Vec<Event>>,
    error: Option<String>,
}

pub struct Data {
    feeds: Vec<FeedData>,
    pub view: View,
}

/// What the page gets.
#[derive(Serialize, Clone)]
pub struct View {
    /// One per calendar that has a link, in the order they're in the settings.
    feeds: Vec<FeedView>,
    events: Vec<Item>,
}

#[derive(Serialize, Clone)]
struct FeedView {
    /// Why this calendar couldn't be fetched, if it couldn't: what's shown for it is older.
    error: Option<String>,
}

#[derive(Serialize, Clone)]
struct Item {
    /// Index into `View::feeds`.
    cal: usize,
    title: String,
    location: String,
    all_day: bool,
    /// Unix seconds, or for an all-day event the day (days since 1970-01-01). `end` is exclusive.
    start: i64,
    end: i64,
}

pub type CalendarWatch = Watch<Config, Data>;

pub fn start() -> CalendarWatch {
    Watch::start("calendar", EVERY, REBUILD_EVERY, fetch, |cfg, d| Some(Data { feeds: d.feeds.clone(), view: view(cfg, &d.feeds) }))
}

fn fetch(cfg: &Config, prev: Option<&Data>) -> (Option<Data>, Option<String>) {
    let got = cfg.links.iter().map(|url| net::get(url, "text/calendar, */*;q=0.5", MAX_FILE, TIMEOUT)).collect();
    let (data, error) = assemble(cfg, got, prev);
    (Some(data), error)
}

/// Puts what was fetched (one result per calendar) together. A calendar that failed keeps
/// what it had, and says why.
fn assemble(cfg: &Config, got: Vec<Result<Vec<u8>, Fail>>, prev: Option<&Data>) -> (Data, Option<String>) {
    let mut error = None;
    let feeds: Vec<FeedData> = got
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            let fresh = r.and_then(|bytes| {
                let text = String::from_utf8_lossy(&bytes);
                // A sign-in page or an error page, rather than a calendar.
                if !text.to_ascii_uppercase().contains("BEGIN:VCALENDAR") {
                    return Err(Fail::Other);
                }
                Ok(ics::parse(&text))
            });
            match fresh {
                Ok(events) => FeedData { events: Arc::new(events), error: None },
                Err(f) => {
                    let text = if f == Fail::Other { "Not a calendar".to_string() } else { f.text() };
                    error.get_or_insert_with(|| text.clone());
                    let events = prev.and_then(|p| p.feeds.get(i)).map(|p| p.events.clone()).unwrap_or_default();
                    FeedData { events, error: Some(text) }
                }
            }
        })
        .collect();
    let view = view(cfg, &feeds);
    (Data { feeds, view }, error)
}

fn view(cfg: &Config, feeds: &[FeedData]) -> View {
    let t = now() as i64;
    // From before today starts wherever the PC is, to the end of the last day shown.
    let (from, to) = (t - 2 * 86_400, t + (i64::from(cfg.days) + 1) * 86_400);
    let mut events: Vec<Item> = feeds
        .iter()
        .enumerate()
        .flat_map(|(cal, f)| {
            ics::occurrences(&f.events, from, to).into_iter().map(move |o| Item {
                cal,
                title: o.title,
                location: o.location,
                all_day: o.all_day,
                start: o.start,
                end: o.end,
            })
        })
        .collect();
    events.sort_by_key(|e| (if e.all_day { e.start * 86_400 } else { e.start }, !e.all_day));
    events.truncate(MAX_SHOWN);
    View {
        feeds: feeds.iter().map(|d| FeedView { error: d.error.clone() }).collect(),
        events,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{CalendarFeed, Page};

    const SECRET: &str = "https://calendar.example.test/private/9f8e7d6c5b4a-secret/basic.ics";

    fn settings(on: bool, urls: &[&str]) -> Settings {
        Settings {
            pages: vec![Page { name: "P".into(), layout: vec![if on { vec!["calendar".to_string()] } else { vec![] }], ..Page::default() }],
            calendars: urls.iter().map(|u| CalendarFeed { name: String::new(), color: String::new(), url: u.to_string() }).collect(),
            ..Settings::default()
        }
    }

    fn ics_for(summary: &str, days_ahead: i64) -> Vec<u8> {
        let day = now() as i64 / 86_400 + days_ahead;
        let (y, m, d) = crate::tz::civil_from_days(day);
        format!("BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x\r\nDTSTART:{y:04}{m:02}{d:02}T120000Z\r\nDTEND:{y:04}{m:02}{d:02}T130000Z\r\nSUMMARY:{summary}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n").into_bytes()
    }

    #[test]
    fn only_https_links_on_a_page_are_fetched() {
        assert!(config(&settings(false, &[SECRET])).is_none(), "the widget isn't on a page");
        assert!(config(&settings(true, &[])).is_none());
        assert!(config(&settings(true, &["http://example.test/a.ics", "not a link"])).is_none());
        let c = config(&settings(true, &["webcal://calendar.example.test/a.ics", "http://example.test/b.ics", SECRET])).unwrap();
        assert_eq!(c.links, ["https://calendar.example.test/a.ics", SECRET]);
    }

    #[test]
    fn what_comes_up_is_sorted_and_cut_to_the_days_asked_for() {
        let cfg = config(&settings(true, &[SECRET, "https://calendar.example.test/work.ics"])).unwrap();
        let got = vec![Ok(ics_for("Dentist", 1)), Ok(ics_for("Dinner", 2))];
        let (data, error) = assemble(&cfg, got, None);
        assert_eq!(error, None);
        let titles: Vec<(&str, usize)> = data.view.events.iter().map(|e| (e.title.as_str(), e.cal)).collect();
        assert_eq!(titles, [("Dentist", 0), ("Dinner", 1)]);
        assert_eq!(data.view.feeds.len(), 2);
        // Nothing beyond the days shown (7 by default).
        let far = vec![Ok(ics_for("Next month", 30)), Ok(ics_for("Tomorrow", 1))];
        let (data, _) = assemble(&cfg, far, None);
        assert_eq!(data.view.events.len(), 1);
    }

    #[test]
    fn a_failed_calendar_keeps_its_old_events_and_says_why_without_the_link() {
        let cfg = config(&settings(true, &[SECRET, "https://calendar.example.test/work.ics"])).unwrap();
        let (first, _) = assemble(&cfg, vec![Ok(ics_for("Dentist", 1)), Ok(ics_for("Dinner", 2))], None);

        let (second, error) = assemble(&cfg, vec![Err(Fail::Timeout), Ok(ics_for("Dinner", 2))], Some(&first));
        assert_eq!(error.as_deref(), Some("Took too long"));
        assert_eq!(second.view.events.len(), 2, "the failed calendar's events are still there");
        assert_eq!(second.view.feeds[0].error.as_deref(), Some("Took too long"));
        assert_eq!(second.view.feeds[1].error, None);

        // Never having worked leaves nothing to keep.
        let (third, _) = assemble(&cfg, vec![Err(Fail::Status(404)), Err(Fail::Offline)], None);
        assert!(third.view.events.is_empty());
        assert_eq!(third.view.feeds[0].error.as_deref(), Some("The server said 404"));

        // A sign-in page is not a calendar.
        let (page, error) = assemble(&cfg, vec![Ok(b"<html><body>Sign in</body></html>".to_vec()), Ok(ics_for("Dinner", 2))], None);
        assert_eq!(error.as_deref(), Some("Not a calendar"));
        assert!(page.view.events.iter().all(|e| e.cal == 1));
    }

    #[test]
    fn links_are_never_in_what_the_page_gets() {
        let cfg = config(&settings(true, &[SECRET])).unwrap();
        for r in [Ok(ics_for("Dentist", 1)), Err(Fail::Tls), Err(Fail::Status(403)), Err(Fail::Other)] {
            let (data, error) = assemble(&cfg, vec![r], None);
            let sent = serde_json::to_string(&data.view).unwrap();
            for text in [sent, error.unwrap_or_default()] {
                assert!(!text.contains("secret") && !text.contains("example.test") && !text.contains("https://"), "{text}");
            }
        }
    }
}
