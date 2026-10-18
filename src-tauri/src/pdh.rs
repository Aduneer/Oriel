//! Windows performance counters: the same source Task Manager uses.
//!
//! Wildcard counters on a long-lived query pick up new instances (new
//! processes, adapters) by themselves; rate counters need two collections.

use crate::ffi::*;
use std::collections::HashMap;
use std::ptr::null_mut;

pub struct Query {
    h: HANDLE,
    counters: HashMap<&'static str, HANDLE>,
}

// PDH handles aren't tied to the thread that created them.
unsafe impl Send for Query {}

impl Query {
    pub fn new() -> Option<Self> {
        let mut h = null_mut();
        (unsafe { PdhOpenQueryW(std::ptr::null(), 0, &mut h) } == 0).then(|| Query { h, counters: HashMap::new() })
    }

    /// Add a counter by its English path. Returns false if it doesn't exist on this machine.
    pub fn add(&mut self, key: &'static str, path: &str) -> bool {
        let mut c = null_mut();
        let p = wide(path);
        let ok = unsafe { PdhAddEnglishCounterW(self.h, p.as_ptr(), 0, &mut c) } == 0;
        if ok {
            self.counters.insert(key, c);
        }
        ok
    }

    pub fn collect(&self) {
        unsafe { PdhCollectQueryData(self.h) };
    }

    /// (instance, value) pairs. Instance names can repeat, so this isn't a map.
    pub fn read(&self, key: &str) -> Vec<(String, f64)> {
        let Some(&c) = self.counters.get(key) else { return Vec::new() };
        let fmt = PDH_FMT_DOUBLE | PDH_FMT_NOCAP100;
        for _ in 0..2 {
            let (mut size, mut count) = (0u32, 0u32);
            unsafe {
                if PdhGetFormattedCounterArrayW(c, fmt, &mut size, &mut count, null_mut()) != PDH_MORE_DATA {
                    return Vec::new();
                }
                let mut buf = vec![0u64; (size as usize).div_ceil(8)];
                let status = PdhGetFormattedCounterArrayW(c, fmt, &mut size, &mut count, buf.as_mut_ptr().cast());
                if status == PDH_MORE_DATA {
                    continue; // an instance appeared between the two calls
                }
                if status != 0 {
                    return Vec::new();
                }
                let items = std::slice::from_raw_parts(buf.as_ptr() as *const PDH_FMT_COUNTERVALUE_ITEM_W, count as usize);
                return items
                    .iter()
                    .filter(|i| matches!(i.FmtValue.CStatus, PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA))
                    .map(|i| (from_wide_ptr(i.szName), i.FmtValue.doubleValue))
                    .collect();
            }
        }
        Vec::new()
    }

    pub fn total(&self, key: &str) -> Option<f64> {
        self.read(key).into_iter().find(|(n, _)| n == "_Total").map(|(_, v)| v)
    }
}

impl Drop for Query {
    fn drop(&mut self) {
        unsafe { PdhCloseQuery(self.h) };
    }
}
