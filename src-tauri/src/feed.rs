//! The schedule behind a widget that goes online: when to fetch, what to keep when a fetch fails,
//! and what the page is told.
//!
//! Like Now playing, each runs on a thread of its own, and only does anything while the dashboard
//! is on screen and the widget is on one of its pages. Without that it never touches the network.
//! The last good result is kept in memory, so a failed fetch leaves the widget showing it, marked
//! as stale. Nothing is written to disk.

use serde::Serialize;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// After a failed fetch, try again after this long (or the usual interval, if that's shorter).
const RETRY: Duration = Duration::from_secs(5 * 60);
/// Nobody can refresh by hand more often than this.
const REFRESH_GAP: Duration = Duration::from_secs(10);

/// What the dashboard is told each tick; when `rev` changes it asks for the data itself.
#[derive(Serialize, Clone, Default, Debug, PartialEq)]
pub struct Status {
    /// Goes up whenever there's something new to show.
    pub rev: u32,
    /// When the data last came in whole (Unix seconds), or 0 for never.
    pub at: f64,
    /// Why the last fetch failed, if it did: the data shown is older than it should be.
    pub error: Option<String>,
    /// A fetch is going on right now.
    pub busy: bool,
}

enum Msg {
    Fetch,
    Rebuild,
}

struct State<C, D> {
    config: Option<C>,
    /// Counts config changes, so a fetch that was under way for the old one is thrown away.
    gen: u64,
    data: Option<Arc<D>>,
    status: Status,
    last_try: Option<Instant>,
    last_build: Instant,
}

struct Shared<C, D> {
    state: Mutex<State<C, D>>,
    every: Duration,
    rebuild_every: Duration,
}

/// The handle the sampler and the commands share. Cheap to clone.
pub struct Watch<C, D> {
    shared: Arc<Shared<C, D>>,
    tx: Sender<Msg>,
}

impl<C, D> Clone for Watch<C, D> {
    fn clone(&self) -> Self {
        Watch { shared: self.shared.clone(), tx: self.tx.clone() }
    }
}

impl<C: Clone + PartialEq + Send + 'static, D: Send + Sync + 'static> Watch<C, D> {
    /// `fetch` gets the config and what was shown before, and returns the new data (None to keep
    /// what there was) and an error for the status. `rebuild` runs every `rebuild_every`
    /// without fetching, for data that goes out of date by itself (a calendar's "next event").
    pub fn start<F, R>(name: &str, every: Duration, rebuild_every: Duration, fetch: F, rebuild: R) -> Self
    where
        F: Fn(&C, Option<&D>) -> (Option<D>, Option<String>) + Send + 'static,
        R: Fn(&C, &D) -> Option<D> + Send + 'static,
    {
        let (tx, rx) = channel();
        let shared = Arc::new(Shared {
            state: Mutex::new(State { config: None, gen: 0, data: None, status: Status::default(), last_try: None, last_build: Instant::now() }),
            every,
            rebuild_every,
        });
        let w = Watch { shared: shared.clone(), tx };
        let _ = std::thread::Builder::new().name(name.into()).spawn(move || {
            crate::net::init_thread();
            for msg in rx {
                let (config, gen, prev) = {
                    let s = shared.state.lock().unwrap();
                    (s.config.clone(), s.gen, s.data.clone())
                };
                let fresh = config.map(|config| match msg {
                    Msg::Fetch => fetch(&config, prev.as_deref()),
                    Msg::Rebuild => (prev.as_deref().and_then(|d| rebuild(&config, d)), None),
                });
                let mut s = shared.state.lock().unwrap();
                if let (Some((data, error)), true) = (fresh, s.gen == gen) {
                    let fetched = matches!(msg, Msg::Fetch);
                    if let Some(d) = data {
                        s.data = Some(Arc::new(d));
                        s.status.rev = s.status.rev.wrapping_add(1);
                        if fetched && error.is_none() {
                            s.status.at = now();
                        }
                    }
                    if fetched {
                        if error != s.status.error {
                            s.status.rev = s.status.rev.wrapping_add(1);
                        }
                        s.status.error = error;
                    }
                    s.last_build = Instant::now();
                }
                s.status.busy = false;
            }
        });
        w
    }

    /// What the widget needs: Some while it's on a dashboard page, None when it isn't. A
    /// different config than before throws away what was fetched for the old one.
    pub fn set_config(&self, config: Option<C>) {
        let mut s = self.shared.state.lock().unwrap();
        if s.config == config {
            return;
        }
        s.config = config;
        s.gen += 1;
        s.data = None;
        s.last_try = None;
        s.status = Status { rev: s.status.rev.wrapping_add(1), ..Status::default() };
    }

    pub fn wanted(&self) -> bool {
        self.shared.state.lock().unwrap().config.is_some()
    }

