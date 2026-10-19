//! iCalendar (.ics) files, as Google, Outlook and Apple write them: the events in them and the
//! times each one happens, including recurring ones.
//!
//! What's understood: SUMMARY, LOCATION, STATUS; DTSTART, DTEND and DURATION as UTC, floating,
//! TZID or all-day DATE values; RRULE with DAILY, WEEKLY, MONTHLY and YEARLY, INTERVAL, COUNT,
//! UNTIL, BYDAY (also "2MO" and "-1FR"), BYMONTHDAY, BYMONTH, BYSETPOS and WKST; EXDATE, RDATE
//! and RECURRENCE-ID overrides. A rule that uses anything else (BYWEEKNO, BYYEARDAY, hourly
//! recurrence) shows as a single event rather than being guessed at.
//!
//! A recurring event repeats on the wall clock of its own zone, so a 9:00 meeting stays at 9:00
//! when the clocks change. Zones come from `tz`; a zone Windows doesn't know is treated as this
//! PC's own.

use crate::tz::{civil_from_days, days_from_civil, days_in_month, local_to_utc, weekday};
use std::collections::HashMap;

/// The most occurrences one event gives, so a rule like "every day" can't fill the list.
const MAX_PER_EVENT: usize = 400;
/// The most periods (days, weeks, months or years) walked through for one event.
const MAX_PERIODS: i64 = 60_000;
const MAX_TEXT: usize = 200;

#[derive(Clone, Debug, PartialEq)]
pub enum Zone {
    Utc,
    /// No zone given: the PC's own.
    Floating,
    Named(String),
}

/// A date or a moment.
#[derive(Clone, Debug, PartialEq)]
pub enum When {
    /// An all-day date, as days since 1970-01-01.
    Date(i64),
    /// A wall clock reading (seconds since 1970 as if it were UTC) in a zone.
    Time { wall: i64, zone: Zone },
}

impl When {
    /// The Unix time of a moment; None for an all-day date.
    fn unix(&self) -> Option<i64> {
        match self {
            When::Date(_) => None,
            When::Time { wall, zone: Zone::Utc } => Some(*wall),
            When::Time { wall, zone: Zone::Floating } => local_to_utc("", *wall),
            // A zone Windows doesn't know is read as local time rather than dropping the event.
            When::Time { wall, zone: Zone::Named(z) } => local_to_utc(z, *wall).or_else(|| local_to_utc("", *wall)),
        }
    }

    fn day(&self) -> i64 {
        match self {
            When::Date(d) => *d,
            When::Time { wall, .. } => wall.div_euclid(86_400),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Clone, Debug)]
pub struct Rule {
    freq: Freq,
    interval: i64,
    count: Option<u32>,
    until: Option<When>,
    /// (which one in the month, weekday 0 = Sunday); 0 for every one, -1 for the last.
    by_day: Vec<(i64, u8)>,
    by_monthday: Vec<i64>,
    by_month: Vec<i64>,
    by_setpos: Vec<i64>,
    wkst: u8,
}

#[derive(Clone, Debug)]
pub struct Event {
    pub uid: String,
    pub summary: String,
    pub location: String,
    pub start: When,
    /// Seconds for an event with a time; days for an all-day one.
    len: i64,
    rule: Option<Rule>,
    exdates: Vec<When>,
    rdates: Vec<When>,
    /// Set on an edited copy of one occurrence of a recurring event.
    recurrence_id: Option<When>,
    cancelled: bool,
}

/// One time an event happens.
#[derive(Clone, Debug, PartialEq)]
pub struct Occurrence {
    pub title: String,
    pub location: String,
    pub all_day: bool,
    /// Unix seconds, or for an all-day event the day (days since 1970-01-01).
    pub start: i64,
    /// Where it ends: the same unit as `start`, and for all-day events the day after the last.
    pub end: i64,
}

// --- reading the file ----------------------------------------------------------

/// Long lines are folded: the next line starts with a space or tab, which isn't part of the text.
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.trim_start_matches('\u{feff}').split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if let (Some(rest), Some(last)) = (raw.strip_prefix([' ', '\t']), lines.last_mut()) {
            last.push_str(rest);
        } else {
            lines.push(raw.to_string());
        }
    }
    lines
}

struct Prop<'a> {
    name: String,
    params: Vec<(String, String)>,
    value: &'a str,
}

impl Prop<'_> {
    #[cfg(test)]
    fn param(&self, name: &str) -> Option<&str> {
        self.params.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

/// Splits at the first `sep` outside double quotes.
fn split_unquoted(s: &str, sep: char) -> Option<(&str, &str)> {
    let mut quoted = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' => quoted = !quoted,
            c if c == sep && !quoted => return Some((&s[..i], &s[i + c.len_utf8()..])),
            _ => {}
        }
    }
    None
}

fn prop(line: &str) -> Option<Prop<'_>> {
    let (head, value) = split_unquoted(line, ':')?;
    let mut parts = Vec::new();
    let mut rest = head;
    while let Some((a, b)) = split_unquoted(rest, ';') {
        parts.push(a);
        rest = b;
    }
    parts.push(rest);
    let name = parts[0].trim().to_ascii_uppercase();
    let params = parts[1..]
        .iter()
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((k.trim().to_ascii_uppercase(), v.trim().trim_matches('"').to_string()))
        })
        .collect();
    (!name.is_empty()).then_some(Prop { name, params, value })
}

/// Text values escape `\n`, `\,`, `\;` and `\\`. Anything else that isn't printable is dropped.
fn text(value: &str, newline: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n' | 'N') => out.push_str(newline),
                Some(c) => out.push(c),
                None => {}
            },
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.trim().chars().take(MAX_TEXT).collect()
}

fn digits(s: &str) -> Option<i64> {
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok()).flatten()
}

/// `20260115`, `20260115T090000` or `20260115T090000Z`, with the zone from the property's TZID.
fn when(value: &str, params: &[(String, String)]) -> Option<When> {
    let v = value.trim();
    let param = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    let date = |s: &str| -> Option<i64> {
        let (y, m, d) = (digits(s.get(0..4)?)?, digits(s.get(4..6)?)?, digits(s.get(6..8)?)?);
        ((1..=12).contains(&m) && (1..=days_in_month(y, m)).contains(&d) && y >= 1970).then(|| days_from_civil(y, m, d))
    };
    if v.len() == 8 || param("VALUE").is_some_and(|k| k.eq_ignore_ascii_case("DATE")) {
        return date(v.get(..8)?).map(When::Date);
    }
    let (clock, utc) = match v.strip_suffix(['Z', 'z']) {
        Some(c) => (c, true),
        None => (v, false),
    };
    if clock.len() != 15 || clock.as_bytes()[8] != b'T' {
        return None;
    }
    let day = date(&clock[..8])?;
    let (h, m, s) = (digits(&clock[9..11])?, digits(&clock[11..13])?, digits(&clock[13..15])?);
    if h > 23 || m > 59 || s > 60 {
        return None;
    }
    let zone = match param("TZID") {
        _ if utc => Zone::Utc,
        Some(z) if !z.is_empty() => Zone::Named(z.to_string()),
        _ => Zone::Floating,
    };
    Some(When::Time { wall: day * 86_400 + h * 3600 + m * 60 + s.min(59), zone })
}

