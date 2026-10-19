//! HTTPS GET, for the widgets that go online (weather, calendar). Nothing else in Oriel uses it.
//!
//! It goes through Windows' own HTTP client (WinRT's `Windows.Web.Http`), so TLS, certificates
//! and the proxy settings are the system's, and Oriel links nothing it didn't already. Only
//! https:// is fetched, redirects included; each response is cut off at a size limit, and each
//! request has a deadline. Errors never carry the URL, because calendar links are secrets.

use std::time::{Duration, Instant};
use windows::core::{Interface, HSTRING};
use windows::Foundation::Uri;
use windows::Storage::Streams::{DataReader, InputStreamOptions};
use windows::Web::Http::Filters::{HttpBaseProtocolFilter, HttpCookieUsageBehavior, IHttpFilter};
use windows::Web::Http::{HttpClient, HttpCompletionOption, HttpMethod, HttpRequestMessage};

const MAX_URL: usize = 2048;
const MAX_REDIRECTS: usize = 4;
const CHUNK: u32 = 32 * 1024;

/// Why a fetch failed, in terms that are safe to show.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fail {
    NotHttps,
    Offline,
    Timeout,
    Tls,
    Status(u16),
    TooLarge,
    Other,
}

impl Fail {
    pub fn text(self) -> String {
        match self {
            Fail::NotHttps => "Only secure (https) links work".into(),
            Fail::Offline => "Can't connect".into(),
            Fail::Timeout => "Took too long".into(),
            Fail::Tls => "Secure connection failed".into(),
            Fail::Status(code) => format!("The server said {code}"),
            Fail::TooLarge => "Response too large".into(),
            Fail::Other => "Couldn't load it".into(),
        }
    }
}

fn classify(e: windows::core::Error) -> Fail {
    match e.code().0 as u32 {
        0x8007_2EE2 => Fail::Timeout,
        // The name didn't resolve, the connection failed, was aborted or reset.
        0x8007_2EE7 | 0x8007_2EFD | 0x8007_2EFE | 0x8007_2EFF | 0x8007_2EE9 => Fail::Offline,
        // Certificate problems: unknown authority, bad date, wrong name, revoked.
        0x8007_2F05 | 0x8007_2F06 | 0x8007_2F0D | 0x8007_2F0F | 0x8007_2F7D | 0x8007_2F8F => Fail::Tls,
        _ => Fail::Other,
    }
}

/// A link someone typed or pasted, as the https:// URL to fetch. webcal:// and webcals:// are how
/// calendar apps write https:// links. Anything else, and anything with spaces or control
/// characters, or no host, is refused.
pub fn https_url(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > MAX_URL || raw.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let (scheme, rest) = raw.split_once("://")?;
    if !["https", "webcal", "webcals"].iter().any(|s| scheme.eq_ignore_ascii_case(s)) {
        return None;
    }
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.is_empty() && !host.starts_with('@')).then(|| format!("https://{rest}"))
}

thread_local! {
    // WinRT wants the thread in an apartment; the multithreaded one, as we never pump messages.
    static COM: () = unsafe { crate::ffi::CoInitializeEx(std::ptr::null_mut(), crate::ffi::COINIT_MULTITHREADED); };
}

/// Make this thread fit to call Windows' runtime. Safe to call as often as you like.
pub fn init_thread() {
    COM.with(|_| {});
}

/// Waits for a WinRT operation, and cancels it if it isn't done by `deadline`.
macro_rules! wait {
    ($op:expr, $deadline:expr) => {{
        let op = $op;
        let (tx, rx) = std::sync::mpsc::channel();
        op.when(move |_| {
            let _ = tx.send(());
        })
        .map_err(classify)?;
        match rx.recv_timeout($deadline.saturating_duration_since(Instant::now())) {
            Ok(()) => op.GetResults().map_err(classify)?,
            Err(_) => {
                let _ = op.Cancel();
                return Err(Fail::Timeout);
            }
        }
    }};
}

/// GET `url` (https:// or webcal://) and return the body, if it's under `max` bytes and arrives
/// within `timeout`. `accept` is what the request says it wants, e.g. "application/json".
pub fn get(url: &str, accept: &str, max: usize, timeout: Duration) -> Result<Vec<u8>, Fail> {
    init_thread();
    let deadline = Instant::now() + timeout;
    let mut url = https_url(url).ok_or(Fail::NotHttps)?;
    let client = client(accept)?;
    for _ in 0..=MAX_REDIRECTS {
        let uri = Uri::CreateUri(&HSTRING::from(&url)).map_err(|_| Fail::NotHttps)?;
        let request = HttpRequestMessage::Create(&HttpMethod::Get().map_err(classify)?, &uri).map_err(classify)?;
        let response = wait!(client.SendRequestWithOptionAsync(&request, HttpCompletionOption::ResponseHeadersRead).map_err(classify)?, deadline);
        let status = response.StatusCode().map_err(classify)?.0;
        if matches!(status, 301 | 302 | 303 | 307 | 308) {
            // Followed by hand, so a redirect can't take us off https.
            let next = response.Headers().and_then(|h| h.Location()).map_err(classify)?;
            if !next.SchemeName().map_err(classify)?.to_string().eq_ignore_ascii_case("https") {
                return Err(Fail::NotHttps);
            }
            url = next.AbsoluteUri().map_err(classify)?.to_string();
            continue;
        }
        if status != 200 {
            return Err(Fail::Status(status as u16));
        }
        let stream = wait!(response.Content().and_then(|c| c.ReadAsInputStreamAsync()).map_err(classify)?, deadline);
        let reader = DataReader::CreateDataReader(&stream).map_err(classify)?;
        // Hand back what has arrived rather than waiting to fill a whole chunk.
        reader.SetInputStreamOptions(InputStreamOptions::Partial).map_err(classify)?;
        let mut body = Vec::new();
        loop {
            let n = wait!(reader.LoadAsync(CHUNK).map_err(classify)?, deadline) as usize;
            if n == 0 {
                return Ok(body);
            }
            if body.len() + n > max {
                return Err(Fail::TooLarge);
            }
            let at = body.len();
            body.resize(at + n, 0);
            reader.ReadBytes(&mut body[at..]).map_err(classify)?;
        }
    }
    Err(Fail::Other)
}

