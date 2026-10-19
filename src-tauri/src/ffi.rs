//! Hand-written bindings for the flat Win32 / NT calls Oriel uses.
//! COM (DXGI) goes through the `windows` crate instead.
#![allow(non_snake_case, non_camel_case_types, clippy::upper_case_acronyms)]

use std::ffi::c_void;
use std::ptr::null_mut;

pub type HANDLE = *mut c_void;

// --- Performance Data Helper ---------------------------------------------------

pub const PDH_FMT_DOUBLE: u32 = 0x0000_0200;
pub const PDH_FMT_NOCAP100: u32 = 0x0000_8000;
pub const PDH_MORE_DATA: u32 = 0x8000_07D2;
pub const PDH_CSTATUS_VALID_DATA: u32 = 0;
pub const PDH_CSTATUS_NEW_DATA: u32 = 1;

#[repr(C)]
pub struct PDH_FMT_COUNTERVALUE {
    pub CStatus: u32,
    pub doubleValue: f64,
}

#[repr(C)]
pub struct PDH_FMT_COUNTERVALUE_ITEM_W {
    pub szName: *const u16,
    pub FmtValue: PDH_FMT_COUNTERVALUE,
}

#[link(name = "pdh", kind = "raw-dylib")]
extern "system" {
    pub fn PdhOpenQueryW(source: *const u16, user: usize, query: *mut HANDLE) -> u32;
    pub fn PdhAddEnglishCounterW(query: HANDLE, path: *const u16, user: usize, counter: *mut HANDLE) -> u32;
    pub fn PdhCollectQueryData(query: HANDLE) -> u32;
    pub fn PdhGetFormattedCounterArrayW(
        counter: HANDLE,
        format: u32,
        size: *mut u32,
        count: *mut u32,
        items: *mut PDH_FMT_COUNTERVALUE_ITEM_W,
    ) -> u32;
    pub fn PdhCloseQuery(query: HANDLE) -> u32;
}

// --- NT process list -----------------------------------------------------------

pub const SYSTEM_PROCESS_INFORMATION_CLASS: u32 = 5;
pub const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004_u32 as i32;

#[repr(C)]
pub struct UNICODE_STRING {
    pub Length: u16,
    pub MaximumLength: u16,
    pub Buffer: *const u16,
}

/// Leading part of SYSTEM_PROCESS_INFORMATION; we never read past InheritedFromUniqueProcessId.
#[repr(C)]
pub struct SYSTEM_PROCESS_INFORMATION {
    pub NextEntryOffset: u32,
    pub NumberOfThreads: u32,
    pub WorkingSetPrivateSize: i64,
    pub HardFaultCount: u32,
    pub NumberOfThreadsHighWatermark: u32,
    pub CycleTime: u64,
    pub CreateTime: i64,
    pub UserTime: i64,
    pub KernelTime: i64,
    pub ImageName: UNICODE_STRING,
    pub BasePriority: i32,
    pub UniqueProcessId: usize,
    pub InheritedFromUniqueProcessId: usize,
}

#[link(name = "ntdll", kind = "raw-dylib")]
extern "system" {
    pub fn NtQuerySystemInformation(class: u32, info: *mut c_void, len: u32, ret_len: *mut u32) -> i32;
}

// --- kernel32 / advapi32 -------------------------------------------------------

#[repr(C)]
#[derive(Default)]
pub struct MEMORYSTATUSEX {
    pub dwLength: u32,
    pub dwMemoryLoad: u32,
    pub ullTotalPhys: u64,
    pub ullAvailPhys: u64,
    pub ullTotalPageFile: u64,
    pub ullAvailPageFile: u64,
    pub ullTotalVirtual: u64,
    pub ullAvailVirtual: u64,
    pub ullAvailExtendedVirtual: u64,
}

#[repr(C)]
#[derive(Default)]
pub struct PERFORMANCE_INFORMATION {
    pub cb: u32,
    pub CommitTotal: usize,
    pub CommitLimit: usize,
    pub CommitPeak: usize,
    pub PhysicalTotal: usize,
    pub PhysicalAvailable: usize,
    pub SystemCache: usize,
    pub KernelTotal: usize,
    pub KernelPaged: usize,
    pub KernelNonpaged: usize,
    pub PageSize: usize,
    pub HandleCount: u32,
    pub ProcessCount: u32,
    pub ThreadCount: u32,
}

