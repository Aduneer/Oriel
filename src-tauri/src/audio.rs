//! The volume mixer: the default output's volume and every app playing through it, as Windows'
//! own mixer shows them, and setting either.
//!
//! Like Now playing, this runs on a thread of its own and only reads while the dashboard is on
//! screen and the widget is in the layout.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use windows::core::{Interface, Result, GUID, PWSTR};
use windows::Win32::Foundation::{PROPERTYKEY, S_OK};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eConsole, eRender, AudioSessionStateActive, AudioSessionStateExpired, IAudioSessionControl2, IAudioSessionManager2, IMMDevice,
    IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantClear;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL, STGM_READ};
use windows::Win32::System::Variant::VT_LPWSTR;

/// PKEY_Device_FriendlyName: "Speakers (Realtek(R) Audio)" and the like.
const FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY { fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0), pid: 14 };

#[derive(Serialize, Clone, Default, Debug)]
pub struct Audio {
    /// The default output, as Windows names it.
    device: String,
    /// 0–100, as the volume flyout shows it.
    volume: u8,
    muted: bool,
    apps: Vec<App>,
}

#[derive(Serialize, Clone, Default, Debug, PartialEq)]
pub struct App {
    /// 0 for Windows' system sounds.
    pid: u32,
    /// The program's file name without ".exe"; the page makes it readable.
    exe: String,
    /// The name an app gives its audio, which most leave to Windows.
    title: String,
    /// 0–100 of the device's volume, as Windows' own mixer shows it.
    volume: u8,
    muted: bool,
    /// Playing right now, rather than just holding the device open.
    active: bool,
}

#[derive(Clone, Copy)]
pub enum Change {
    /// 0.0–1.0
    Volume(f32),
    Mute(bool),
}

enum Msg {
    Read,
    /// None for the device itself, else an app (see `App::pid`).
    Set(Option<u32>, Change),
}

/// The handle the sampler and the commands share. Cheap to clone.
#[derive(Clone)]
pub struct AudioWatch {
    tx: Sender<Msg>,
    latest: Arc<Mutex<Option<Audio>>>,
    wanted: Arc<AtomicBool>,
    reading: Arc<AtomicBool>,
}

impl AudioWatch {
    pub fn start() -> Self {
        let (tx, rx) = channel();
        let w = AudioWatch { tx, latest: Default::default(), wanted: Default::default(), reading: Default::default() };
        let thread = w.clone();
        let _ = std::thread::Builder::new().name("audio".into()).spawn(move || thread.run(rx));
        w
    }

    /// Whether the widget is on the dashboard. Without it, nothing is read at all.
    pub fn set_wanted(&self, on: bool) {
        self.wanted.store(on, Ordering::Relaxed);
        if !on {
            *self.latest.lock().unwrap() = None;
        }
    }

    /// Ask for a fresh read; the result shows up in `latest`. Skipped while a read is still going.
    pub fn poll(&self) {
        if self.wanted.load(Ordering::Relaxed) && !self.reading.swap(true, Ordering::Relaxed) {
            let _ = self.tx.send(Msg::Read);
        }
    }

    /// None when there's no output device (or the widget isn't on the dashboard).
    pub fn latest(&self) -> Option<Audio> {
        self.latest.lock().unwrap().clone()
    }

    /// Change the device's volume (`pid` None) or an app's.
    pub fn set(&self, pid: Option<u32>, change: Change) {
        let _ = self.tx.send(Msg::Set(pid, change));
    }

    fn run(self, rx: Receiver<Msg>) {
        // Core Audio is COM; the multithreaded apartment, as we never pump messages.
        unsafe { crate::ffi::CoInitializeEx(std::ptr::null_mut(), crate::ffi::COINIT_MULTITHREADED) };
        let mut mixer = Mixer::default();
        while let Ok(mut msg) = rx.recv() {
            // Dragging a slider sends a stream of changes: make all that have queued up, then read once.
            loop {
                if let Msg::Set(pid, change) = msg {
                    mixer.set(pid, change);
                }
                match rx.try_recv() {
                    Ok(next) => msg = next,
                    Err(_) => break,
                }
            }
            let audio = if self.wanted.load(Ordering::Relaxed) { mixer.read() } else { None };
            *self.latest.lock().unwrap() = audio;
            self.reading.store(false, Ordering::Relaxed);
        }
    }
}

