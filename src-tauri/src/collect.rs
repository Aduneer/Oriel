//! Builds one snapshot of the system per tick.

use crate::audio::{Audio, AudioWatch};
use crate::ffi::*;
use crate::gpu::{GpuInfo, GpuMonitor, GpuSample};
use crate::lhm::{Fan, Lhm};
use crate::media::{Media, MediaWatch};
use crate::openwin::{Win, Windows};
use crate::pdh::Query;
use crate::procs::{Proc, ProcessSampler};
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

// Virtual adapters whose traffic is also counted on a physical one.
const SKIP_NICS: &[&str] = &["hyper-v virtual", "vethernet", "isatap", "teredo", "loopback", "pseudo-interface"];
const DRIVES_EVERY: u64 = 30; // ticks between free-space checks
const CPU_KEY: &str = r"HARDWARE\DESCRIPTION\System\CentralProcessor\0";
const NT_KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

#[derive(Serialize, Clone)]
pub struct Info {
    host: String,
    os: String,
    version: String,
    cpu: CpuInfo,
    mem_total: u64,
    gpus: Vec<GpuInfo>,
    has_battery: bool,
}

#[derive(Serialize, Clone)]
struct CpuInfo {
    name: String,
    cores: usize,
    threads: usize,
    base_mhz: Option<u32>,
}

#[derive(Serialize, Clone)]
pub struct Snapshot {
    t: f64,
    cpu: Cpu,
    mem: Mem,
    gpus: Vec<GpuSample>,
    net: Net,
    disk: Disk,
    battery: Option<Battery>,
    uptime: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    procs: Option<Vec<Proc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    drives: Option<Vec<Drive>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    media: Option<Media>,
    #[serde(skip_serializing_if = "Option::is_none")]
    audio: Option<Audio>,
    #[serde(skip_serializing_if = "Option::is_none")]
    windows: Option<Vec<Win>>,
}

impl Snapshot {
    /// What the graphs need for history: drops the process list, drives, media, the mixer and the window list.
    pub fn slim(&self) -> Snapshot {
        Snapshot { procs: None, drives: None, media: None, audio: None, windows: None, ..self.clone() }
    }
}

#[derive(Serialize, Clone)]
struct Cpu {
    total: f64,
    cores: Vec<f64>,
    mhz: Option<f64>,
    temp: Option<f64>,
    temp_source: Option<&'static str>,
    power: Option<f64>,
    fans: Vec<Fan>,
}

#[derive(Serialize, Clone)]
struct Mem {
    used: u64,
    total: u64,
    pct: f64,
    commit_used: u64,
    commit_limit: u64,
}

#[derive(Serialize, Clone)]
struct Net {
    down: f64,
    up: f64,
}

#[derive(Serialize, Clone)]
struct Disk {
    read: f64,
    write: f64,
    active: Option<f64>,
}

#[derive(Serialize, Clone)]
struct Battery {
    pct: u8,
    plugged: bool,
    secs_left: Option<u32>,
}

#[derive(Serialize, Clone)]
struct Drive {
    mount: String,
    used: u64,
    total: u64,
}