/// `PT1H30M`, `P1D`, `P2W`, `-PT15M`: seconds.
fn duration(value: &str) -> Option<i64> {
    let v = value.trim();
    let (neg, v) = match v.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, v.strip_prefix('+').unwrap_or(v)),
    };
    let mut rest = v.strip_prefix('P')?;
    let (mut secs, mut in_time, mut any) = (0i64, false, false);
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('T') {
            in_time = true;
            rest = r;
            continue;
        }
        let end = rest.find(|c: char| !c.is_ascii_digit())?;
        let n = digits(&rest[..end])?;
        let unit = match (rest.as_bytes()[end], in_time) {
            (b'W', false) => 7 * 86_400,
            (b'D', false) => 86_400,
            (b'H', true) => 3600,
            (b'M', true) => 60,
            (b'S', true) => 1,
            _ => return None,
        };
        secs += n * unit;
        any = true;
        rest = &rest[end + 1..];
    }
    any.then_some(if neg { -secs } else { secs })
}

/// An RRULE for an event starting at `start`; None for one that's malformed or uses parts this doesn't read.
fn rule(value: &str, start: &When) -> Option<Rule> {
    let mut r = Rule { freq: Freq::Daily, interval: 1, count: None, until: None, by_day: vec![], by_monthday: vec![], by_month: vec![], by_setpos: vec![], wkst: 1 };
    let mut freq = None;
    let all_day = matches!(start, When::Date(_));
    for part in value.split(';') {
        let (k, v) = part.split_once('=')?;
        let list = |max: i64| -> Option<Vec<i64>> {
            v.split(',').map(|n| n.trim().parse::<i64>().ok().filter(|n| *n != 0 && n.abs() <= max)).collect()
        };
        match k.trim().to_ascii_uppercase().as_str() {
            "FREQ" => {
                freq = Some(match v.to_ascii_uppercase().as_str() {
                    "DAILY" => Freq::Daily,
                    "WEEKLY" => Freq::Weekly,
                    "MONTHLY" => Freq::Monthly,
                    "YEARLY" => Freq::Yearly,
                    _ => return None,
                })
            }
            "INTERVAL" => r.interval = v.parse::<i64>().ok().filter(|n| (1..=1000).contains(n))?,
            "COUNT" => r.count = Some(v.parse().ok()?),
            "UNTIL" => {
                // Written as a date or a moment, matching the start; a bad one means no end.
                r.until = when(v, &[]);
                if all_day {
                    r.until = r.until.map(|u| When::Date(u.day()));
                }
            }
            "BYDAY" => {
                for d in v.split(',') {
                    let d = d.trim().to_ascii_uppercase();
                    let at = d.len().checked_sub(2)?;
                    let wd = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"].iter().position(|w| Some(*w) == d.get(at..))? as u8;
                    let ord = if at == 0 { 0 } else { d.get(..at)?.parse::<i64>().ok().filter(|n| *n != 0 && n.abs() <= 53)? };
                    r.by_day.push((ord, wd));
                }
            }
            "BYMONTHDAY" => r.by_monthday = list(31)?,
            "BYMONTH" => r.by_month = list(12)?.into_iter().filter(|m| *m > 0).collect(),
            "BYSETPOS" => r.by_setpos = list(366)?,
            "WKST" => r.wkst = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"].iter().position(|w| w.eq_ignore_ascii_case(v.trim()))? as u8,
            "BYHOUR" | "BYMINUTE" | "BYSECOND" => return None,
            // Weeks of the year, days of the year: not understood, so not guessed at.
            "BYWEEKNO" | "BYYEARDAY" => return None,
            _ => {}
        }
    }
    r.freq = freq?;
    Some(r)
}

#[derive(Default)]
struct Builder {
    uid: String,
    summary: String,
    location: String,
    status: String,
    start: Option<When>,
    end: Option<When>,
    duration: Option<i64>,
    rule: Option<String>,
    exdates: Vec<When>,
    rdates: Vec<When>,
    recurrence_id: Option<When>,
}

impl Builder {
    fn add(&mut self, p: &Prop) {
        match p.name.as_str() {
            "UID" => self.uid = p.value.trim().chars().take(MAX_TEXT).collect(),
            "SUMMARY" => self.summary = text(p.value, " "),
            "LOCATION" => self.location = text(p.value, ", "),
            "STATUS" => self.status = p.value.trim().to_ascii_uppercase(),
            "DTSTART" => self.start = when(p.value, &p.params),
            "DTEND" => self.end = when(p.value, &p.params),
            "DURATION" => self.duration = duration(p.value),
            "RRULE" => self.rule = self.rule.take().or_else(|| Some(p.value.to_string())),
            "EXDATE" => self.exdates.extend(p.value.split(',').filter_map(|v| when(v, &p.params))),
            // Only plain dates and moments; "start/end" periods aren't read.
            "RDATE" => self.rdates.extend(p.value.split(',').filter(|v| !v.contains('/')).filter_map(|v| when(v, &p.params))),
            "RECURRENCE-ID" => self.recurrence_id = when(p.value, &p.params),
            _ => {}
        }
    }

    fn finish(self) -> Option<Event> {
        let start = self.start?;
        let len = match &start {
            When::Date(d) => match (&self.end, self.duration) {
                (Some(e), _) => (e.day() - d).max(1),
                (None, Some(secs)) => ((secs + 86_399) / 86_400).max(1),
                (None, None) => 1,
            },
            When::Time { .. } => match (&self.end, self.duration) {
                (Some(e), _) => e.unix().zip(start.unix()).map_or(0, |(e, s)| (e - s).max(0)),
                (None, Some(secs)) => secs.max(0),
                (None, None) => 0,
            },
        };
        Some(Event {
            uid: self.uid,
            summary: self.summary,
            location: self.location,
            rule: self.rule.as_deref().and_then(|r| rule(r, &start)),
            start,
            len,
            exdates: self.exdates,
            rdates: self.rdates,
            recurrence_id: self.recurrence_id,
            cancelled: self.status == "CANCELLED",
        })
    }
}