fn client(accept: &str) -> Result<HttpClient, Fail> {
    let filter = HttpBaseProtocolFilter::new().map_err(classify)?;
    filter.SetAllowAutoRedirect(false).map_err(classify)?;
    filter.SetAllowUI(false).map_err(classify)?; // no sign-in prompts
    filter.SetCookieUsageBehavior(HttpCookieUsageBehavior::NoCookies).map_err(classify)?;
    let client = HttpClient::Create(&filter.cast::<IHttpFilter>().map_err(classify)?).map_err(classify)?;
    let headers = client.DefaultRequestHeaders().map_err(classify)?;
    let _ = headers.TryAppendWithoutValidation(&HSTRING::from("User-Agent"), &HSTRING::from(concat!("Oriel/", env!("CARGO_PKG_VERSION"))));
    let _ = headers.TryAppendWithoutValidation(&HSTRING::from("Accept"), &HSTRING::from(accept));
    // Without this Windows asks for gzip and deflate, and then fails to decode Open-Meteo's
    // "deflate" replies (error 0x80072F8F). Plain also keeps the size limit about what's on the wire.
    let _ = headers.TryAppendWithoutValidation(&HSTRING::from("Accept-Encoding"), &HSTRING::from("identity"));
    Ok(client)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_links_are_fetched() {
        assert_eq!(https_url("https://example.com/cal.ics").as_deref(), Some("https://example.com/cal.ics"));
        assert_eq!(https_url("  HTTPS://Example.com/a?b=c#d \n").as_deref(), Some("https://Example.com/a?b=c#d"));
        assert_eq!(https_url("webcal://example.com/cal.ics").as_deref(), Some("https://example.com/cal.ics"));
        assert_eq!(https_url("WebCals://example.com/x").as_deref(), Some("https://example.com/x"));
        assert_eq!(https_url("http://example.com/cal.ics"), None);
        assert_eq!(https_url("ftp://example.com/cal.ics"), None);
        assert_eq!(https_url("file:///C:/cal.ics"), None);
        assert_eq!(https_url("javascript:alert(1)"), None);
        assert_eq!(https_url("example.com/cal.ics"), None);
        assert_eq!(https_url("https://"), None);
        assert_eq!(https_url("https:///path"), None);
        assert_eq!(https_url("https://@evil.example/"), None);
        assert_eq!(https_url("https://example.com/a b"), None);
        assert_eq!(https_url("https://example.com/a\r\nHost: x"), None);
        assert_eq!(https_url(""), None);
        assert_eq!(https_url(&format!("https://example.com/{}", "a".repeat(MAX_URL))), None);
    }

    #[test]
    fn errors_never_mention_the_link() {
        for f in [Fail::NotHttps, Fail::Offline, Fail::Timeout, Fail::Tls, Fail::Status(404), Fail::TooLarge, Fail::Other] {
            assert!(!f.text().contains("http://") && !f.text().contains("https://"));
        }
        assert_eq!(Fail::Status(503).text(), "The server said 503");
    }

    #[test]
    fn plain_http_is_refused_before_anything_is_sent() {
        assert_eq!(get("http://example.com/", "*/*", 1024, Duration::from_secs(5)), Err(Fail::NotHttps));
    }

    #[test]
    fn windows_http_client_starts() {
        // Offline: the WinRT classes activate from our own thread and take our settings. Nothing is sent.
        init_thread();
        assert!(client("application/json").is_ok());
    }

    // Goes online (Open-Meteo), so it only runs on request: cargo test -- --ignored
    #[test]
    #[ignore = "needs the internet and a third-party service"]
    fn a_fetch_comes_back_or_fails_cleanly() {
        // What a fetch must do either way: the WinRT calls work from our own thread, and the deadline holds.
        let started = Instant::now();
        let r = get("https://geocoding-api.open-meteo.com/v1/search?name=Berlin&count=1", "application/json", 256 << 10, Duration::from_secs(20));
        assert!(started.elapsed() < Duration::from_secs(25), "the deadline didn't hold");
        match r {
            Ok(body) => {
                assert!(String::from_utf8_lossy(&body).contains("Berlin"));
                // A response over the limit is refused, not truncated.
                let small = get("https://geocoding-api.open-meteo.com/v1/search?name=Berlin&count=1", "application/json", 10, Duration::from_secs(20));
                assert_eq!(small, Err(Fail::TooLarge));
            }
            Err(e) => println!("no network here: {}", e.text()),
        }
    }
}