#[repr(C)]
#[derive(Default)]
pub struct SYSTEM_POWER_STATUS {
    pub ACLineStatus: u8,
    pub BatteryFlag: u8,
    pub BatteryLifePercent: u8,
    pub SystemStatusFlag: u8,
    pub BatteryLifeTime: u32,
    pub BatteryFullLifeTime: u32,
}

pub const DRIVE_FIXED: u32 = 3;

pub const PROCESS_TERMINATE: u32 = 0x0001;
pub const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
pub const ERROR_ACCESS_DENIED: i32 = 5;

/// 100 ns ticks since 1601 (UTC), in two halves.
#[repr(C)]
#[derive(Default)]
pub struct FILETIME {
    pub lo: u32,
    pub hi: u32,
}

impl FILETIME {
    pub fn ticks(&self) -> i64 {
        ((self.hi as i64) << 32) | self.lo as i64
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
extern "system" {
    pub fn OpenProcess(access: u32, inherit: i32, pid: u32) -> HANDLE;
    pub fn GetProcessTimes(process: HANDLE, created: *mut FILETIME, exited: *mut FILETIME, kernel: *mut FILETIME, user: *mut FILETIME) -> i32;
    pub fn TerminateProcess(process: HANDLE, exit_code: u32) -> i32;
    pub fn QueryFullProcessImageNameW(process: HANDLE, flags: u32, name: *mut u16, len: *mut u32) -> i32;
    pub fn CloseHandle(handle: HANDLE) -> i32;
    pub fn GlobalMemoryStatusEx(buf: *mut MEMORYSTATUSEX) -> i32;
    pub fn K32GetPerformanceInfo(info: *mut PERFORMANCE_INFORMATION, cb: u32) -> i32;
    pub fn GetSystemPowerStatus(status: *mut SYSTEM_POWER_STATUS) -> i32;
    pub fn GetTickCount64() -> u64;
    pub fn GetSystemDirectoryW(buf: *mut u16, len: u32) -> u32;
    pub fn GetLogicalDriveStringsW(len: u32, buf: *mut u16) -> u32;
    pub fn GetDriveTypeW(root: *const u16) -> u32;
    pub fn GetDiskFreeSpaceExW(dir: *const u16, avail: *mut u64, total: *mut u64, free: *mut u64) -> i32;
    pub fn GetLogicalProcessorInformationEx(relation: u32, buf: *mut c_void, len: *mut u32) -> i32;
}

/// HKEY_LOCAL_MACHINE is a sign-extended 32-bit constant.
const HKEY_LOCAL_MACHINE: isize = 0x8000_0002_u32 as i32 as isize;
const RRF_RT_REG_SZ: u32 = 0x0000_0002;
const RRF_RT_REG_DWORD: u32 = 0x0000_0010;

#[link(name = "advapi32", kind = "raw-dylib")]
extern "system" {
    fn RegGetValueW(
        hkey: isize,
        subkey: *const u16,
        value: *const u16,
        flags: u32,
        ty: *mut u32,
        data: *mut c_void,
        len: *mut u32,
    ) -> i32;
}

// --- COM -----------------------------------------------------------------------

pub const COINIT_MULTITHREADED: u32 = 0;

#[link(name = "ole32", kind = "raw-dylib")]
extern "system" {
    pub fn CoInitializeEx(reserved: *mut c_void, coinit: u32) -> i32;
}

// --- shell ---------------------------------------------------------------------

pub const SW_SHOWNORMAL: i32 = 1;

#[link(name = "shell32", kind = "raw-dylib")]
extern "system" {
    /// Returns a value > 32 on success.
    pub fn ShellExecuteW(hwnd: isize, op: *const u16, file: *const u16, params: *const u16, dir: *const u16, show: i32) -> isize;
}

// --- windows -------------------------------------------------------------------

pub const GWL_EXSTYLE: i32 = -20;
pub const GW_OWNER: u32 = 4;
pub const SW_RESTORE: i32 = 9;
pub const WM_CLOSE: u32 = 0x0010;
pub const DWMWA_CLOAKED: u32 = 14;

pub type WNDENUMPROC = unsafe extern "system" fn(hwnd: isize, lparam: isize) -> i32;

#[link(name = "user32", kind = "raw-dylib")]
extern "system" {
    pub fn EnumWindows(callback: WNDENUMPROC, lparam: isize) -> i32;
    pub fn IsWindow(hwnd: isize) -> i32;
    pub fn IsWindowVisible(hwnd: isize) -> i32;
    pub fn IsIconic(hwnd: isize) -> i32;
    pub fn GetWindow(hwnd: isize, cmd: u32) -> isize;
    pub fn GetWindowLongW(hwnd: isize, index: i32) -> i32;
    pub fn GetWindowTextW(hwnd: isize, buf: *mut u16, max: i32) -> i32;
    pub fn GetClassNameW(hwnd: isize, buf: *mut u16, max: i32) -> i32;
    pub fn GetWindowThreadProcessId(hwnd: isize, pid: *mut u32) -> u32;
    pub fn GetForegroundWindow() -> isize;
    pub fn SetForegroundWindow(hwnd: isize) -> i32;
    pub fn ShowWindow(hwnd: isize, cmd: i32) -> i32;
    pub fn PostMessageW(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> i32;
}

#[link(name = "dwmapi", kind = "raw-dylib")]
extern "system" {
    pub fn DwmGetWindowAttribute(hwnd: isize, attr: u32, value: *mut c_void, size: u32) -> i32;
}

// --- helpers -------------------------------------------------------------------

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// C:\Windows\System32, asked of Windows rather than built from environment variables.
pub fn system_dir() -> Option<std::path::PathBuf> {
    let mut buf = [0u16; 260];
    let n = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    (n > 0 && n < buf.len()).then(|| String::from_utf16_lossy(&buf[..n]).into())
}

/// # Safety
/// `p` must be null or point to a NUL-terminated UTF-16 string.
pub unsafe fn from_wide_ptr(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
}

pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

pub fn reg_string(path: &str, value: &str) -> Option<String> {
    let (p, v) = (wide(path), wide(value));
    let mut len = 0u32;
    unsafe {
        if RegGetValueW(HKEY_LOCAL_MACHINE, p.as_ptr(), v.as_ptr(), RRF_RT_REG_SZ, null_mut(), null_mut(), &mut len) != 0 {
            return None;
        }
        let mut buf = vec![0u16; len as usize / 2 + 1];
        if RegGetValueW(HKEY_LOCAL_MACHINE, p.as_ptr(), v.as_ptr(), RRF_RT_REG_SZ, null_mut(), buf.as_mut_ptr().cast(), &mut len) != 0 {
            return None;
        }
        Some(from_wide(&buf).trim().to_string())
    }
}

pub fn reg_dword(path: &str, value: &str) -> Option<u32> {
    let (p, v) = (wide(path), wide(value));
    let mut data = 0u32;
    let mut len = 4u32;
    let ok = unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, p.as_ptr(), v.as_ptr(), RRF_RT_REG_DWORD, null_mut(), (&mut data as *mut u32).cast(), &mut len)
    } == 0;
    ok.then_some(data)
}

pub fn physical_cores() -> usize {
    const RELATION_PROCESSOR_CORE: u32 = 0;
    let mut len = 0u32;
    unsafe {
        GetLogicalProcessorInformationEx(RELATION_PROCESSOR_CORE, null_mut(), &mut len);
        let mut buf = vec![0u8; len as usize];
        if GetLogicalProcessorInformationEx(RELATION_PROCESSOR_CORE, buf.as_mut_ptr().cast(), &mut len) == 0 {
            return 0;
        }
        // Variable-size records: { Relationship: u32, Size: u32, ... }
        let (mut off, mut n) = (0usize, 0usize);
        while off + 8 <= len as usize {
            let size = u32::from_ne_bytes(buf[off + 4..off + 8].try_into().unwrap()) as usize;
            if size == 0 {
                break;
            }
            n += 1;
            off += size;
        }
        n
    }
}
