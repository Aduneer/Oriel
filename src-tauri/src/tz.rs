//! Time zones and calendar dates, for the calendar widget.
//!
//! Zone rules come from Windows itself: WinRT's `Calendar` takes IANA ids ("Europe/Berlin", which
//! Google and Apple put in their files) and knows every daylight saving rule, so there's no
//! time zone database to ship. Outlook names zones the Windows way ("W. Europe Standard Time"),
//! which `Calendar` doesn't take, so those go through a table first.

use std::cell::RefCell;
use std::collections::HashMap;
use windows::core::HSTRING;
use windows::Foundation::DateTime;
use windows::Globalization::Calendar;

// --- dates ---------------------------------------------------------------------

/// Days since 1970-01-01 of a calendar date (proleptic Gregorian).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date `days` after 1970-01-01, as (year, month 1–12, day 1–31).
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// 0 for Sunday to 6 for Saturday.
pub fn weekday(days: i64) -> u8 {
    (days + 4).rem_euclid(7) as u8
}

pub fn days_in_month(y: i64, m: i64) -> i64 {
    days_from_civil(if m == 12 { y + 1 } else { y }, if m == 12 { 1 } else { m + 1 }, 1) - days_from_civil(y, m, 1)
}

// --- zones ---------------------------------------------------------------------

/// 100 ns ticks between 1601 (WinRT's epoch) and 1970.
const TICKS_1970: i64 = 116_444_736_000_000_000;
/// Zones are named in files we don't control; don't remember an endless supply of them.
const MAX_ZONES: usize = 64;

thread_local! {
    // A Calendar per zone, kept because making one is the slow part. They belong to the thread
    // that made them, which is the calendar thread.
    static CALENDARS: RefCell<HashMap<String, Option<Calendar>>> = RefCell::new(HashMap::new());
}

fn make(zone: &str) -> Option<Calendar> {
    let cal = Calendar::new().ok()?;
    // Whatever the user's own calendar is (Buddhist, Hijri), we want plain dates and hours 0–23.
    cal.ChangeCalendarSystem(&HSTRING::from("GregorianCalendar")).ok()?;
    cal.ChangeClock(&HSTRING::from("24HourClock")).ok()?;
    if !zone.is_empty() {
        cal.ChangeTimeZone(&HSTRING::from(iana(zone))).ok()?;
    }
    Some(cal)
}

/// Seconds east of UTC in `zone` at the Unix time `unix`. An empty `zone` is this PC's own;
/// None for a zone Windows has never heard of.
pub fn offset(zone: &str, unix: i64) -> Option<i64> {
    CALENDARS.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() >= MAX_ZONES && !c.contains_key(zone) {
            c.clear();
        }
        let cal = c.entry(zone.to_string()).or_insert_with(|| make(zone)).as_ref()?;
        cal.SetDateTime(DateTime { UniversalTime: unix.checked_mul(10_000_000)?.checked_add(TICKS_1970)? }).ok()?;
        let day = days_from_civil(cal.Year().ok()?.into(), cal.Month().ok()?.into(), cal.Day().ok()?.into());
        let secs = i64::from(cal.Hour().ok()?) * 3600 + i64::from(cal.Minute().ok()?) * 60 + i64::from(cal.Second().ok()?);
        Some(day * 86_400 + secs - unix)
    })
}

/// A wall clock reading in `zone`, given as seconds since 1970 as if it were UTC, to the real
/// Unix time. In the hour that daylight saving skips or repeats it picks one of the two.
pub fn local_to_utc(zone: &str, wall: i64) -> Option<i64> {
    let guess = wall - offset(zone, wall)?;
    Some(wall - offset(zone, guess)?)
}

/// Windows' zone names to the IANA ones `Calendar` takes (Unicode CLDR's table, one id per name).
/// Names that are already IANA ids come back as they are.
fn iana(name: &str) -> &str {
    WINDOWS_ZONES.iter().find(|(w, _)| w.eq_ignore_ascii_case(name)).map_or(name, |(_, i)| i)
}

