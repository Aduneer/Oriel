//! Every process's CPU time, private memory and parent from one NtQuerySystemInformation
//! call (a few ms). The page groups them by name (`chrome ×14`) or lays them out as a tree.

use crate::ffi::*;
use serde::Serialize;
use std::collections::HashMap;
use std::time::Instant;

#[derive(Serialize, Clone)]
pub struct Proc {
    pid: u32,
    /// 0 for a root: no parent, or the parent has exited (Windows doesn't reparent orphans)
    ppid: u32,
    name: String,
    cpu: f64,
    mem: u64,
}

pub struct ProcessSampler {
    ncpu: f64,
    buf: Vec<u64>, // u64 so entries are 8-byte aligned
    prev: HashMap<(usize, i64), i64>,
    prev_t: Option<Instant>,
}

struct Raw {
    pid: usize,
    ppid: usize,
    created: i64,
    name: String,
    cpu: i64,
    mem: i64,
}

impl ProcessSampler {
    pub fn new(ncpu: usize) -> Self {
        ProcessSampler { ncpu: ncpu.max(1) as f64, buf: vec![0; 64 * 1024], prev: HashMap::new(), prev_t: None }
    }

    fn query(&mut self) -> Vec<Raw> {
        let mut ret = 0u32;
        loop {
            let status = unsafe {
                NtQuerySystemInformation(
                    SYSTEM_PROCESS_INFORMATION_CLASS,
                    self.buf.as_mut_ptr().cast(),
                    (self.buf.len() * 8) as u32,
                    &mut ret,
                )
            };
            if status == STATUS_INFO_LENGTH_MISMATCH {
                // Room for what it asked plus growth, and always bigger than before so this ends.
                self.buf.resize((ret as usize / 8 + 1).max(self.buf.len()) * 2, 0);
                continue;
            }
            if status != 0 {
                return Vec::new();
            }
            break;
        }
        let base = self.buf.as_ptr() as *const u8;
        let mut out = Vec::with_capacity(400);
        let mut off = 0usize;
        loop {
            let p: SYSTEM_PROCESS_INFORMATION = unsafe { std::ptr::read_unaligned(base.add(off).cast()) };
            if p.UniqueProcessId != 0 {
                let name = if p.ImageName.Buffer.is_null() {
                    "?".to_string()
                } else {
                    let s = unsafe { std::slice::from_raw_parts(p.ImageName.Buffer, p.ImageName.Length as usize / 2) };
                    String::from_utf16_lossy(s)
                };
                out.push(Raw {
                    pid: p.UniqueProcessId,
                    ppid: p.InheritedFromUniqueProcessId,
                    created: p.CreateTime,
                    name,
                    cpu: p.UserTime + p.KernelTime,
                    mem: p.WorkingSetPrivateSize,
                });
            }
            if p.NextEntryOffset == 0 {
                return out;
            }
            off += p.NextEntryOffset as usize;
        }
    }

    /// Every process. CPU is % of the whole machine.
    pub fn sample(&mut self) -> Vec<Proc> {
        let now = Instant::now();
        let raw = self.query();
        let wall = self.prev_t.map(|t| now.duration_since(t).as_secs_f64() * 1e7 * self.ncpu);
        // Windows reuses process ids, so a parent id only counts if that process is still
        // running and is older than the child; otherwise the id belongs to someone else by now.
        let born: HashMap<usize, i64> = raw.iter().map(|r| (r.pid, r.created)).collect();
        let mut prev = HashMap::with_capacity(raw.len());
        let mut out = Vec::with_capacity(raw.len());
        for r in raw {
            let key = (r.pid, r.created);
            let cpu = match (wall, self.prev.get(&key)) {
                (Some(wall), Some(&before)) => (r.cpu - before).max(0) as f64 / wall * 100.0,
                _ => 0.0,
            };
            prev.insert(key, r.cpu);
            let ppid = match born.get(&r.ppid) {
                Some(&t) if r.ppid != r.pid && t <= r.created => r.ppid,
                _ => 0,
            };
            out.push(Proc {
                pid: r.pid as u32,
                ppid: ppid as u32,
                name: strip_exe(r.name),
                // Two decimals, so a group of many nearly idle processes still adds up.
                cpu: (cpu.min(100.0) * 100.0).round() / 100.0,
                mem: r.mem.max(0) as u64,
            });
        }
        self.prev = prev;
        self.prev_t = Some(now);
        out
    }
}

fn strip_exe(mut name: String) -> String {
    let n = name.len();
    if n > 4 && name.get(n - 4..).is_some_and(|ext| ext.eq_ignore_ascii_case(".exe")) {
        name.truncate(n - 4);
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn parents_form_a_tree() {
        let mut sampler = ProcessSampler::new(1);
        let procs = sampler.sample();
        let pids: HashSet<u32> = procs.iter().map(|p| p.pid).collect();
        assert!(procs.len() > 10, "only {} processes", procs.len());
        assert_eq!(pids.len(), procs.len(), "duplicate pids");
        // Every parent is a process in the same snapshot, and following parents always ends at a root.
        let parent: HashMap<u32, u32> = procs.iter().map(|p| (p.pid, p.ppid)).collect();
        for p in &procs {
            assert!(p.ppid == 0 || pids.contains(&p.ppid), "{} ({}) has a missing parent {}", p.name, p.pid, p.ppid);
            let (mut at, mut steps) = (p.pid, 0);
            while at != 0 {
                at = parent[&at];
                steps += 1;
                assert!(steps < 64, "parent loop at {} ({})", p.name, p.pid);
            }
        }
        // This test runs under cargo, so it has a parent.
        let me = std::process::id();
        let mine = procs.iter().find(|p| p.pid == me).expect("own process missing");
        assert_ne!(mine.ppid, 0, "own process has no parent");
        let name = |pid: u32| procs.iter().find(|p| p.pid == pid).map_or("?", |p| p.name.as_str());
        let mut chain = vec![mine.name.clone()];
        let mut at = mine.ppid;
        while at != 0 {
            chain.push(name(at).to_string());
            at = parent[&at];
        }
        let roots = procs.iter().filter(|p| p.ppid == 0).count();
        println!("{} processes, {} roots; this test's ancestry: {}", procs.len(), roots, chain.join(" <- "));
    }
}
