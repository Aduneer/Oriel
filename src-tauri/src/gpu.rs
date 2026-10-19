//! GPUs from any vendor.
//!
//! Load and memory come from the "GPU Engine" / "GPU Adapter Memory" counters
//! (vendor-neutral, what Task Manager shows). Names come from DXGI. On NVIDIA,
//! NVML adds temperature, power, clocks and fan, but only while the GPU is
//! already awake: on Optimus laptops any NVML call wakes a sleeping dGPU.

use crate::pdh::Query;
use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};
use nvml_wrapper::Nvml;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

struct Adapter {
    luid: String,
    name: String,
    vendor: &'static str,
    dedicated: u64,
    shared: u64,
    integrated: bool,
}

#[derive(Serialize, Clone)]
pub struct GpuInfo {
    id: String,
    name: String,
    vendor: String,
    integrated: bool,
    vram_total: u64,
    shared_total: u64,
    sensors: bool,
}

#[derive(Serialize, Clone, Default)]
pub struct GpuSample {
    id: String,
    util: Option<f64>,
    vram_used: Option<f64>,
    shared_used: Option<f64>,
    asleep: bool,
    temp: Option<u32>,
    power: Option<f64>,
    power_limit: Option<f64>,
    clock: Option<u32>,
    mem_clock: Option<u32>,
    fan: Option<u32>,
}

fn vendor(id: u32) -> &'static str {
    match id {
        0x10DE => "NVIDIA",
        0x1002 | 0x1022 => "AMD",
        0x8086 => "Intel",
        0x5143 => "Qualcomm",
        _ => "Other",
    }
}

fn list_adapters() -> Vec<Adapter> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};
    let mut out: Vec<Adapter> = Vec::new();
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else { return out };
        let mut i = 0;
        while let Ok(adapter) = factory.EnumAdapters1(i) {
            i += 1;
            let Ok(desc) = adapter.GetDesc1() else { continue };
            if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue; // Microsoft Basic Render Driver
            }
            let luid = format!("0x{:08x}_0x{:08x}", desc.AdapterLuid.HighPart as u32, desc.AdapterLuid.LowPart);
            if out.iter().any(|a| a.luid == luid) {
                continue;
            }
            let dedicated = desc.DedicatedVideoMemory as u64;
            out.push(Adapter {
                luid,
                name: crate::ffi::from_wide(&desc.Description).trim().to_string(),
                vendor: vendor(desc.VendorId),
                dedicated,
                shared: desc.SharedSystemMemory as u64,
                integrated: dedicated < 1536 * 1024 * 1024,
            });
        }
    }
    out.sort_by_key(|a| std::cmp::Reverse(a.dedicated)); // discrete first
    out
}

/// "pid_12_luid_0x00000000_0x0000f178_phys_0_eng_3_engtype_3D" -> luid
fn luid_of(inst: &str) -> Option<String> {
    let i = inst.find("luid_")? + 5;
    inst.get(i..i + 21).map(|s| s.to_ascii_lowercase())
}

fn engine_of(inst: &str) -> Option<u32> {
    let rest = &inst[inst.find("_eng_")? + 5..];
    rest[..rest.find('_').unwrap_or(rest.len())].parse().ok()
}

pub struct GpuMonitor {
    adapters: Vec<Adapter>,
    has_engine: bool,
    nvml: Option<NvmlPoller>,
}

impl GpuMonitor {
    pub fn new(q: &mut Query, use_nvml: bool) -> Self {
        let adapters = list_adapters();
        let has_engine = q.add("gpu_engine", r"\GPU Engine(*)\Utilization Percentage");
        q.add("gpu_ded", r"\GPU Adapter Memory(*)\Dedicated Usage");
        q.add("gpu_shared", r"\GPU Adapter Memory(*)\Shared Usage");
        let nvidia: Vec<(String, String)> = adapters
            .iter()
            .filter(|a| a.vendor == "NVIDIA")
            .map(|a| (a.luid.clone(), a.name.clone()))
            .collect();
        let nvml = (use_nvml && !nvidia.is_empty()).then(|| NvmlPoller::start(nvidia));
        GpuMonitor { adapters, has_engine, nvml }
    }

    pub fn info(&self) -> Vec<GpuInfo> {
        self.adapters
            .iter()
            .map(|a| GpuInfo {
                id: a.luid.clone(),
                name: a.name.clone(),
                vendor: a.vendor.to_string(),
                integrated: a.integrated,
                vram_total: a.dedicated,
                shared_total: a.shared,
                sensors: self.nvml.is_some() && a.vendor == "NVIDIA",
            })
            .collect()
    }

