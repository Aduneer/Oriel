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
    /// Rows of widget ids, top to bottom
    pub layout: Vec<Vec<String>>,
    pub limits: Limits,
    /// Nudge the layout a pixel or two now and then so static UI can't burn into OLED panels
    pub burn_in_shift: bool,
    /// Let the dashboard draw with the GPU. Off by default: software drawing uses ~130 MB less
    /// memory for a little more CPU, and a page that redraws a few graphs a second doesn't need it.
    pub gpu: bool,
    pub nvml: bool,
    pub lhm_url: String,
}

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
            layout: vec![row(&["cpu", "gpu", "memory"]), row(&["network", "disk", "processes"])],
            limits: Limits::default(),
            burn_in_shift: true,
            gpu: false,
            nvml: true,
            lhm_url: "http://127.0.0.1:8085/data.json".into(),
        }
    }
}

pub fn path() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("Oriel").join("settings.json")
}

impl Settings {
    pub fn load() -> Self {
        // Notepad and PowerShell can save with a byte-order mark, which serde_json rejects.
        std::fs::read_to_string(path())
            .ok()
            .and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok())
            .unwrap_or_default()
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

    pub fn clamp(mut self) -> Self {
        self.interval_ms = self.interval_ms.clamp(250, 10_000);
        self.graph_seconds = self.graph_seconds.clamp(30, 600);
        // It ends up in an HTML attribute, so keep it to a plain id.
        let id_ok = |s: &str| !s.is_empty() && s.len() <= 32 && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !id_ok(&self.palette) {
            self.palette = "oriel".into();
        }
        self
    }
}