/// Every event in an .ics file. Things that aren't events (time zone definitions, alarms inside
/// events, to-dos) are skipped, and so is any event without a readable start.
pub fn parse(file: &str) -> Vec<Event> {
    let mut events = Vec::new();
    let mut current: Option<Builder> = None;
    let mut nested = 0; // inside an alarm or the like, within an event
    for line in unfold(file) {
        let Some(p) = prop(&line) else { continue };
        let in_event = current.is_some();
        match (p.name.as_str(), in_event) {
            ("BEGIN", true) => nested += 1,
            ("BEGIN", false) if p.value.trim().eq_ignore_ascii_case("VEVENT") => current = Some(Builder::default()),
            ("END", true) if nested > 0 => nested -= 1,
            ("END", true) => events.extend(current.take().and_then(Builder::finish)),
            (_, true) if nested == 0 => {
                if let Some(b) = current.as_mut() {
                    b.add(&p);
                }
            }
            _ => {}
        }
    }
    events
}

// --- when events happen --------------------------------------------------------

/// Whether two dates or moments are the same one: for the dates EXDATE and RECURRENCE-ID name.
fn same(a: &When, b: &When) -> bool {
    match (a, b) {
        (When::Date(x), When::Date(y)) => x == y,
        (When::Time { .. }, When::Time { .. }) => a.unix() == b.unix(),
        _ => a.day() == b.day(),
    }
}

/// Days of the month with weekday `wd`: all of them for `ord` 0, else the nth (negative: from the end).
fn weekdays_in(first: i64, n: i64, ord: i64, wd: u8) -> Vec<i64> {
    let all: Vec<i64> = (first..first + n).filter(|d| weekday(*d) == wd).collect();
    let pick = match ord {
        0 => return all,
        o if o > 0 => (o - 1) as usize,
        o => match all.len().checked_sub(o.unsigned_abs() as usize) {
            Some(i) => i,
            None => return vec![],
        },
    };
    all.get(pick).copied().into_iter().collect()
}

/// A month's days that the rule picks; `dom` is the start's day of the month, for rules with no BYDAY or BYMONTHDAY.
fn month_days(r: &Rule, y: i64, m: i64, dom: i64) -> Vec<i64> {
    let (first, n) = (days_from_civil(y, m, 1), days_in_month(y, m));
    let by_md: Vec<i64> = r
        .by_monthday
        .iter()
        .filter_map(|&d| {
            let d = if d < 0 { n + 1 + d } else { d };
            (1..=n).contains(&d).then_some(first + d - 1)
        })
        .collect();
    let by_wd: Vec<i64> = r.by_day.iter().flat_map(|&(ord, wd)| weekdays_in(first, n, ord, wd)).collect();
    match (r.by_monthday.is_empty(), r.by_day.is_empty()) {
        (true, true) => (dom <= n).then_some(first + dom - 1).into_iter().collect(),
        (false, true) => by_md,
        (true, false) => by_wd,
        (false, false) => by_md.into_iter().filter(|d| by_wd.contains(d)).collect(),
    }
}

/// Period number `k` of the rule, counting from the start's: where it begins, and the days it picks.
fn period(r: &Rule, k: i64, start_day: i64) -> (i64, Vec<i64>) {
    let (sy, sm, sdom) = civil_from_days(start_day);
    let month_of = |day: i64| civil_from_days(day).1;
    let (base, mut days) = match r.freq {
        Freq::Daily => {
            let d = start_day + k * r.interval;
            (d, vec![d])
        }
        Freq::Weekly => {
            let back = (i64::from(weekday(start_day)) - i64::from(r.wkst)).rem_euclid(7);
            let week = start_day - back + 7 * r.interval * k;
            let days = if r.by_day.is_empty() {
                vec![week + back]
            } else {
                r.by_day.iter().map(|&(_, wd)| week + (i64::from(wd) - i64::from(r.wkst)).rem_euclid(7)).collect()
            };
            (week, days)
        }
        Freq::Monthly => {
            let months = sy * 12 + (sm - 1) + k * r.interval;
            let (y, m) = (months.div_euclid(12), months.rem_euclid(12) + 1);
            let days = if r.by_month.is_empty() || r.by_month.contains(&m) { month_days(r, y, m, sdom) } else { vec![] };
            (days_from_civil(y, m, 1), days)
        }
        Freq::Yearly => {
            let y = sy + k * r.interval;
            let days = if r.by_month.is_empty() {
                month_days(r, y, sm, sdom)
            } else {
                r.by_month.iter().flat_map(|&m| month_days(r, y, m, sdom)).collect()
            };
            (days_from_civil(y, 1, 1), days)
        }
    };
    // BYDAY, BYMONTHDAY and BYMONTH narrow a day-by-day or week-by-week rule down.
    if matches!(r.freq, Freq::Daily | Freq::Weekly) {
        if !r.by_month.is_empty() {
            days.retain(|d| r.by_month.contains(&month_of(*d)));
        }
        if r.freq == Freq::Daily && !r.by_day.is_empty() {
            days.retain(|d| r.by_day.iter().any(|&(_, wd)| weekday(*d) == wd));
        }
        if !r.by_monthday.is_empty() {
            days.retain(|d| {
                let (y, m, dom) = civil_from_days(*d);
                r.by_monthday.iter().any(|&n| if n > 0 { n == dom } else { days_in_month(y, m) + 1 + n == dom })
            });
        }
    }
    days.sort_unstable();
    days.dedup();
    if !r.by_setpos.is_empty() {
        let all = days;
        days = r
            .by_setpos
            .iter()
            .filter_map(|&p| {
                let i = if p > 0 { p as usize - 1 } else { all.len().checked_sub(p.unsigned_abs() as usize)? };
                all.get(i).copied()
            })
            .collect();
        days.sort_unstable();
        days.dedup();
    }
    (base, days)
}

/// Whether an occurrence starting on `day` (as `cand`) is on or before the rule's UNTIL.
fn until_ok(until: &Option<When>, day: i64, cand: &When) -> bool {
    match (until, cand) {
        (None, _) => true,
        (Some(When::Date(d)), _) => day <= *d,
        (Some(When::Time { wall, zone: Zone::Utc }), When::Time { .. }) => cand.unix().is_none_or(|c| c <= *wall),
        // Written without a zone, it's in the event's own.
        (Some(When::Time { wall, .. }), When::Time { wall: c, .. }) => c <= wall,
        (Some(When::Time { wall, .. }), When::Date(_)) => day <= wall.div_euclid(86_400),
    }
}

