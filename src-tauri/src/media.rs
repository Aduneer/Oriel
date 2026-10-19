//! What's playing, from Windows' media controls: the session the volume flyout and the lock
//! screen show, so it covers Spotify, browsers, Media Player and most other players. Plus
//! play/pause, next and previous for it.
//!
//! These calls go through the player and can be slow, so they run on a thread of their own and
//! never hold up the sampler. The sampler asks for a fresh read each tick, but only while the
//! dashboard is on screen and the widget is on one of its pages; otherwise the thread just waits.

use base64::Engine;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows::Foundation::TimeSpan;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Storage::Streams::{DataReader, IRandomAccessStreamReference};

#[derive(Serialize, Clone, Default, Debug)]
pub struct Media {
    /// The player as Windows identifies it: "Spotify.exe", or a Store app's id. The page makes it readable.
    app: String,
    title: String,
    artist: String,
    album: String,
    playing: bool,
    /// Seconds into the track as of `at` (Unix time); the page moves it along while playing.
    /// None when the player doesn't say (some browser tabs, radio streams).
    position: Option<f64>,
    duration: Option<f64>,
    at: f64,
    can_prev: bool,
    can_next: bool,
    can_toggle: bool,
    /// Changes when the cover does, 0 for none. The page then fetches it with `media_art`, so a
    /// picture isn't sent with every sample.
    art: u32,
}

pub enum Action {
    Toggle,
    Next,
    Previous,
}

enum Msg {
    Read,
    Do(Action),
}

/// The handle the sampler and the commands share. Cheap to clone.
#[derive(Clone)]
pub struct MediaWatch {
    tx: Sender<Msg>,
    latest: Arc<Mutex<Option<Media>>>,
    art: Arc<Mutex<Option<String>>>,
    wanted: Arc<AtomicBool>,
    reading: Arc<AtomicBool>,
}

impl MediaWatch {
    pub fn start() -> Self {
        let (tx, rx) = channel();
        let w = MediaWatch {
            tx,
            latest: Default::default(),
            art: Default::default(),
            wanted: Default::default(),
            reading: Default::default(),
        };
        let thread = w.clone();
        let _ = std::thread::Builder::new().name("media".into()).spawn(move || thread.run(rx));
        w
    }

    /// Whether the widget is on the dashboard. Without it, nothing is read at all.
    pub fn set_wanted(&self, on: bool) {
        self.wanted.store(on, Ordering::Relaxed);
        if !on {
            *self.latest.lock().unwrap() = None;
            *self.art.lock().unwrap() = None;
        }
    }

    /// Ask for a fresh read; the result shows up in `latest`. Skipped while a read is still going.
    pub fn poll(&self) {
        if self.wanted.load(Ordering::Relaxed) && !self.reading.swap(true, Ordering::Relaxed) {
            let _ = self.tx.send(Msg::Read);
        }
    }

    pub fn latest(&self) -> Option<Media> {
        self.latest.lock().unwrap().clone()
    }

    /// The current cover as a data: URL (PNG, JPEG, GIF, WebP or BMP only).
    pub fn art(&self) -> Option<String> {
        self.art.lock().unwrap().clone()
    }

    pub fn control(&self, action: Action) {
        let _ = self.tx.send(Msg::Do(action));
    }

    fn run(self, rx: Receiver<Msg>) {
        // WinRT wants the thread in an apartment; the multithreaded one, as we never pump messages.
        unsafe { crate::ffi::CoInitializeEx(std::ptr::null_mut(), crate::ffi::COINIT_MULTITHREADED) };
        let mut reader = Reader::default();
        for msg in rx {
            if let Msg::Do(action) = msg {
                reader.act(action);
                // Give the player a moment, so the read below already shows the change.
                std::thread::sleep(Duration::from_millis(150));
            }
            let media = if self.wanted.load(Ordering::Relaxed) { reader.read(&self.art) } else { None };
            *self.latest.lock().unwrap() = media;
            self.reading.store(false, Ordering::Relaxed);
        }
    }
}

#[derive(Default)]
struct Reader {
    manager: Option<Manager>,
    /// The track the cover belongs to, a counter for the page, and tries left to fetch it
    /// (players often send the cover a moment after the title).
    track: String,
    art: u32,
    art_tries: u8,
}

impl Reader {
    fn session(&mut self) -> Option<Session> {
        if self.manager.is_none() {
            self.manager = Manager::RequestAsync().and_then(|op| op.join()).ok();
        }
        // An error here just means nothing is playing.
        self.manager.as_ref()?.GetCurrentSession().ok()
    }

    fn act(&mut self, action: Action) {
        let Some(s) = self.session() else { return };
        let op = match action {
            Action::Toggle => s.TryTogglePlayPauseAsync(),
            Action::Next => s.TrySkipNextAsync(),
            Action::Previous => s.TrySkipPreviousAsync(),
        };
        let _ = op.and_then(|op| op.join());
    }