const WINDOWS_ZONES: &[(&str, &str)] = &[
    ("Dateline Standard Time", "Etc/GMT+12"),
    ("UTC-11", "Etc/GMT+11"),
    ("Aleutian Standard Time", "America/Adak"),
    ("Hawaiian Standard Time", "Pacific/Honolulu"),
    ("Marquesas Standard Time", "Pacific/Marquesas"),
    ("Alaskan Standard Time", "America/Anchorage"),
    ("UTC-09", "Etc/GMT+9"),
    ("Pacific Standard Time (Mexico)", "America/Tijuana"),
    ("UTC-08", "Etc/GMT+8"),
    ("Pacific Standard Time", "America/Los_Angeles"),
    ("US Mountain Standard Time", "America/Phoenix"),
    ("Mountain Standard Time (Mexico)", "America/Mazatlan"),
    ("Mountain Standard Time", "America/Denver"),
    ("Yukon Standard Time", "America/Whitehorse"),
    ("Central America Standard Time", "America/Guatemala"),
    ("Central Standard Time", "America/Chicago"),
    ("Easter Island Standard Time", "Pacific/Easter"),
    ("Central Standard Time (Mexico)", "America/Mexico_City"),
    ("Canada Central Standard Time", "America/Regina"),
    ("SA Pacific Standard Time", "America/Bogota"),
    ("Eastern Standard Time (Mexico)", "America/Cancun"),
    ("Eastern Standard Time", "America/New_York"),
    ("Haiti Standard Time", "America/Port-au-Prince"),
    ("Cuba Standard Time", "America/Havana"),
    ("US Eastern Standard Time", "America/Indianapolis"),
    ("Turks And Caicos Standard Time", "America/Grand_Turk"),
    ("Paraguay Standard Time", "America/Asuncion"),
    ("Atlantic Standard Time", "America/Halifax"),
    ("Venezuela Standard Time", "America/Caracas"),
    ("Central Brazilian Standard Time", "America/Cuiaba"),
    ("SA Western Standard Time", "America/La_Paz"),
    ("Pacific SA Standard Time", "America/Santiago"),
    ("Newfoundland Standard Time", "America/St_Johns"),
    ("Tocantins Standard Time", "America/Araguaina"),
    ("E. South America Standard Time", "America/Sao_Paulo"),
    ("SA Eastern Standard Time", "America/Cayenne"),
    ("Argentina Standard Time", "America/Buenos_Aires"),
    ("Greenland Standard Time", "America/Godthab"),
    ("Montevideo Standard Time", "America/Montevideo"),
    ("Magallanes Standard Time", "America/Punta_Arenas"),
    ("Saint Pierre Standard Time", "America/Miquelon"),
    ("Bahia Standard Time", "America/Bahia"),
    ("UTC-02", "Etc/GMT+2"),
    ("Mid-Atlantic Standard Time", "Etc/GMT+2"),
    ("Azores Standard Time", "Atlantic/Azores"),
    ("Cape Verde Standard Time", "Atlantic/Cape_Verde"),
    ("UTC", "Etc/UTC"),
    ("GMT Standard Time", "Europe/London"),
    ("Greenwich Standard Time", "Atlantic/Reykjavik"),
    ("Sao Tome Standard Time", "Africa/Sao_Tome"),
    ("Morocco Standard Time", "Africa/Casablanca"),
    ("W. Europe Standard Time", "Europe/Berlin"),
    ("Central Europe Standard Time", "Europe/Budapest"),
    ("Romance Standard Time", "Europe/Paris"),
    ("Central European Standard Time", "Europe/Warsaw"),
    ("W. Central Africa Standard Time", "Africa/Lagos"),
    ("GTB Standard Time", "Europe/Bucharest"),
    ("Middle East Standard Time", "Asia/Beirut"),
    ("Egypt Standard Time", "Africa/Cairo"),
    ("E. Europe Standard Time", "Europe/Chisinau"),
    ("Syria Standard Time", "Asia/Damascus"),
    ("West Bank Standard Time", "Asia/Hebron"),
    ("South Africa Standard Time", "Africa/Johannesburg"),
    ("FLE Standard Time", "Europe/Kiev"),
    ("Israel Standard Time", "Asia/Jerusalem"),
    ("South Sudan Standard Time", "Africa/Juba"),
    ("Kaliningrad Standard Time", "Europe/Kaliningrad"),
    ("Sudan Standard Time", "Africa/Khartoum"),
    ("Libya Standard Time", "Africa/Tripoli"),
    ("Namibia Standard Time", "Africa/Windhoek"),
    ("Jordan Standard Time", "Asia/Amman"),
    ("Arabic Standard Time", "Asia/Baghdad"),
    ("Turkey Standard Time", "Europe/Istanbul"),
    ("Arab Standard Time", "Asia/Riyadh"),
    ("Belarus Standard Time", "Europe/Minsk"),
    ("Russian Standard Time", "Europe/Moscow"),
    ("E. Africa Standard Time", "Africa/Nairobi"),
    ("Iran Standard Time", "Asia/Tehran"),
    ("Arabian Standard Time", "Asia/Dubai"),
    ("Astrakhan Standard Time", "Europe/Astrakhan"),
    ("Azerbaijan Standard Time", "Asia/Baku"),
    ("Russia Time Zone 3", "Europe/Samara"),
    ("Mauritius Standard Time", "Indian/Mauritius"),
    ("Saratov Standard Time", "Europe/Saratov"),
    ("Georgian Standard Time", "Asia/Tbilisi"),
    ("Volgograd Standard Time", "Europe/Volgograd"),
    ("Caucasus Standard Time", "Asia/Yerevan"),
    ("Afghanistan Standard Time", "Asia/Kabul"),
    ("West Asia Standard Time", "Asia/Tashkent"),
    ("Ekaterinburg Standard Time", "Asia/Yekaterinburg"),
    ("Pakistan Standard Time", "Asia/Karachi"),
    ("Qyzylorda Standard Time", "Asia/Qyzylorda"),
    ("India Standard Time", "Asia/Calcutta"),
    ("Sri Lanka Standard Time", "Asia/Colombo"),
    ("Nepal Standard Time", "Asia/Katmandu"),
    ("Central Asia Standard Time", "Asia/Almaty"),
    ("Bangladesh Standard Time", "Asia/Dhaka"),
    ("Omsk Standard Time", "Asia/Omsk"),
    ("Myanmar Standard Time", "Asia/Rangoon"),
    ("SE Asia Standard Time", "Asia/Bangkok"),
    ("Altai Standard Time", "Asia/Barnaul"),
    ("W. Mongolia Standard Time", "Asia/Hovd"),
    ("North Asia Standard Time", "Asia/Krasnoyarsk"),
    ("N. Central Asia Standard Time", "Asia/Novosibirsk"),
    ("Tomsk Standard Time", "Asia/Tomsk"),
    ("China Standard Time", "Asia/Shanghai"),
    ("North Asia East Standard Time", "Asia/Irkutsk"),
    ("Singapore Standard Time", "Asia/Singapore"),
    ("W. Australia Standard Time", "Australia/Perth"),
    ("Taipei Standard Time", "Asia/Taipei"),
    ("Ulaanbaatar Standard Time", "Asia/Ulaanbaatar"),
    ("Aus Central W. Standard Time", "Australia/Eucla"),
    ("Transbaikal Standard Time", "Asia/Chita"),
    ("Tokyo Standard Time", "Asia/Tokyo"),
    ("North Korea Standard Time", "Asia/Pyongyang"),
    ("Korea Standard Time", "Asia/Seoul"),
    ("Yakutsk Standard Time", "Asia/Yakutsk"),
    ("Cen. Australia Standard Time", "Australia/Adelaide"),
    ("AUS Central Standard Time", "Australia/Darwin"),
    ("E. Australia Standard Time", "Australia/Brisbane"),
    ("AUS Eastern Standard Time", "Australia/Sydney"),
    ("West Pacific Standard Time", "Pacific/Port_Moresby"),
    ("Tasmania Standard Time", "Australia/Hobart"),
    ("Vladivostok Standard Time", "Asia/Vladivostok"),
    ("Lord Howe Standard Time", "Australia/Lord_Howe"),
    ("Bougainville Standard Time", "Pacific/Bougainville"),
    ("Russia Time Zone 10", "Asia/Srednekolymsk"),
    ("Magadan Standard Time", "Asia/Magadan"),
    ("Norfolk Standard Time", "Pacific/Norfolk"),
    ("Sakhalin Standard Time", "Asia/Sakhalin"),
    ("Central Pacific Standard Time", "Pacific/Guadalcanal"),
    ("Russia Time Zone 11", "Asia/Kamchatka"),
    ("New Zealand Standard Time", "Pacific/Auckland"),
    ("UTC+12", "Etc/GMT-12"),
    ("Fiji Standard Time", "Pacific/Fiji"),
    ("Kamchatka Standard Time", "Asia/Kamchatka"),
    ("Chatham Islands Standard Time", "Pacific/Chatham"),
    ("UTC+13", "Etc/GMT-13"),
    ("Tonga Standard Time", "Pacific/Tongatapu"),
    ("Samoa Standard Time", "Pacific/Apia"),
    ("Line Islands Standard Time", "Pacific/Kiritimati"),
];