pub struct Collector {
    q: Query,
    gpu: GpuMonitor,
    procs: ProcessSampler,
    lhm: Option<Lhm>,
    media: MediaWatch,
    audio: AudioWatch,
    windows: Windows,
    threads: usize,
    tick: u64,
    drives: Vec<Drive>,
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

impl Collector {
    pub fn new(use_nvml: bool, lhm_url: Option<String>, media: MediaWatch, audio: AudioWatch) -> Self {
        let mut q = Query::new().expect("PDH unavailable");
        q.add("cpu_util", r"\Processor Information(*)\% Processor Utility");
        q.add("cpu_perf", r"\Processor Information(_Total)\% Processor Performance");
        q.add("cpu_freq", r"\Processor Information(_Total)\Processor Frequency");
        q.add("disk_idle", r"\PhysicalDisk(_Total)\% Idle Time");
        q.add("disk_read", r"\PhysicalDisk(_Total)\Disk Read Bytes/sec");
        q.add("disk_write", r"\PhysicalDisk(_Total)\Disk Write Bytes/sec");
        q.add("net_down", r"\Network Interface(*)\Bytes Received/sec");
        q.add("net_up", r"\Network Interface(*)\Bytes Sent/sec");
        q.add("tz", r"\Thermal Zone Information(*)\High Precision Temperature");
        let gpu = GpuMonitor::new(&mut q, use_nvml);
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        let mut procs = ProcessSampler::new(threads);
        procs.sample();
        q.collect(); // baseline for rate counters
        Collector { q, gpu, procs, lhm: lhm_url.map(Lhm::start), media, audio, windows: Windows::default(), threads, tick: 0, drives: drives() }
    }

    pub fn info(&self) -> Info {
        let build: u32 = reg_string(NT_KEY, "CurrentBuildNumber").and_then(|b| b.parse().ok()).unwrap_or(0);
        let os = if build >= 22000 { "Windows 11" } else { "Windows 10" };
        let os = match reg_string(NT_KEY, "DisplayVersion") {
            Some(v) => format!("{os} {v}"),
            None => os.to_string(),
        };
        let mut ms = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
        unsafe { GlobalMemoryStatusEx(&mut ms) };
        Info {
            host: std::env::var("COMPUTERNAME").unwrap_or_default(),
            os,
            version: env!("CARGO_PKG_VERSION").to_string(),
            cpu: CpuInfo {
                name: reg_string(CPU_KEY, "ProcessorNameString").unwrap_or_else(|| "CPU".into()),
                cores: physical_cores(),
                threads: self.threads,
                base_mhz: reg_dword(CPU_KEY, "~MHz"),
            },
            mem_total: ms.ullTotalPhys,
            gpus: self.gpu.info(),
            has_battery: battery().is_some(),
        }
    }

    /// `live`: the dashboard is on screen, so it's worth asking the player what's on and
    /// reading the mixer.
    pub fn sample(&mut self, live: bool) -> Snapshot {
        self.q.collect();
        if self.tick > 0 && self.tick.is_multiple_of(DRIVES_EVERY) {
            self.drives = drives();
        }
        self.tick += 1;
        Snapshot {
            t: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0),
            cpu: self.cpu(),
            mem: mem(),
            gpus: self.gpu.sample(&self.q),
            net: self.net(),
            disk: self.disk(),
            battery: battery(),
            uptime: unsafe { GetTickCount64() } as f64 / 1000.0,
            procs: Some(self.procs.sample()),
            drives: Some(self.drives.clone()),
            // The read happens on the media thread, so this is the previous tick's: fine for a song.
            media: if live {
                self.media.poll();
                self.media.latest()
            } else {
                None
            },
            audio: if live {
                self.audio.poll();
                self.audio.latest()
            } else {
                None
            },
            windows: if live { self.windows.poll() } else { None },
        }
    }

