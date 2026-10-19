//! Optional sensors from LibreHardwareMonitor's web server (Options > Remote Web Server).
//!
//! Windows doesn't give normal programs the CPU temperature sensor or fan speeds;
//! LHM has a driver that does. We poll its data.json on our own thread.

use serde::Serialize;
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Serialize, Clone)]
pub struct Fan {
    pub name: String,
    pub rpm: u32,
}

#[derive(Clone, Default)]
pub struct LhmData {
    pub cpu_temp: Option<f64>,
    pub cpu_power: Option<f64>,
    pub fans: Vec<Fan>,
}

const CPU_TEMP_NAMES: &[&str] = &["core (tctl/tdie)", "cpu package", "core (tctl)", "core (tdie)", "package", "core max", "core average"];
const CPU_POWER_NAMES: &[&str] = &["package", "cpu package"];

pub struct Lhm {
    last: Arc<Mutex<Option<LhmData>>>,
}

impl Lhm {
    pub fn start(url: String) -> Self {
        let last = Arc::new(Mutex::new(None));
        let l = last.clone();
        let _ = std::thread::Builder::new().name("lhm".into()).spawn(move || loop {
            let data = http_get(&url).and_then(|body| serde_json::from_str::<Value>(body.trim_start_matches('\u{feff}')).ok());
            let ok = data.is_some();
            *l.lock().unwrap() = data.map(|d| parse(&d));
            std::thread::sleep(Duration::from_secs(if ok { 2 } else { 30 }));
        });
        Lhm { last }
    }

    pub fn read(&self) -> Option<LhmData> {
        self.last.lock().unwrap().clone()
    }
}

/// Minimal HTTP/1.0 GET for a localhost JSON endpoint; no TLS, no proxies.
fn http_get(url: &str) -> Option<String> {
    let rest = url.strip_prefix("http://")?;
    let (host, path) = match rest.split_once('/') {
        Some((h, p)) => (h, format!("/{p}")),
        None => (rest, "/".to_string()),
    };
    let addr_str = if host.contains(':') { host.to_string() } else { format!("{host}:80") };
    let addr = addr_str.to_socket_addrs().ok()?.next()?;
    let timeout = Duration::from_secs(1);
    let mut s = TcpStream::connect_timeout(&addr, timeout).ok()?;
    s.set_read_timeout(Some(timeout)).ok()?;
    write!(s, "GET {path} HTTP/1.0\r\nHost: {host}\r\nAccept: application/json\r\nConnection: close\r\n\r\n").ok()?;
    // LHM's data.json is a few hundred KB at most. Cap it, so whatever else might be listening on
    // that port can't make us buffer without end.
    let mut buf = Vec::new();
    s.take(8 << 20).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let (head, body) = text.split_once("\r\n\r\n")?;
    (head.split_whitespace().nth(1)? == "200").then(|| body.to_string())
}

fn num(v: &Value) -> Option<f64> {
    // "55,3 °C" (comma locales) or "55.3 °C"
    let s = v.as_str()?;
    let start = s.find(|c: char| c.is_ascii_digit() || c == '-')?;
    let t: String = s[start..].chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',' || *c == '-').collect();
    t.replace(',', ".").parse().ok()
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// Yields (hardware node, group text, sensor node) for every sensor leaf.
fn walk<'a>(node: &'a Value, hw: Option<&'a Value>, group: Option<String>, out: &mut Vec<(&'a Value, String, &'a Value)>) {
    let children = node.get("Children").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
    if children.is_empty() {
        if let (Some(hw), Some(group)) = (hw, group) {
            out.push((hw, group, node));
        }
        return;
    }
    let image = text(node, "ImageURL").to_lowercase();
    let is_hw = node.get("HardwareId").is_some() || (image.starts_with("images_icon/") && !image.ends_with("computer.png"));
    for child in children {
        match (hw, &group) {
            (None, _) if is_hw => walk(child, Some(node), None, out),
            (Some(h), None) => walk(child, Some(h), Some(text(node, "Text")), out),
            _ => walk(child, hw, group.clone(), out),
        }
    }
}

fn parse(data: &Value) -> LhmData {
    let mut leaves = Vec::new();
    walk(data, None, None, &mut leaves);
    let (mut temps, mut powers, mut fans) = (Vec::new(), Vec::new(), Vec::new());
    for (hw, group, s) in leaves {
        let kind = s.get("Type").and_then(Value::as_str).map(str::to_string).unwrap_or(group).to_lowercase();
        let name = text(s, "Text").trim().to_string();
        let Some(value) = s.get("Value").and_then(num) else { continue };
        let is_cpu = text(hw, "HardwareId").to_lowercase().contains("cpu") || text(hw, "ImageURL").to_lowercase().ends_with("cpu.png");
        if is_cpu && kind.starts_with("temperature") {
            temps.push((name.to_lowercase(), value));
        } else if is_cpu && kind.starts_with("power") {
            powers.push((name.to_lowercase(), value));
        }
        let pct = s.get("Value").and_then(Value::as_str).is_some_and(|v| v.contains('%'));
        if kind.starts_with("fan") && !pct && value > 0.0 {
            fans.push(Fan { name, rpm: value.round() as u32 });
        }
    }
    let pick = |found: &[(String, f64)], prefs: &[&str]| {
        prefs
            .iter()
            .find_map(|p| found.iter().find(|(n, _)| n == p).map(|(_, v)| *v))
            .or_else(|| found.iter().map(|(_, v)| *v).reduce(f64::max))
    };
    LhmData { cpu_temp: pick(&temps, CPU_TEMP_NAMES), cpu_power: pick(&powers, CPU_POWER_NAMES), fans }
}