    pub fn sample(&self, q: &Query) -> Vec<GpuSample> {
        // luid -> engine index -> utilization summed across processes
        let mut engines: HashMap<String, HashMap<u32, f64>> = HashMap::new();
        for (inst, v) in q.read("gpu_engine") {
            if let (Some(luid), Some(eng)) = (luid_of(&inst), engine_of(&inst)) {
                *engines.entry(luid).or_default().entry(eng).or_default() += v;
            }
        }
        let sum_by_luid = |key: &str| {
            let mut m: HashMap<String, f64> = HashMap::new();
            for (inst, v) in q.read(key) {
                if let Some(luid) = luid_of(&inst) {
                    *m.entry(luid).or_default() += v;
                }
            }
            m
        };
        let ded = sum_by_luid("gpu_ded");
        let shared = sum_by_luid("gpu_shared");

        let mut awake_nvidia = HashSet::new();
        let out = self
            .adapters
            .iter()
            .map(|a| {
                let util = engines
                    .get(&a.luid)
                    .map(|e| e.values().cloned().fold(0.0, f64::max))
                    .unwrap_or(0.0)
                    .min(100.0);
                let vram = ded.get(&a.luid).copied();
                // A powered-down laptop dGPU reports no load and no VRAM. Desktop
                // cards always hold a framebuffer, so they never match this.
                let asleep = !a.integrated && self.has_engine && util == 0.0 && vram.unwrap_or(0.0) < 1024.0 * 1024.0;
                let mut s = GpuSample {
                    id: a.luid.clone(),
                    util: self.has_engine.then_some((util * 10.0).round() / 10.0),
                    vram_used: vram,
                    shared_used: shared.get(&a.luid).copied(),
                    asleep,
                    ..Default::default()
                };
                if let (Some(nv), "NVIDIA", false) = (&self.nvml, a.vendor, asleep) {
                    awake_nvidia.insert(a.luid.clone());
                    if let Some(r) = nv.get(&a.luid) {
                        s.temp = r.temp;
                        s.power = r.power;
                        s.power_limit = r.power_limit;
                        s.clock = r.clock;
                        s.mem_clock = r.mem_clock;
                        s.fan = r.fan;
                    }
                }
                s
            })
            .collect();
        if let Some(nv) = &self.nvml {
            nv.want(awake_nvidia);
        }
        out
    }
}

// --- NVML on its own thread ------------------------------------------------------

#[derive(Clone, Default)]
struct NvReading {
    temp: Option<u32>,
    power: Option<f64>,
    power_limit: Option<f64>,
    clock: Option<u32>,
    mem_clock: Option<u32>,
    fan: Option<u32>,
}

#[derive(Default)]
struct NvShared {
    wanted: HashSet<String>,
    data: HashMap<String, NvReading>,
}

struct NvmlPoller {
    shared: Arc<(Mutex<NvShared>, Condvar)>,
}

impl NvmlPoller {
    fn start(adapters: Vec<(String, String)>) -> Self {
        let shared = Arc::new((Mutex::new(NvShared::default()), Condvar::new()));
        let s = shared.clone();
        let _ = std::thread::Builder::new().name("nvml".into()).spawn(move || nvml_loop(adapters, s));
        NvmlPoller { shared }
    }

    fn want(&self, luids: HashSet<String>) {
        let (m, cv) = &*self.shared;
        let mut g = m.lock().unwrap();
        if g.wanted != luids {
            g.data.retain(|k, _| luids.contains(k));
            g.wanted = luids;
            cv.notify_one();
        }
    }

    fn get(&self, luid: &str) -> Option<NvReading> {
        self.shared.0.lock().unwrap().data.get(luid).cloned()
    }
}

/// Where the NVIDIA driver puts nvml.dll. Never load it by bare name: Windows looks in Oriel's
/// own folder first, so a planted nvml.dll next to a portable copy (say, in Downloads) would run.
fn nvml_path() -> Option<std::path::PathBuf> {
    let system = crate::ffi::system_dir().map(|d| d.join("nvml.dll"));
    // Drivers before 2019 kept it here instead.
    let nvsmi = std::env::var_os("ProgramW6432").map(|p| std::path::Path::new(&p).join(r"NVIDIA Corporation\NVSMI\nvml.dll"));
    [system, nvsmi].into_iter().flatten().find(|p| p.is_file())
}

fn nvml_loop(adapters: Vec<(String, String)>, shared: Arc<(Mutex<NvShared>, Condvar)>) {
    let mut nvml: Option<Nvml> = None;
    let mut index: HashMap<String, u32> = HashMap::new(); // luid -> NVML device index
    loop {
        let wanted = {
            let (m, cv) = &*shared;
            let g = m.lock().unwrap();
            let (g, _) = cv.wait_timeout(g, Duration::from_secs(2)).unwrap();
            g.wanted.clone()
        };
        if wanted.is_empty() {
            continue; // nothing awake: don't touch the driver at all
        }
        if nvml.is_none() {
            let Some(lib) = nvml_path() else { return };
            let Ok(n) = Nvml::builder().lib_path(lib.as_os_str()).init() else { return };
            let mut unclaimed: Vec<(u32, String)> = (0..n.device_count().unwrap_or(0))
                .filter_map(|i| Some((i, n.device_by_index(i).ok()?.name().ok()?.to_lowercase())))
                .collect();
            for (luid, name) in &adapters {
                let name = name.to_lowercase();
                if let Some(pos) = unclaimed.iter().position(|(_, d)| name.contains(d.as_str()) || d.contains(name.as_str())) {
                    index.insert(luid.clone(), unclaimed.remove(pos).0);
                }
            }
            nvml = Some(n);
        }
        let n = nvml.as_ref().unwrap();
        for luid in wanted {
            let Some(dev) = index.get(&luid).and_then(|&i| n.device_by_index(i).ok()) else { continue };
            let reading = NvReading {
                temp: dev.temperature(TemperatureSensor::Gpu).ok(),
                power: dev.power_usage().ok().map(|mw| mw as f64 / 1000.0),
                power_limit: dev.enforced_power_limit().ok().map(|mw| mw as f64 / 1000.0),
                clock: dev.clock_info(Clock::Graphics).ok(),
                mem_clock: dev.clock_info(Clock::Memory).ok(),
                fan: dev.fan_speed(0).ok(),
            };
            let mut g = shared.0.lock().unwrap();
            if g.wanted.contains(&luid) {
                g.data.insert(luid, reading); // skip if the GPU went to sleep meanwhile
            }
        }
    }
}
