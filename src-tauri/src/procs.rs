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

/// What came of ending the processes the page asked about.
#[derive(Serialize, Default, Debug)]
pub struct Ended {
    ended: u32,
    /// Windows said no: the process runs as admin, as another user, or as part of Windows.
    denied: u32,
    /// Already exited, or its id has since gone to a process the page never showed.
    gone: u32,
    /// One of them was Oriel itself, which it leaves alone.
    own: bool,
}

enum Outcome {
    Ended,
    Denied,
    Gone,
}

/// Ends processes the way Task Manager's "End task" does, with the user's own rights.
///
/// `seen` is the time (Unix seconds) of the sample the page was showing. Windows reuses process
/// ids, so a process that started after that sample can't be the one the user picked, even with
/// the same id; it's left alone. Samples are timed before the process list is read, so this
/// errs towards leaving things running.
pub fn end(pids: &[u32], seen: f64) -> Ended {
    // FILETIME counts from 1601, Unix time from 1970.
    let seen = ((seen + 11_644_473_600.0) * 1e7) as i64;
    let mut out = Ended::default();
    for &pid in pids {
        if pid == std::process::id() {
            out.own = true;
            continue;
        }
        match end_one(pid, seen) {
            Outcome::Ended => out.ended += 1,
            Outcome::Denied => out.denied += 1,
            Outcome::Gone => out.gone += 1,
        }
    }
    out
}

fn end_one(pid: u32, seen: i64) -> Outcome {
    let denied_or_gone = || match std::io::Error::last_os_error().raw_os_error() {
        Some(ERROR_ACCESS_DENIED) => Outcome::Denied,
        _ => Outcome::Gone, // ERROR_INVALID_PARAMETER: no process with that id
    };
    let h = unsafe { OpenProcess(PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if h.is_null() {
        return denied_or_gone();
    }
    let [mut created, mut exited, mut kernel, mut user]: [FILETIME; 4] = Default::default();
    let outcome = if unsafe { GetProcessTimes(h, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
        Outcome::Gone
    } else if exited.ticks() != 0 || created.ticks() > seen {
        // Exited but still held open by someone, or a newer process under a reused id.
        Outcome::Gone
    } else if unsafe { TerminateProcess(h, 1) } != 0 {
        Outcome::Ended
    } else {
        denied_or_gone()
    };
    unsafe { CloseHandle(h) };
    outcome
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

    fn now() -> f64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64()
    }

    #[test]
    fn ends_only_the_process_it_was_shown() {
        // Something harmless that would otherwise run for a minute.
        let mut child = std::process::Command::new("ping")
            .args(["-n", "60", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("couldn't start ping");
        let pid = child.id();

        // A sample from before it started: the id isn't the process the user saw, so hands off.
        let r = end(&[pid], now() - 60.0);
        assert_eq!((r.ended, r.gone), (0, 1), "{r:?}");
        assert!(child.try_wait().unwrap().is_none(), "ended a process the page never showed");

        let r = end(&[pid], now());
        assert_eq!(r.ended, 1, "{r:?}");
        assert_eq!(child.wait().unwrap().code(), Some(1));

        // Exited (we still hold it open, so the id isn't reused yet).
        assert_eq!(end(&[pid], now()).gone, 1);
        // Windows' own System process refuses.
        assert_eq!(end(&[4], now()).denied, 1);
        // And never Oriel itself.
        let r = end(&[std::process::id()], now());
        assert!(r.own && r.ended == 0, "{r:?}");
    }
}