/// The times `ev` happens that overlap [from, to) (Unix seconds), added to `out`.
/// `overridden` are the occurrences replaced by edited copies, which show up as events of their own.
fn expand(ev: &Event, from: i64, to: i64, overridden: &[When], out: &mut Vec<Occurrence>) {
    let first_out = out.len();
    let (from_day, to_day) = (from.div_euclid(86_400), to.div_euclid(86_400));
    let all_day = matches!(ev.start, When::Date(_));
    // Adds the occurrence if it's in the window and wasn't excluded or replaced. True once the event has given all it may.
    let mut emit = |w: &When| {
        if !ev.exdates.iter().chain(overridden).any(|x| same(w, x)) {
            let (start, end) = match w {
                When::Date(d) => (*d, d + ev.len),
                When::Time { .. } => match w.unix() {
                    Some(s) => (s, s + ev.len),
                    None => return false,
                },
            };
            let overlaps = if all_day { end * 86_400 > from && start * 86_400 < to } else { end.max(start + 1) > from && start < to };
            if overlaps {
                out.push(Occurrence { title: ev.summary.clone(), location: ev.location.clone(), all_day, start, end });
            }
        }
        out.len() - first_out >= MAX_PER_EVENT
    };

    let (start_day, tod, zone) = match &ev.start {
        When::Date(d) => (*d, 0, None),
        When::Time { wall, zone } => (wall.div_euclid(86_400), wall.rem_euclid(86_400), Some(zone)),
    };
    let at = |day: i64| match zone {
        Some(z) => When::Time { wall: day * 86_400 + tod, zone: z.clone() },
        None => When::Date(day),
    };

    // An edited copy of one occurrence is that one event, whatever rule it carries.
    match ev.rule.as_ref().filter(|_| ev.recurrence_id.is_none()) {
        None => {
            emit(&ev.start);
        }
        Some(r) => {
            // With no COUNT to keep track of, jump close to the window; each period is at most this many days.
            let span = match r.freq {
                Freq::Daily => 1,
                Freq::Weekly => 7,
                Freq::Monthly => 31,
                Freq::Yearly => 366,
            };
            let first = if r.count.is_some() { 0 } else { ((from_day - start_day) / (span * r.interval) - 1).max(0) };
            let mut n = 0u32; // occurrences so far, counting those out of the window or excluded
            'periods: for k in first..first + MAX_PERIODS {
                let (base, days) = period(r, k, start_day);
                if base > to_day + 1 {
                    break;
                }
                for day in days.into_iter().filter(|d| *d >= start_day) {
                    let cand = at(day);
                    if !until_ok(&r.until, day, &cand) {
                        break 'periods;
                    }
                    n += 1;
                    if r.count.is_some_and(|c| n > c) || emit(&cand) {
                        break 'periods;
                    }
                }
            }
        }
    }
    for extra in &ev.rdates {
        if emit(extra) {
            break;
        }
    }
}