    fn cpu(&self) -> Cpu {
        let util = self.q.read("cpu_util");
        let mut per: Vec<((u32, u32), f64)> = util
            .iter()
            .filter(|(n, _)| !n.contains("_Total"))
            .filter_map(|(n, v)| {
                let (g, i) = n.split_once(',')?;
                Some(((g.parse().ok()?, i.parse().ok()?), *v))
            })
            .collect();
        per.sort_by_key(|(k, _)| *k);
        let cores: Vec<f64> = per.iter().map(|(_, v)| round1(v.min(100.0))).collect();
        let total = util
            .iter()
            .find(|(n, _)| n == "_Total")
            .map(|(_, v)| *v)
            .unwrap_or_else(|| cores.iter().sum::<f64>() / cores.len().max(1) as f64)
            .min(100.0);

        let mhz = match (self.q.total("cpu_perf"), self.q.total("cpu_freq")) {
            (Some(perf), Some(base)) if perf > 0.0 => Some((base * perf / 100.0).round()),
            _ => None,
        };

        let sensors = self.lhm.as_ref().and_then(Lhm::read);
        let (mut temp, mut source) = (None, None);
        if let Some(t) = sensors.as_ref().and_then(|s| s.cpu_temp) {
            (temp, source) = (Some(t), Some("lhm"));
        } else {
            // ACPI thermal zone (tenths of a kelvin): not the die sensor, but close on many laptops.
            let zone = self.q.read("tz").iter().map(|(_, v)| v / 10.0 - 273.15).filter(|c| *c > 0.0 && *c < 125.0).reduce(f64::max);
            if zone.is_some() {
                (temp, source) = (zone, Some("acpi"));
            }
        }
        Cpu {
            total: round1(total),
            cores,
            mhz,
            temp: temp.map(round1),
            temp_source: source,
            power: sensors.as_ref().and_then(|s| s.cpu_power).map(round1),
            fans: sensors.map(|s| s.fans).unwrap_or_default(),
        }
    }

    fn net(&self) -> Net {
        let total = |key| {
            self.q
                .read(key)
                .iter()
                .filter(|(n, _)| {
                    let n = n.to_lowercase();
                    !SKIP_NICS.iter().any(|s| n.contains(s))
                })
                .map(|(_, v)| v)
                .sum()
        };
        Net { down: total("net_down"), up: total("net_up") }
    }

    fn disk(&self) -> Disk {
        Disk {
            read: self.q.total("disk_read").unwrap_or(0.0),
            write: self.q.total("disk_write").unwrap_or(0.0),
            active: self.q.total("disk_idle").map(|idle| round1((100.0 - idle).max(0.0))),
        }
    }
}

fn mem() -> Mem {
    let mut ms = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    let mut pi = PERFORMANCE_INFORMATION { cb: std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32, ..Default::default() };
    unsafe {
        GlobalMemoryStatusEx(&mut ms);
        K32GetPerformanceInfo(&mut pi, pi.cb);
    }
    let used = ms.ullTotalPhys.saturating_sub(ms.ullAvailPhys);
    Mem {
        used,
        total: ms.ullTotalPhys,
        pct: round1(used as f64 / ms.ullTotalPhys.max(1) as f64 * 100.0),
        commit_used: (pi.CommitTotal * pi.PageSize) as u64,
        commit_limit: (pi.CommitLimit * pi.PageSize) as u64,
    }
}

fn battery() -> Option<Battery> {
    let mut s = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut s) } == 0 || s.BatteryFlag & 128 != 0 || s.BatteryFlag == 255 || s.BatteryLifePercent > 100 {
        return None;
    }
    Some(Battery {
        pct: s.BatteryLifePercent,
        plugged: s.ACLineStatus == 1,
        secs_left: (s.BatteryLifeTime != u32::MAX).then_some(s.BatteryLifeTime),
    })
}

fn drives() -> Vec<Drive> {
    let mut buf = [0u16; 512];
    let n = unsafe { GetLogicalDriveStringsW(buf.len() as u32, buf.as_mut_ptr()) } as usize;
    buf[..n.min(buf.len())]
        .split(|&c| c == 0)
        .filter(|root| !root.is_empty())
        .filter_map(|root| {
            let mut z = root.to_vec();
            z.push(0);
            if unsafe { GetDriveTypeW(z.as_ptr()) } != DRIVE_FIXED {
                return None;
            }
            let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
            if unsafe { GetDiskFreeSpaceExW(z.as_ptr(), &mut avail, &mut total, &mut free) } == 0 {
                return None;
            }
            Some(Drive { mount: String::from_utf16_lossy(root).trim_end_matches('\\').to_string(), used: total - free, total })
        })
        .collect()
}