#[cfg(test)]
mod tests {
    use super::*;

    const JAN_15_NOON: i64 = 1_768_478_400; // 2026-01-15 12:00 UTC
    const JUL_01_NOON: i64 = 1_782_907_200; // 2026-07-01 12:00 UTC

    #[test]
    fn dates_round_trip() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2026, 7, 1) * 86_400 + 12 * 3600, JUL_01_NOON);
        assert_eq!(days_from_civil(2026, 1, 15) * 86_400 + 12 * 3600, JAN_15_NOON);
        for z in [-800_000, -1, 0, 59, 11_016, 20_000, 2_932_896] {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z);
        }
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
        assert_eq!(weekday(0), 4); // 1970-01-01 was a Thursday
        assert_eq!(weekday(days_from_civil(2026, 10, 19)), 1); // a Monday
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2026, 12), 31);
    }

    #[test]
    fn iana_ids_and_their_daylight_saving() {
        assert_eq!(offset("America/New_York", JAN_15_NOON), Some(-5 * 3600));
        assert_eq!(offset("America/New_York", JUL_01_NOON), Some(-4 * 3600));
        assert_eq!(offset("Europe/Berlin", JAN_15_NOON), Some(3600));
        assert_eq!(offset("Europe/Berlin", JUL_01_NOON), Some(7200));
        assert_eq!(offset("Asia/Kolkata", JUL_01_NOON), Some(5 * 3600 + 1800));
        assert_eq!(offset("Australia/Lord_Howe", JAN_15_NOON), Some(11 * 3600)); // summer: +11
        assert_eq!(offset("Australia/Lord_Howe", JUL_01_NOON), Some(10 * 3600 + 1800));
        assert_eq!(offset("UTC", JUL_01_NOON), Some(0));
        assert_eq!(offset("Etc/GMT+5", JUL_01_NOON), Some(-5 * 3600));
    }

    #[test]
    fn windows_names_work_like_iana_ones() {
        assert_eq!(offset("Eastern Standard Time", JUL_01_NOON), Some(-4 * 3600));
        assert_eq!(offset("W. Europe Standard Time", JAN_15_NOON), Some(3600));
        assert_eq!(offset("w. europe standard time", JUL_01_NOON), Some(7200));
        assert_eq!(offset("India Standard Time", JUL_01_NOON), Some(5 * 3600 + 1800));
        assert_eq!(offset("Pacific Standard Time", JAN_15_NOON), Some(-8 * 3600));
        assert_eq!(offset("AUS Eastern Standard Time", JAN_15_NOON), Some(11 * 3600));
    }

    #[test]
    fn every_table_entry_is_a_zone_windows_knows() {
        for (windows, iana) in WINDOWS_ZONES {
            assert!(offset(windows, JUL_01_NOON).is_some(), "{windows} -> {iana}");
        }
    }

    #[test]
    fn unknown_zones_are_none() {
        assert_eq!(offset("Nowhere/Atlantis", JUL_01_NOON), None);
        assert_eq!(offset("(UTC+01:00) Somewhere", JUL_01_NOON), None);
        assert_eq!(local_to_utc("Nowhere/Atlantis", JUL_01_NOON), None);
    }

    #[test]
    fn wall_clock_to_utc() {
        let wall = |y: i64, m: i64, d: i64, h: i64, mi: i64| days_from_civil(y, m, d) * 86_400 + h * 3600 + mi * 60;
        // Nine in the morning stays nine through the clocks changing.
        assert_eq!(local_to_utc("Europe/Berlin", wall(2026, 3, 27, 9, 0)), Some(wall(2026, 3, 27, 8, 0)));
        assert_eq!(local_to_utc("Europe/Berlin", wall(2026, 3, 30, 9, 0)), Some(wall(2026, 3, 30, 7, 0)));
        assert_eq!(local_to_utc("America/New_York", wall(2026, 11, 2, 9, 0)), Some(wall(2026, 11, 2, 14, 0)));
        // A time that exists twice or not at all still lands within an hour of where it was meant.
        let gap = local_to_utc("America/New_York", wall(2026, 3, 8, 2, 30)).unwrap();
        assert!((gap - wall(2026, 3, 8, 7, 30)).abs() <= 3600);
        // This PC's own zone: whatever it is, it's consistent with itself.
        let local = local_to_utc("", wall(2026, 7, 1, 12, 0)).unwrap();
        assert_eq!(local + offset("", local).unwrap(), wall(2026, 7, 1, 12, 0));
    }
}