/// Everything in `events` that overlaps [from, to) (Unix seconds), earliest first: recurring
/// events expanded, cancelled ones left out, edited occurrences replacing the ones they edit
/// (and cancelled ones removing theirs).
pub fn occurrences(events: &[Event], from: i64, to: i64) -> Vec<Occurrence> {
    let mut replaced: HashMap<&str, Vec<When>> = HashMap::new();
    for e in events.iter().filter(|e| !e.uid.is_empty()) {
        if let Some(id) = &e.recurrence_id {
            replaced.entry(e.uid.as_str()).or_default().push(id.clone());
        }
    }
    let mut out = Vec::new();
    let none: &[When] = &[];
    for ev in events.iter().filter(|e| !e.cancelled) {
        let replaced = replaced.get(ev.uid.as_str()).filter(|_| ev.recurrence_id.is_none());
        expand(ev, from, to, replaced.map_or(none, |v| v.as_slice()), &mut out);
    }
    // All-day events first within their day.
    out.sort_by(|a, b| {
        let key = |o: &Occurrence| (if o.all_day { o.start * 86_400 } else { o.start }, !o.all_day);
        key(a).cmp(&key(b)).then_with(|| a.title.cmp(&b.title))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unix time of a UTC moment.
    fn utc(y: i64, m: i64, d: i64, h: i64, mi: i64) -> i64 {
        days_from_civil(y, m, d) * 86_400 + h * 3600 + mi * 60
    }
    fn day(y: i64, m: i64, d: i64) -> i64 {
        days_from_civil(y, m, d)
    }

    /// What's on between two UTC dates (midnight to midnight).
    fn between(events: &[Event], from: (i64, i64, i64), to: (i64, i64, i64)) -> Vec<Occurrence> {
        occurrences(events, utc(from.0, from.1, from.2, 0, 0), utc(to.0, to.1, to.2, 0, 0))
    }
    fn titles(o: &[Occurrence]) -> Vec<&str> {
        o.iter().map(|o| o.title.as_str()).collect()
    }
    fn starts(o: &[Occurrence]) -> Vec<i64> {
        o.iter().map(|o| o.start).collect()
    }
    fn one(ics_body: &str) -> Vec<Event> {
        parse(&format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:t@example.test\r\nSUMMARY:Thing\r\n{ics_body}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"))
    }

    // Fictional, in the style of what Google Calendar exports.
    const GOOGLE: &str = "BEGIN:VCALENDAR\r
PRODID:-//Google Inc//Google Calendar 70.9054//EN\r
VERSION:2.0\r
CALSCALE:GREGORIAN\r
METHOD:PUBLISH\r
X-WR-CALNAME:Maple Street Volunteers\r
X-WR-TIMEZONE:America/New_York\r
BEGIN:VEVENT\r
DTSTART;TZID=America/New_York:20260105T090000\r
DTEND;TZID=America/New_York:20260105T100000\r
RRULE:FREQ=WEEKLY;UNTIL=20260121T140000Z;BYDAY=MO,WE\r
EXDATE;TZID=America/New_York:20260112T090000\r
DTSTAMP:20260101T000000Z\r
UID:standup-7f3a@google.com\r
SEQUENCE:0\r
STATUS:CONFIRMED\r
SUMMARY:Garden crew standup\r
LOCATION:Shed 2\\, Maple Street Community Garden\r
END:VEVENT\r
BEGIN:VEVENT\r
DTSTART;TZID=America/New_York:20260114T103000\r
DTEND;TZID=America/New_York:20260114T113000\r
RECURRENCE-ID;TZID=America/New_York:20260114T090000\r
UID:standup-7f3a@google.com\r
STATUS:CONFIRMED\r
SUMMARY:Garden crew standup (moved)\r
END:VEVENT\r
BEGIN:VEVENT\r
DTSTART;VALUE=DATE:20260120\r
DTEND;VALUE=DATE:20260122\r
DTSTAMP:20260101T000000Z\r
UID:fair-0001@google.com\r
SUMMARY:Seed swap fair\r
END:VEVENT\r
BEGIN:VEVENT\r
DTSTART:20260116T170000Z\r
DTEND:20260116T180000Z\r
UID:cancelled-1@google.com\r
STATUS:CANCELLED\r
SUMMARY:Cancelled thing\r
END:VEVENT\r
END:VCALENDAR\r
";

    #[test]
    fn google_weekly_with_exdate_override_until_and_allday() {
        let events = parse(GOOGLE);
        assert_eq!(events.len(), 4);
        let all = between(&events, (2026, 1, 1), (2026, 2, 1));
        // Mondays and Wednesdays at 9:00 New York (14:00Z) from the 5th through the 21st (UNTIL
        // is that last 9:00 exactly), without the 12th, and the 14th is the moved one at 10:30.
        let expected = [
            ("Garden crew standup", utc(2026, 1, 5, 14, 0)),
            ("Garden crew standup", utc(2026, 1, 7, 14, 0)),
            ("Garden crew standup (moved)", utc(2026, 1, 14, 15, 30)),
            ("Garden crew standup", utc(2026, 1, 19, 14, 0)),
            ("Garden crew standup", utc(2026, 1, 21, 14, 0)),
        ];
        let timed: Vec<_> = all.iter().filter(|o| !o.all_day).map(|o| (o.title.as_str(), o.start)).collect();
        assert_eq!(timed, expected);
        // An hour each, and the text is unescaped.
        let first = all.iter().find(|o| o.start == utc(2026, 1, 5, 14, 0)).unwrap();
        assert_eq!(first.end - first.start, 3600);
        assert_eq!(first.location, "Shed 2, Maple Street Community Garden");
        // Two all-day days, the second exclusive.
        let fair = all.iter().find(|o| o.all_day).unwrap();
        assert_eq!((fair.title.as_str(), fair.start, fair.end), ("Seed swap fair", day(2026, 1, 20), day(2026, 1, 22)));
        // The cancelled one isn't there.
        assert!(!titles(&all).contains(&"Cancelled thing"));
        // Nothing after UNTIL.
        assert!(between(&events, (2026, 1, 22), (2026, 12, 31)).iter().all(|o| o.all_day || o.title != "Garden crew standup"));
    }

    // Fictional, in the style of Outlook: Windows zone names, a VTIMEZONE block, folded lines.
    const OUTLOOK: &str = "BEGIN:VCALENDAR\r
METHOD:PUBLISH\r
PRODID:Microsoft Exchange Server 2010\r
VERSION:2.0\r
X-WR-CALNAME:Calendar\r
BEGIN:VTIMEZONE\r
TZID:W. Europe Standard Time\r
BEGIN:STANDARD\r
DTSTART:16010101T030000\r
TZOFFSETFROM:+0200\r
TZOFFSETTO:+0100\r
RRULE:FREQ=YEARLY;INTERVAL=1;BYDAY=-1SU;BYMONTH=10\r
END:STANDARD\r
BEGIN:DAYLIGHT\r
DTSTART:16010101T020000\r
TZOFFSETFROM:+0100\r
TZOFFSETTO:+0200\r
RRULE:FREQ=YEARLY;INTERVAL=1;BYDAY=-1SU;BYMONTH=3\r
END:DAYLIGHT\r
END:VTIMEZONE\r
BEGIN:VEVENT\r
CLASS:PUBLIC\r
DESCRIPTION:Agenda attached.\\nBring the Q1 numbers.\r
DTEND;TZID=\"W. Europe Standard Time\":20260105T150000\r
DTSTART;TZID=\"W. Europe Standard Time\":20260105T140000\r
LOCATION:Lindenhof Meeting Room 3\r
 B\r
PRIORITY:5\r
RRULE:FREQ=WEEKLY;COUNT=14;INTERVAL=1;BYDAY=MO;WKST=MO\r
SUMMARY:Budget review\\, Q1\r
UID:040000008200E00074C5B7101A82E0080000000001\r
BEGIN:VALARM\r
TRIGGER:-PT15M\r
ACTION:DISPLAY\r
DESCRIPTION:Reminder\r
END:VALARM\r
END:VEVENT\r
BEGIN:VEVENT\r
DTSTART;TZID=\"W. Europe Standard Time\":20260130T170000\r
DTEND;TZID=\"W. Europe Standard Time\":20260130T173000\r
RRULE:FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1\r
SUMMARY:Month-end close\r
UID:close-1\r
END:VEVENT\r
END:VCALENDAR\r
";

    #[test]
    fn outlook_windows_zone_names_folding_and_alarms() {
        let events = parse(OUTLOOK);
        assert_eq!(events.len(), 2, "the VTIMEZONE and VALARM aren't events");
        let all = between(&events, (2026, 1, 1), (2026, 5, 1));
        let review: Vec<_> = all.iter().filter(|o| o.title == "Budget review, Q1").collect();
        // 14:00 in Berlin on Mondays: 13:00Z in winter, 12:00Z once summer time starts on the 29th of March.
        assert_eq!(review[0].start, utc(2026, 1, 5, 13, 0));
        assert_eq!(review[0].end - review[0].start, 3600);
        assert!(review.iter().any(|o| o.start == utc(2026, 3, 23, 13, 0)), "{:?}", starts(&all));
        assert!(review.iter().any(|o| o.start == utc(2026, 3, 30, 12, 0)), "{:?}", starts(&all));
        // COUNT=14 Mondays from the 5th of January: the last is the 6th of April.
        assert_eq!(review.len(), 14);
        assert_eq!(review.last().unwrap().start, utc(2026, 4, 6, 12, 0));
        // The folded location lost only its fold.
        assert_eq!(review[0].location, "Lindenhof Meeting Room 3B");
        // The last working day of each month, 17:00 in Berlin: Friday the 30th of January, Friday the
        // 27th of February (the 28th is a Saturday), Tuesday the 31st of March (summer time).
        let close: Vec<i64> = all.iter().filter(|o| o.title == "Month-end close").map(|o| o.start).collect();
        assert_eq!(close, [utc(2026, 1, 30, 16, 0), utc(2026, 2, 27, 16, 0), utc(2026, 3, 31, 15, 0), utc(2026, 4, 30, 15, 0)]);
    }

    // Fictional, in the style of Apple Calendar's export.
    const APPLE: &str = "BEGIN:VCALENDAR\r
VERSION:2.0\r
PRODID:-//Apple Inc.//macOS 15.0//EN\r
CALSCALE:GREGORIAN\r
BEGIN:VEVENT\r
CREATED:20251201T120000Z\r
DTEND;TZID=Europe/London:20260311T200000\r
DTSTAMP:20251201T120000Z\r
DTSTART;TZID=Europe/London:20260311T190000\r
LAST-MODIFIED:20251201T120000Z\r
RRULE:FREQ=MONTHLY;BYDAY=2WE\r
SEQUENCE:0\r
SUMMARY:Book club\r
TRANSP:OPAQUE\r
UID:6E1A5D0B-0F55-4F1E-8D44-1C0F0E3E6A11\r
END:VEVENT\r
BEGIN:VEVENT\r
DTSTART;VALUE=DATE:20000301\r
DTEND;VALUE=DATE:20000302\r
RRULE:FREQ=YEARLY\r
SUMMARY:Sam's birthday\r
UID:bday-1\r
TRANSP:TRANSPARENT\r
END:VEVENT\r
BEGIN:VEVENT\r
DTSTART:20260301T100000Z\r
DURATION:PT1H30M\r
SUMMARY:Call with the printer\r
UID:call-1\r
END:VEVENT\r
END:VCALENDAR\r
";

    #[test]
    fn apple_zones_birthdays_and_durations() {
        let events = parse(APPLE);
        assert_eq!(events.len(), 3);
        let all = between(&events, (2026, 3, 1), (2026, 7, 1));
        // The second Wednesday: 11 March, 8 April, 13 May, 10 June, at 19:00 London: GMT, then BST from 29 March.
        let club: Vec<i64> = all.iter().filter(|o| o.title == "Book club").map(|o| o.start).collect();
        assert_eq!(club, [utc(2026, 3, 11, 19, 0), utc(2026, 4, 8, 18, 0), utc(2026, 5, 13, 18, 0), utc(2026, 6, 10, 18, 0)]);
        // A birthday from 2000 comes round to 2026, and only once.
        let bday: Vec<_> = all.iter().filter(|o| o.title == "Sam's birthday").collect();
        assert_eq!(bday.len(), 1);
        assert_eq!((bday[0].all_day, bday[0].start, bday[0].end), (true, day(2026, 3, 1), day(2026, 3, 2)));
        // DURATION as the length.
        let call = all.iter().find(|o| o.title == "Call with the printer").unwrap();
        assert_eq!((call.start, call.end), (utc(2026, 3, 1, 10, 0), utc(2026, 3, 1, 11, 30)));
    }

    #[test]
    fn daily_count_counts_excluded_days() {
        let events = one("DTSTART:20260101T080000Z\r\nDTEND:20260101T083000Z\r\nRRULE:FREQ=DAILY;COUNT=5\r\nEXDATE:20260103T080000Z");
        let o = between(&events, (2025, 12, 1), (2026, 6, 1));
        let days: Vec<i64> = o.iter().map(|o| (o.start - 8 * 3600) / 86_400).collect();
        assert_eq!(days, [day(2026, 1, 1), day(2026, 1, 2), day(2026, 1, 4), day(2026, 1, 5)]);
    }

    #[test]
    fn interval_skips_periods() {
        // Every other Friday from 2 January.
        let events = one("DTSTART:20260102T120000Z\r\nRRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=FR");
        let o = between(&events, (2026, 1, 1), (2026, 2, 15));
        assert_eq!(starts(&o), [utc(2026, 1, 2, 12, 0), utc(2026, 1, 16, 12, 0), utc(2026, 1, 30, 12, 0), utc(2026, 2, 13, 12, 0)]);
        // The same, found by jumping far ahead rather than counting from the start.
        let far = between(&events, (2030, 1, 1), (2030, 1, 30));
        assert_eq!(starts(&far), [utc(2030, 1, 11, 12, 0), utc(2030, 1, 25, 12, 0)]);
    }

    #[test]
    fn monthly_rules() {
        let window = ((2026, 1, 1), (2026, 6, 1));
        // The second Tuesday, and the last Friday.
        let o = between(&one("DTSTART:20260113T090000Z\r\nRRULE:FREQ=MONTHLY;BYDAY=2TU"), window.0, window.1);
        assert_eq!(starts(&o), [utc(2026, 1, 13, 9, 0), utc(2026, 2, 10, 9, 0), utc(2026, 3, 10, 9, 0), utc(2026, 4, 14, 9, 0), utc(2026, 5, 12, 9, 0)]);
        let o = between(&one("DTSTART:20260130T090000Z\r\nRRULE:FREQ=MONTHLY;BYDAY=-1FR"), window.0, window.1);
        assert_eq!(starts(&o), [utc(2026, 1, 30, 9, 0), utc(2026, 2, 27, 9, 0), utc(2026, 3, 27, 9, 0), utc(2026, 4, 24, 9, 0), utc(2026, 5, 29, 9, 0)]);
        // The 31st: months without one are skipped, as the standard says.
        let o = between(&one("DTSTART:20260131T090000Z\r\nRRULE:FREQ=MONTHLY"), window.0, window.1);
        assert_eq!(starts(&o), [utc(2026, 1, 31, 9, 0), utc(2026, 3, 31, 9, 0), utc(2026, 5, 31, 9, 0)]);
        // The last day of the month, and the 1st and 15th.
        let o = between(&one("DTSTART:20260131T090000Z\r\nRRULE:FREQ=MONTHLY;BYMONTHDAY=-1"), window.0, (2026, 4, 1));
        assert_eq!(starts(&o), [utc(2026, 1, 31, 9, 0), utc(2026, 2, 28, 9, 0), utc(2026, 3, 31, 9, 0)]);
        let o = between(&one("DTSTART:20260101T090000Z\r\nRRULE:FREQ=MONTHLY;BYMONTHDAY=1,15"), window.0, (2026, 3, 1));
        assert_eq!(starts(&o), [utc(2026, 1, 1, 9, 0), utc(2026, 1, 15, 9, 0), utc(2026, 2, 1, 9, 0), utc(2026, 2, 15, 9, 0)]);
        // Every third month.
        let o = between(&one("DTSTART:20260115T090000Z\r\nRRULE:FREQ=MONTHLY;INTERVAL=3"), window.0, (2026, 12, 31));
        assert_eq!(starts(&o), [utc(2026, 1, 15, 9, 0), utc(2026, 4, 15, 9, 0), utc(2026, 7, 15, 9, 0), utc(2026, 10, 15, 9, 0)]);
    }

    #[test]
    fn yearly_rules() {
        // A leap day only comes in leap years.
        let o = between(&one("DTSTART:20240229T120000Z\r\nRRULE:FREQ=YEARLY"), (2025, 1, 1), (2033, 1, 1));
        assert_eq!(starts(&o), [utc(2028, 2, 29, 12, 0), utc(2032, 2, 29, 12, 0)]);
        // The fourth Thursday of November.
        let o = between(&one("DTSTART:20251127T120000Z\r\nRRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=4TH"), (2025, 1, 1), (2028, 1, 1));
        assert_eq!(starts(&o), [utc(2025, 11, 27, 12, 0), utc(2026, 11, 26, 12, 0), utc(2027, 11, 25, 12, 0)]);
        // Two months a year.
        let o = between(&one("DTSTART:20260205T120000Z\r\nRRULE:FREQ=YEARLY;BYMONTH=2,8;BYMONTHDAY=5"), (2026, 1, 1), (2027, 3, 1));
        assert_eq!(starts(&o), [utc(2026, 2, 5, 12, 0), utc(2026, 8, 5, 12, 0), utc(2027, 2, 5, 12, 0)]);
    }

    #[test]
    fn daily_with_byday_and_bymonth_narrowing() {
        let o = between(&one("DTSTART:20260105T070000Z\r\nRRULE:FREQ=DAILY;BYDAY=MO,TU,WE,TH,FR;COUNT=7"), (2026, 1, 1), (2026, 2, 1));
        let days: Vec<i64> = o.iter().map(|o| o.start / 86_400).collect();
        assert_eq!(days, [day(2026, 1, 5), day(2026, 1, 6), day(2026, 1, 7), day(2026, 1, 8), day(2026, 1, 9), day(2026, 1, 12), day(2026, 1, 13)]);
        let o = between(&one("DTSTART:20260105T070000Z\r\nRRULE:FREQ=WEEKLY;BYMONTH=1,3;BYDAY=MO"), (2026, 1, 1), (2026, 4, 1));
        let days: Vec<i64> = o.iter().map(|o| o.start / 86_400).collect();
        assert_eq!(days, [day(2026, 1, 5), day(2026, 1, 12), day(2026, 1, 19), day(2026, 1, 26), day(2026, 3, 2), day(2026, 3, 9), day(2026, 3, 16), day(2026, 3, 23), day(2026, 3, 30)]);
    }

    #[test]
    fn all_day_events_and_exdates_by_date() {
        let events = one("DTSTART;VALUE=DATE:20260102\r\nRRULE:FREQ=DAILY;COUNT=4\r\nEXDATE;VALUE=DATE:20260103");
        let o = between(&events, (2025, 12, 30), (2026, 1, 20));
        assert_eq!(starts(&o), [day(2026, 1, 2), day(2026, 1, 4), day(2026, 1, 5)]);
        assert!(o.iter().all(|o| o.all_day && o.end == o.start + 1));
        // A multi-day event that began before the window still counts while it overlaps.
        let events = one("DTSTART;VALUE=DATE:20260110\r\nDTEND;VALUE=DATE:20260115");
        assert_eq!(between(&events, (2026, 1, 13), (2026, 1, 14)).len(), 1);
        assert_eq!(between(&events, (2026, 1, 15), (2026, 1, 16)).len(), 0);
        // Until, as a date.
        let events = one("DTSTART;VALUE=DATE:20260101\r\nRRULE:FREQ=WEEKLY;UNTIL=20260115");
        assert_eq!(starts(&between(&events, (2026, 1, 1), (2026, 3, 1))), [day(2026, 1, 1), day(2026, 1, 8), day(2026, 1, 15)]);
    }

    #[test]
    fn zones_and_daylight_saving_in_recurrence() {
        // 9:00 in Berlin every Monday, across the change on the 29th of March.
        let events = one("DTSTART;TZID=Europe/Berlin:20260323T090000\r\nRRULE:FREQ=WEEKLY;COUNT=2");
        assert_eq!(starts(&between(&events, (2026, 3, 1), (2026, 5, 1))), [utc(2026, 3, 23, 8, 0), utc(2026, 3, 30, 7, 0)]);
        // The same wall clock time in New York, whose clocks change two weeks earlier.
        let events = one("DTSTART;TZID=America/New_York:20260302T090000\r\nRRULE:FREQ=WEEKLY;COUNT=2;INTERVAL=2");
        assert_eq!(starts(&between(&events, (2026, 3, 1), (2026, 5, 1))), [utc(2026, 3, 2, 14, 0), utc(2026, 3, 16, 13, 0)]);
        // A UTC event has no zone to follow.
        let events = one("DTSTART:20260323T090000Z\r\nRRULE:FREQ=WEEKLY;COUNT=2");
        assert_eq!(starts(&between(&events, (2026, 3, 1), (2026, 5, 1))), [utc(2026, 3, 23, 9, 0), utc(2026, 3, 30, 9, 0)]);
    }

    #[test]
    fn floating_times_and_unknown_zones_are_this_pcs_own() {
        let wall = utc(2026, 1, 15, 18, 0);
        let here = local_to_utc("", wall).unwrap();
        let events = one("DTSTART:20260115T180000\r\nDTEND:20260115T190000");
        assert_eq!(starts(&between(&events, (2026, 1, 14), (2026, 1, 17))), [here]);
        let events = one("DTSTART;TZID=Customized Time Zone:20260115T180000");
        assert_eq!(starts(&between(&events, (2026, 1, 14), (2026, 1, 17))), [here]);
    }

    #[test]
    fn overrides_cancellations_and_rdates() {
        let file = "BEGIN:VCALENDAR\r
BEGIN:VEVENT\r
UID:a\r
DTSTART:20260105T100000Z\r
RRULE:FREQ=DAILY;COUNT=4\r
SUMMARY:Daily\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:a\r
RECURRENCE-ID:20260106T100000Z\r
DTSTART:20260106T120000Z\r
SUMMARY:Daily (later)\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:a\r
RECURRENCE-ID:20260107T100000Z\r
DTSTART:20260107T100000Z\r
STATUS:CANCELLED\r
SUMMARY:Daily\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:b\r
DTSTART:20260110T090000Z\r
RDATE:20260112T090000Z,20260114T090000Z\r
SUMMARY:Odd days\r
END:VEVENT\r
END:VCALENDAR\r
";
        let o = between(&parse(file), (2026, 1, 1), (2026, 2, 1));
        let got: Vec<(&str, i64)> = o.iter().map(|o| (o.title.as_str(), o.start)).collect();
        assert_eq!(
            got,
            [
                ("Daily", utc(2026, 1, 5, 10, 0)),
                ("Daily (later)", utc(2026, 1, 6, 12, 0)),
                // the 7th was cancelled
                ("Daily", utc(2026, 1, 8, 10, 0)),
                ("Odd days", utc(2026, 1, 10, 9, 0)),
                ("Odd days", utc(2026, 1, 12, 9, 0)),
                ("Odd days", utc(2026, 1, 14, 9, 0)),
            ]
        );
    }

    #[test]
    fn the_window_decides_what_shows() {
        let events = one("DTSTART:20260115T100000Z\r\nDTEND:20260115T120000Z");
        // Overlapping counts, before the start and after the end doesn't.
        assert_eq!(occurrences(&events, utc(2026, 1, 15, 11, 0), utc(2026, 1, 15, 13, 0)).len(), 1);
        assert_eq!(occurrences(&events, utc(2026, 1, 15, 12, 0), utc(2026, 1, 15, 13, 0)).len(), 0);
        assert_eq!(occurrences(&events, utc(2026, 1, 15, 8, 0), utc(2026, 1, 15, 10, 0)).len(), 0);
        // An event with no end is a moment.
        let events = one("DTSTART:20260115T100000Z");
        assert_eq!(occurrences(&events, utc(2026, 1, 15, 10, 0), utc(2026, 1, 15, 10, 1)).len(), 1);
        assert_eq!(occurrences(&events, utc(2026, 1, 15, 10, 1), utc(2026, 1, 15, 11, 0)).len(), 0);
    }

    #[test]
    fn an_endless_rule_stays_bounded() {
        let events = one("DTSTART:20150101T000000Z\r\nRRULE:FREQ=DAILY");
        let o = between(&events, (2026, 1, 1), (2026, 1, 15));
        assert_eq!(o.len(), 14);
        // Even asking for a century gives a capped number, not a hang.
        let o = between(&events, (2026, 1, 1), (2100, 1, 1));
        assert!(o.len() <= MAX_PER_EVENT);
    }

    #[test]
    fn a_calendar_at_the_size_limit_is_quick() {
        // The most a calendar can be (5 MB, see calendar.rs): thousands of events, a third of them recurring.
        let mut file = String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n");
        let rules = ["FREQ=DAILY", "FREQ=WEEKLY;BYDAY=MO,WE,FR", "FREQ=MONTHLY;BYDAY=2TU", "FREQ=YEARLY"];
        let filler = "Lorem ipsum dolor sit amet. ".repeat(6);
        let mut n = 0;
        while file.len() < 5 << 20 {
            let (year, m, d) = (15 + n % 10, 1 + n % 12, 1 + n % 28);
            let rule = if n % 3 == 0 { format!("RRULE:{}\r\n", rules[(n / 3) % rules.len()]) } else { String::new() };
            file.push_str(&format!(
                "BEGIN:VEVENT\r\nUID:event-{n}@example.test\r\nDTSTART;TZID=Europe/Berlin:20{year:02}{m:02}{d:02}T090000\r\nDTEND;TZID=Europe/Berlin:20{year:02}{m:02}{d:02}T100000\r\n{rule}SUMMARY:Event number {n} with a reasonably long title\r\nLOCATION:Somewhere\\, Room {n}\r\nDESCRIPTION:{filler}\r\nEND:VEVENT\r\n"
            ));
            n += 1;
        }
        file.push_str("END:VCALENDAR\r\n");

        let started = std::time::Instant::now();
        let events = parse(&file);
        let parsed = started.elapsed();
        let now = utc(2026, 10, 19, 0, 0);
        let started = std::time::Instant::now();
        let coming = occurrences(&events, now, now + 15 * 86_400);
        let expanded = started.elapsed();
        println!("{} events in {} KB: parsed in {parsed:?}, a 14-day agenda in {expanded:?} ({} occurrences)", events.len(), file.len() / 1024, coming.len());
        assert_eq!(events.len(), n);
        assert!(!coming.is_empty());
        // Generous, for a debug build on a slow CI machine; a release build is many times faster.
        assert!(parsed.as_secs() < 20 && expanded.as_secs() < 20);
    }

    #[test]
    fn rules_it_cant_do_show_once() {
        let events = one("DTSTART:20260105T100000Z\r\nRRULE:FREQ=YEARLY;BYWEEKNO=20;BYDAY=MO");
        assert_eq!(between(&events, (2026, 1, 1), (2030, 1, 1)).len(), 1);
        let events = one("DTSTART:20260105T100000Z\r\nRRULE:FREQ=HOURLY;COUNT=5");
        assert_eq!(between(&events, (2026, 1, 1), (2030, 1, 1)).len(), 1);
    }

    #[test]
    fn broken_input_is_skipped_not_fatal() {
        assert!(parse("").is_empty());
        assert!(parse("not a calendar at all\r\n\u{0}\u{1}").is_empty());
        assert!(parse("BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nSUMMARY:No start\r\nEND:VEVENT\r\nEND:VCALENDAR").is_empty());
        // A bad date drops that event only; so does a missing END.
        let file = "BEGIN:VEVENT\r\nDTSTART:20261301T100000Z\r\nSUMMARY:Bad month\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nDTSTART:20260105T100000Z\r\nSUMMARY:Fine\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nDTSTART:20260106T100000Z\r\nSUMMARY:Never ends";
        let events = parse(file);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].summary, "Fine");
        // Bare LF line ends and a byte order mark, as some exporters write.
        let events = parse("\u{feff}BEGIN:VEVENT\nDTSTART:20260105T100000Z\nSUMMARY:Unix\nEND:VEVENT\n");
        assert_eq!(events[0].summary, "Unix");
    }

    #[test]
    fn text_is_cleaned() {
        let events = one("SUMMARY:Lunch\\; \\\\ the\\Nteam\\, ok\u{7}\r\nLOCATION:Line one\\nLine two\r\nDTSTART:20260105T100000Z");
        // (the later SUMMARY wins over the helper's own)
        assert_eq!(events[0].summary, "Lunch; \\ the team, ok");
        assert_eq!(events[0].location, "Line one, Line two");
        let long = format!("SUMMARY:{}\r\nDTSTART:20260105T100000Z", "x".repeat(1000));
        assert_eq!(one(&long)[0].summary.chars().count(), MAX_TEXT);
    }

    #[test]
    fn durations_and_times_parse() {
        assert_eq!(duration("PT1H30M"), Some(5400));
        assert_eq!(duration("P1DT2H"), Some(93_600));
        assert_eq!(duration("P2W"), Some(1_209_600));
        assert_eq!(duration("-PT15M"), Some(-900));
        assert_eq!(duration("PT"), None);
        assert_eq!(duration("1H"), None);
        assert_eq!(duration("P1H"), None);
        let p = |tz: &str| if tz.is_empty() { vec![] } else { vec![("TZID".to_string(), tz.to_string())] };
        assert_eq!(when("20260105T090000Z", &p("")), Some(When::Time { wall: utc(2026, 1, 5, 9, 0), zone: Zone::Utc }));
        assert_eq!(when("20260105T090000", &p("")), Some(When::Time { wall: utc(2026, 1, 5, 9, 0), zone: Zone::Floating }));
        assert_eq!(when("20260105T090000", &p("Europe/Paris")), Some(When::Time { wall: utc(2026, 1, 5, 9, 0), zone: Zone::Named("Europe/Paris".into()) }));
        assert_eq!(when("20260105", &p("")), Some(When::Date(day(2026, 1, 5))));
        assert_eq!(when("20260230", &p("")), None);
        assert_eq!(when("20260105T250000", &p("")), None);
        assert_eq!(when("tomorrow", &p("")), None);
    }

    #[test]
    fn properties_split_at_the_right_colon() {
        let p = prop("DTSTART;TZID=\"Europe/Berlin\";VALUE=DATE-TIME:20260105T090000").unwrap();
        assert_eq!((p.name.as_str(), p.param("TZID"), p.value), ("DTSTART", Some("Europe/Berlin"), "20260105T090000"));
        let p = prop("DESCRIPTION;ALTREP=\"cid:part1:x\":Meet at 10:30").unwrap();
        assert_eq!((p.name.as_str(), p.value), ("DESCRIPTION", "Meet at 10:30"));
        assert!(prop("no colon here").is_none());
    }
}