    /// Called each tick while the dashboard is on screen. Starts a fetch when one is due.
    pub fn poll(&self) {
        let mut s = self.shared.state.lock().unwrap();
        if s.config.is_none() || s.status.busy {
            return;
        }
        let now = Instant::now();
        let wait = if s.status.error.is_some() { RETRY.min(self.shared.every) } else { self.shared.every };
        let msg = if s.last_try.is_none_or(|t| now.duration_since(t) >= wait) {
            s.last_try = Some(now);
            Msg::Fetch
        } else if s.data.is_some() && now.duration_since(s.last_build) >= self.shared.rebuild_every {
            Msg::Rebuild
        } else {
            return;
        };
        s.status.busy = true;
        let _ = self.tx.send(msg);
    }

    /// The refresh button: fetch again at the next tick, unless it just did.
    pub fn refresh(&self) {
        let mut s = self.shared.state.lock().unwrap();
        if s.config.is_some() && s.last_try.is_none_or(|t| t.elapsed() >= REFRESH_GAP) {
            s.last_try = None;
        }
    }

    pub fn status(&self) -> Status {
        self.shared.state.lock().unwrap().status.clone()
    }

    pub fn data(&self) -> Option<Arc<D>> {
        self.shared.state.lock().unwrap().data.clone()
    }
}

pub fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A watch whose "fetch" counts its calls and answers with the config, or fails when it's "bad".
    fn counting(every: Duration) -> (Watch<String, String>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let w = Watch::start(
            "test-feed",
            every,
            Duration::from_secs(3600),
            move |cfg: &String, _prev| {
                let n = c.fetch_add(1, Ordering::SeqCst) + 1;
                if cfg == "bad" || (cfg == "flaky" && n >= 2) {
                    (None, Some("Can't connect".to_string()))
                } else {
                    (Some(format!("data for {cfg}")), None)
                }
            },
            |_, _| None,
        );
        (w, calls)
    }

    fn settle(w: &Watch<String, String>) {
        for _ in 0..200 {
            std::thread::sleep(Duration::from_millis(10));
            if !w.status().busy {
                return;
            }
        }
        panic!("the fetch never finished");
    }

    #[test]
    fn does_nothing_until_wanted() {
        let (w, calls) = counting(Duration::from_secs(1800));
        w.poll();
        w.poll();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(!w.wanted());
        assert!(w.data().is_none());
    }

    #[test]
    fn fetches_once_then_waits_for_the_interval() {
        let (w, calls) = counting(Duration::from_secs(1800));
        w.set_config(Some("berlin".into()));
        w.poll();
        w.poll(); // busy, or not due: no second fetch either way
        settle(&w);
        w.poll();
        settle(&w);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(w.data().as_deref().map(String::as_str), Some("data for berlin"));
        let s = w.status();
        assert!(s.at > 1.0e9 && s.error.is_none() && s.rev >= 1);
    }

    #[test]
    fn a_failed_fetch_keeps_what_was_there() {
        let (w, calls) = counting(Duration::from_millis(1));
        w.set_config(Some("flaky".into())); // fetches fine once, then fails
        w.poll();
        settle(&w);
        let good = w.status();
        assert!(good.error.is_none() && good.at > 0.0);

        std::thread::sleep(Duration::from_millis(5));
        w.poll();
        settle(&w);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let stale = w.status();
        assert_eq!(stale.error.as_deref(), Some("Can't connect"));
        assert_eq!(stale.at, good.at, "a failure isn't an update");
        assert_ne!(stale.rev, good.rev, "the page is told, so it can mark the data stale");
        assert_eq!(w.data().as_deref().map(String::as_str), Some("data for flaky"));

        // And a config that never worked has nothing to show.
        let (bad, _) = counting(Duration::from_millis(1));
        bad.set_config(Some("bad".into()));
        bad.poll();
        settle(&bad);
        assert!(bad.data().is_none());
        assert_eq!(bad.status().error.as_deref(), Some("Can't connect"));
        assert_eq!(bad.status().at, 0.0);
    }

    #[test]
    fn changing_the_config_starts_over() {
        let (w, calls) = counting(Duration::from_secs(1800));
        w.set_config(Some("berlin".into()));
        w.poll();
        settle(&w);
        let rev = w.status().rev;
        w.set_config(Some("berlin".into())); // the same: nothing happens
        assert_eq!(w.status().rev, rev);
        assert!(w.data().is_some());

        w.set_config(Some("lima".into()));
        assert!(w.data().is_none());
        assert_ne!(w.status().rev, rev);
        w.poll();
        settle(&w);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(w.data().as_deref().map(String::as_str), Some("data for lima"));

        w.set_config(None);
        assert!(!w.wanted() && w.data().is_none());
        w.poll();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn refresh_asks_for_another_fetch() {
        let (w, calls) = counting(Duration::from_secs(1800));
        w.set_config(Some("berlin".into()));
        w.poll();
        settle(&w);
        w.refresh(); // just fetched: too soon
        w.poll();
        settle(&w);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