#[derive(Default)]
struct Mixer {
    devices: Option<IMMDeviceEnumerator>,
    output: Option<Output>,
    /// Program names by process id, so each is looked up once.
    exes: HashMap<u32, String>,
}

/// The default output device, kept until Windows switches to another one.
struct Output {
    id: String,
    name: String,
    volume: IAudioEndpointVolume,
    sessions: IAudioSessionManager2,
}

impl Mixer {
    /// Make sure `output` is the current default; it changes when headphones are plugged in, say.
    /// Fails when there's no output device at all.
    fn open(&mut self) -> Result<()> {
        unsafe {
            if self.devices.is_none() {
                self.devices = Some(CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?);
            }
            let device = self.devices.as_ref().unwrap().GetDefaultAudioEndpoint(eRender, eConsole)?;
            let id = take(device.GetId()?);
            if self.output.as_ref().is_none_or(|o| o.id != id) {
                self.output = Some(Output {
                    name: friendly_name(&device).unwrap_or_default(),
                    volume: device.Activate(CLSCTX_ALL, None)?,
                    sessions: device.Activate(CLSCTX_ALL, None)?,
                    id,
                });
            }
        }
        Ok(())
    }

    fn read(&mut self) -> Option<Audio> {
        match self.try_read() {
            Ok(audio) => Some(audio),
            Err(_) => {
                // No output device, or the audio service restarted: start over next time.
                *self = Mixer::default();
                None
            }
        }
    }

    fn try_read(&mut self) -> Result<Audio> {
        self.open()?;
        let out = self.output.as_ref().unwrap();
        let mut apps = Vec::new();
        for s in sessions(&out.sessions)? {
            // One that closed while we looked is simply left out.
            if let Ok(app) = app(&s, &mut self.exes) {
                merge(&mut apps, app);
            }
        }
        self.exes.retain(|pid, _| apps.iter().any(|a| a.pid == *pid));
        unsafe {
            Ok(Audio {
                device: out.name.clone(),
                volume: pct(out.volume.GetMasterVolumeLevelScalar()?),
                muted: out.volume.GetMute()?.as_bool(),
                apps,
            })
        }
    }

    fn set(&mut self, pid: Option<u32>, change: Change) {
        // A failure shows in the next read, which starts over.
        let _ = self.try_set(pid, change);
    }

    fn try_set(&mut self, pid: Option<u32>, change: Change) -> Result<()> {
        self.open()?;
        let out = self.output.as_ref().unwrap();
        let no_context = std::ptr::null();
        unsafe {
            let Some(pid) = pid else {
                return match change {
                    Change::Volume(v) => out.volume.SetMasterVolumeLevelScalar(v, no_context),
                    Change::Mute(m) => out.volume.SetMute(m, no_context),
                };
            };
            // All of the app's sessions, so they stay one row.
            for s in sessions(&out.sessions)? {
                if owner(&s).ok() != Some(pid) {
                    continue;
                }
                let vol: ISimpleAudioVolume = s.cast()?;
                match change {
                    Change::Volume(v) => vol.SetMasterVolume(v, no_context)?,
                    Change::Mute(m) => vol.SetMute(m, no_context)?,
                }
            }
        }
        Ok(())
    }
}

/// The output's sessions that are still open: one or more for each app that has opened it.
fn sessions(manager: &IAudioSessionManager2) -> Result<Vec<IAudioSessionControl2>> {
    unsafe {
        let list = manager.GetSessionEnumerator()?;
        Ok((0..list.GetCount()?)
            .filter_map(|i| list.GetSession(i).ok())
            .filter(|s| s.GetState().is_ok_and(|state| state != AudioSessionStateExpired))
            .filter_map(|s| s.cast().ok())
            .collect())
    }
}

/// The app a session belongs to: its process, or 0 for system sounds.
fn owner(s: &IAudioSessionControl2) -> Result<u32> {
    unsafe {
        if s.IsSystemSoundsSession() == S_OK {
            Ok(0)
        } else {
            s.GetProcessId()
        }
    }
}