    fn read(&mut self, art: &Mutex<Option<String>>) -> Option<Media> {
        let session = self.session()?;
        match self.read_session(&session, art) {
            Ok(m) => Some(m),
            Err(_) => {
                // The media service may have restarted; start over next time.
                self.manager = None;
                None
            }
        }
    }

    fn read_session(&mut self, s: &Session, art: &Mutex<Option<String>>) -> windows::core::Result<Media> {
        let props = s.TryGetMediaPropertiesAsync()?.join()?;
        let info = s.GetPlaybackInfo()?;
        let controls = info.Controls()?;
        let time = s.GetTimelineProperties()?;
        let secs = |t: TimeSpan| t.Duration as f64 / 1e7;
        let (start, end, pos) = (secs(time.StartTime()?), secs(time.EndTime()?), secs(time.Position()?));
        let timed = end > start;
        let updated = time.LastUpdatedTime()?.UniversalTime;
        let mut m = Media {
            app: s.SourceAppUserModelId()?.to_string(),
            title: props.Title()?.to_string(),
            artist: props.Artist()?.to_string(),
            album: props.AlbumTitle()?.to_string(),
            playing: info.PlaybackStatus()? == Status::Playing,
            position: timed.then(|| (pos - start).clamp(0.0, end - start)),
            duration: timed.then_some(end - start),
            at: if updated > 0 { unix(updated) } else { now() },
            can_prev: controls.IsPreviousEnabled()?,
            can_next: controls.IsNextEnabled()?,
            can_toggle: controls.IsPlayPauseToggleEnabled()?,
            art: 0,
        };

        let track = format!("{}\n{}\n{}\n{}", m.app, m.title, m.artist, m.album);
        if track != self.track {
            self.track = track;
            self.art_tries = 5;
            *art.lock().unwrap() = None;
        }
        if self.art_tries > 0 && art.lock().unwrap().is_none() {
            self.art_tries -= 1;
            if let Some(url) = props.Thumbnail().ok().and_then(|t| cover(&t)) {
                *art.lock().unwrap() = Some(url);
                self.art = self.art.wrapping_add(1).max(1);
                self.art_tries = 0;
            }
        }
        if art.lock().unwrap().is_some() {
            m.art = self.art;
        }
        Ok(m)
    }
}

/// The cover's bytes as a data: URL, if it's an image type a page can show.
fn cover(r: &IRandomAccessStreamReference) -> Option<String> {
    let stream = r.OpenReadAsync().ok()?.join().ok()?;
    let size = stream.Size().ok()?;
    // Covers are tens to hundreds of KB; anything far bigger isn't worth sending to the page.
    if size == 0 || size > 4 << 20 {
        return None;
    }
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    let n = reader.LoadAsync(size as u32).ok()?.join().ok()?;
    let mut buf = vec![0u8; n as usize];
    reader.ReadBytes(&mut buf).ok()?;
    data_url(&buf)
}

fn data_url(bytes: &[u8]) -> Option<String> {
    // Trust the bytes, not whatever content type the player claims.
    let mime = match bytes {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [0xFF, 0xD8, 0xFF, ..] => "image/jpeg",
        [b'G', b'I', b'F', b'8', ..] => "image/gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        [b'B', b'M', ..] => "image/bmp",
        _ => return None,
    };
    Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// Windows time (100 ns ticks since 1601) to Unix seconds.
fn unix(ticks: i64) -> f64 {
    (ticks - 116_444_736_000_000_000) as f64 / 1e7
}

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_are_images_only() {
        assert_eq!(data_url(b"\x89PNG\r\n\x1a\n").as_deref(), Some("data:image/png;base64,iVBORw0KGgo="));
        assert!(data_url(b"\xFF\xD8\xFF\xE0").unwrap().starts_with("data:image/jpeg;base64,"));
        assert!(data_url(b"RIFF\0\0\0\0WEBPVP8 ").unwrap().starts_with("data:image/webp;"));
        assert_eq!(data_url(b"<svg onload=alert(1)>"), None);
        assert_eq!(data_url(b""), None);
    }

    #[test]
    fn windows_time_to_unix() {
        assert_eq!(unix(116_444_736_000_000_000), 0.0);
        assert_eq!(unix(116_444_736_000_000_000 + 15_000_000), 1.5);
    }

    #[test]
    fn reads_whatever_is_playing() {
        // Whatever this PC is playing, if anything (CI machines play nothing). The point is that
        // the WinRT calls work from our own thread and come back.
        let w = MediaWatch::start();
        w.set_wanted(true);
        w.poll();
        for _ in 0..50 {
            std::thread::sleep(Duration::from_millis(100));
            if !w.reading.load(Ordering::Relaxed) {
                break;
            }
        }
        assert!(!w.reading.load(Ordering::Relaxed), "the read never finished");
        match w.latest() {
            Some(m) => println!("playing={} timed={} art={} from {}", m.playing, m.duration.is_some(), m.art != 0, m.app),
            None => println!("nothing playing"),
        }
    }
}