/// One session as a mixer row.
fn app(s: &IAudioSessionControl2, exes: &mut HashMap<u32, String>) -> Result<App> {
    let pid = owner(s)?;
    unsafe {
        let vol: ISimpleAudioVolume = s.cast()?;
        let title = take(s.GetDisplayName()?);
        Ok(App {
            pid,
            exe: if pid == 0 { String::new() } else { exes.entry(pid).or_insert_with(|| exe_name(pid)).clone() },
            // Windows' own sessions name themselves with a resource reference ("@%SystemRoot%\…").
            title: if title.starts_with('@') { String::new() } else { title },
            volume: pct(vol.GetMasterVolume()?),
            muted: vol.GetMute()?.as_bool(),
            active: s.GetState()? == AudioSessionStateActive,
        })
    }
}

/// Apps can hold several sessions (a browser, one per tab that played something); show one row.
fn merge(apps: &mut Vec<App>, app: App) {
    match apps.iter_mut().find(|a| a.pid == app.pid) {
        Some(a) => {
            a.active |= app.active;
            a.muted &= app.muted;
            a.volume = a.volume.max(app.volume);
            if a.title.is_empty() {
                a.title = app.title;
            }
        }
        None => apps.push(app),
    }
}

fn friendly_name(device: &IMMDevice) -> Option<String> {
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let mut value = store.GetValue(&FRIENDLY_NAME).ok()?;
        let v = &value.Anonymous.Anonymous;
        let name = (v.vt == VT_LPWSTR).then(|| v.Anonymous.pwszVal.to_string().unwrap_or_default());
        let _ = PropVariantClear(&mut value);
        name
    }
}

/// "C:\…\Spotify.exe" -> "Spotify". Empty when Windows won't say (another user's process).
pub fn exe_name(pid: u32) -> String {
    use crate::ffi::{CloseHandle, OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION};
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return String::new();
        }
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len) != 0;
        CloseHandle(h);
        ok
    };
    if !ok {
        return String::new();
    }
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// A string Windows allocated for us: copy it out, then free it.
unsafe fn take(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    let s = p.to_string().unwrap_or_default();
    CoTaskMemFree(Some(p.0 as *const _));
    s
}

/// Windows' 0.0–1.0 to the 0–100 everyone sees.
fn pct(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 100.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volumes_read_as_windows_shows_them() {
        assert_eq!(pct(0.0), 0);
        assert_eq!(pct(0.333), 33);
        assert_eq!(pct(0.666), 67);
        assert_eq!(pct(1.0), 100);
        assert_eq!(pct(1.5), 100);
        assert_eq!(pct(-0.1), 0);
    }

    #[test]
    fn an_app_with_several_sessions_is_one_row() {
        let row = |pid, volume, muted, active, title: &str| App { pid, exe: "chrome".into(), title: title.into(), volume, muted, active };
        let mut apps = Vec::new();
        merge(&mut apps, row(7, 40, true, false, ""));
        merge(&mut apps, row(9, 100, false, true, ""));
        merge(&mut apps, row(7, 60, false, true, "Meet"));
        assert_eq!(apps, vec![row(7, 60, false, true, "Meet"), row(9, 100, false, true, "")]);
        // Muted only if every session is.
        merge(&mut apps, row(9, 100, true, false, ""));
        assert!(!apps[1].muted);
    }

    #[test]
    fn exe_names_without_the_path() {
        let me = exe_name(std::process::id());
        assert!(!me.is_empty() && !me.contains('\\') && !me.to_lowercase().ends_with(".exe"), "{me}");
        assert_eq!(exe_name(u32::MAX - 3), "");
    }

    #[test]
    fn reads_the_default_output() {
        // Whatever this PC has; CI machines have no sound card, so None is fine there. The point
        // is that the COM calls work from our own thread and come back.
        let w = AudioWatch::start();
        w.set_wanted(true);
        w.poll();
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if !w.reading.load(Ordering::Relaxed) {
                break;
            }
        }
        assert!(!w.reading.load(Ordering::Relaxed), "the read never finished");
        match w.latest() {
            Some(a) => println!("an output at {}%{}, {} app(s)", a.volume, if a.muted { " (muted)" } else { "" }, a.apps.len()),
            None => println!("no output device"),
        }
    }
}
