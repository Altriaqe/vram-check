//! 采样：常驻 PDH 查询 + 一轮一次进程快照，产出 `Snapshot`。
//!
//! 线程模型见 `docs/ARCHITECTURE.md` §4：采样线程不碰任何窗口；
//! 数据经 `mpsc` 传回 UI，`PostMessageW` 只做唤醒。

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, HWND, WAIT_OBJECT_0};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{
    CreateEventW, GetProcessTimes, OpenProcess, SetEvent, WaitForMultipleObjects,
    WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::vendor::dxgi::AdapterInfo;
use crate::vendor::pdh::{parse_instance, CounterSet};

use super::model::{
    build_groups, unattributed, FileMeta, Mode, ProcSample, Snapshot, Unattributed,
};
use super::SampleError;

/// 采样线程通知 UI："有新快照"。
pub const WM_SNAPSHOT_READY: u32 = 0x8001;
/// 采样线程通知 UI："这一轮失败了"。
pub const WM_SAMPLE_FAILED: u32 = 0x8002;

const INFINITE: u32 = 0xFFFF_FFFF;

const PATH_LOCAL: &str = r"\GPU Process Memory(*)\Local Usage";
const PATH_NONLOCAL: &str = r"\GPU Process Memory(*)\Non Local Usage";
const PATH_ENGINE: &str = r"\GPU Engine(*)\Utilization Percentage";
const PATH_ADAPTER: &str = r"\GPU Adapter Memory(*)\Dedicated Usage";

struct ProcessInfo {
    name: String,
    parent: u32,
}

pub struct Sampler {
    counters: CounterSet,
    adapter: AdapterInfo,
    /// GPU% 每隔一轮采一次：它最贵（437 个引擎实例），而 2 秒的迟滞用户感觉不到。
    engine_cache: HashMap<u32, f32>,
    tick: u64,
    /// 进程名/父进程缓存：Toolhelp 枚举 336 个进程要 6–12 ms，没必要每秒做。
    procs: HashMap<u32, ProcessInfo>,
    procs_at: Instant,
    /// 按程序名缓存的 exe 版本信息；查询一次就够。
    meta_cache: HashMap<String, Option<FileMeta>>,
}

impl Sampler {
    pub fn new(adapter: AdapterInfo) -> Result<Self, SampleError> {
        let counters = CounterSet::open(&[PATH_LOCAL, PATH_NONLOCAL, PATH_ENGINE, PATH_ADAPTER])
            .map_err(SampleError::Open)?;
        // 速率型计数器（GPU%）需要两个采样点才有值；先采一次，稍后 sample() 再采。
        let _ = counters.collect();
        std::thread::sleep(Duration::from_millis(120));
        Ok(Self {
            counters,
            adapter,
            engine_cache: HashMap::new(),
            tick: 0,
            procs: HashMap::new(),
            procs_at: Instant::now(),
            meta_cache: HashMap::new(),
        })
    }

    /// 采一轮。任何一条数据读不到都宁可缺失，也不写 0。
    pub fn sample(&mut self) -> Result<Snapshot, SampleError> {
        self.counters.collect().map_err(SampleError::Collect)?;
        let local = self.counters.read_array(0).map_err(SampleError::Read)?;
        let nonlocal = self.counters.read_array(1).map_err(SampleError::Read)?;
        self.tick = self.tick.wrapping_add(1);
        let engine = if self.tick % 2 == 1 {
            Some(self.counters.read_array(2).map_err(SampleError::Read)?)
        } else {
            None
        };
        let adapter_rows = self.counters.read_array(3).map_err(SampleError::Read)?;
        let want_luid = self.adapter.luid;

        let mut local_map: HashMap<u32, u64> = HashMap::new();
        for (name, value) in &local {
            let Some(instance) = parse_instance(name) else {
                continue;
            };
            if instance.luid != want_luid || *value < 0.5 {
                continue;
            }
            let Some(pid) = instance.pid else { continue };
            *local_map.entry(pid).or_insert(0) += *value as u64;
        }

        let mut nonlocal_map: HashMap<u32, u64> = HashMap::new();
        for (name, value) in &nonlocal {
            let Some(instance) = parse_instance(name) else {
                continue;
            };
            if instance.luid != want_luid || *value < 0.5 {
                continue;
            }
            let Some(pid) = instance.pid else { continue };
            *nonlocal_map.entry(pid).or_insert(0) += *value as u64;
        }

        // GPU% 取最忙引擎，不能求和；每两轮刷新一次缓存。
        if let Some(engine) = &engine {
            let mut fresh: HashMap<u32, f32> = HashMap::new();
            for (name, value) in engine {
                let Some(instance) = parse_instance(name) else {
                    continue;
                };
                if instance.luid != want_luid {
                    continue;
                }
                let Some(pid) = instance.pid else { continue };
                let pct = (*value as f32).clamp(0.0, 100.0);
                fresh
                    .entry(pid)
                    .and_modify(|old| *old = old.max(pct))
                    .or_insert(pct);
            }
            self.engine_cache = fresh;
        }

        let needed: Vec<u32> = local_map.keys().copied().collect();
        self.ensure_process_table(&needed)?;
        let window_pids = super::windows::visible_window_pids();

        let mut samples: Vec<ProcSample> = Vec::with_capacity(local_map.len());
        for (pid, bytes) in local_map {
            let mut sample = base_sample(pid, &self.procs, &window_pids);
            sample.local_bytes = bytes;
            sample.nonlocal_bytes = nonlocal_map.remove(&pid).unwrap_or(0);
            sample.gpu_pct = self.engine_cache.get(&pid).copied();
            samples.push(sample);
        }

        let alive: HashSet<u32> = samples.iter().map(|s| s.pid).collect();
        self.engine_cache.retain(|pid, _| alive.contains(pid));
        samples.sort_unstable_by_key(|s| std::cmp::Reverse(s.local_bytes));

        // 第一轮已经要刷进程表，别再叠加版本资源查询；下一轮再补。
        let file_meta = if self.tick == 1 {
            HashMap::new()
        } else {
            collect_meta(&samples, &mut self.meta_cache)
        };
        let total_local: u64 = samples.iter().map(|s| s.local_bytes).sum();
        let groups = build_groups(&samples);
        let adapter_used = adapter_used_bytes(&adapter_rows, want_luid);
        let unattributed = match adapter_used {
            Some(used) => unattributed(used, total_local),
            None => Unattributed::Unavailable,
        };

        Ok(Snapshot {
            mode: Mode::Gpu,
            adapter_name: self.adapter.name.clone(),
            capacity_bytes: self.adapter.dedicated_bytes,
            adapter_used,
            total_local,
            groups,
            unattributed,
            file_meta,
        })
    }

    /// 进程表按 5 秒节流刷新；遇到没见过的 pid 立刻补一次（但最多每秒一次）。
    /// 刷新尽量放在不读 GPU% 的偶数轮，避免两个最贵的操作叠在同一次采样里。
    fn ensure_process_table(&mut self, needed: &[u32]) -> Result<(), SampleError> {
        let stale = self.procs.is_empty() || self.procs_at.elapsed() >= Duration::from_secs(5);
        let missing = needed.iter().any(|pid| !self.procs.contains_key(pid));
        let retry_missing = missing && self.procs_at.elapsed() >= Duration::from_secs(1);
        let refresh = self.procs.is_empty() || retry_missing || (stale && self.tick % 2 == 0);
        if refresh {
            self.procs = process_table()?;
            self.procs_at = Instant::now();
        }
        Ok(())
    }
}

/// 从 DXGI 枚举结果里挑一个"真的在被 PDH 报数"的适配器。
///
/// 本机会枚举出多个同名的克隆/虚拟适配器（容量也照抄 8GB），但 PDH 只报真实那一个；
/// 只按容量挑会挑到克隆体，导致所有数据被 LUID 过滤掉。
pub fn choose_primary(adapters: &[AdapterInfo]) -> Result<AdapterInfo, SampleError> {
    if adapters.is_empty() {
        return Err(SampleError::NoAdapter);
    }

    if let Ok(counters) = CounterSet::open(&[PATH_ADAPTER]) {
        let _ = counters.collect();
        std::thread::sleep(Duration::from_millis(120));
        if counters.collect().is_ok() {
            if let Ok(rows) = counters.read_array(0) {
                let mut best: Option<(u64, u64)> = None; // (luid, 已用字节)
                for (name, value) in &rows {
                    let Some(instance) = parse_instance(name) else {
                        continue;
                    };
                    if instance.phys != 0 || *value < 0.0 {
                        continue;
                    }
                    let used = *value as u64;
                    let better = best.map_or(true, |(_, best_used)| used > best_used);
                    if better {
                        best = Some((instance.luid, used));
                    }
                }
                if let Some((luid, _)) = best {
                    if let Some(adapter) = adapters
                        .iter()
                        .find(|a| a.luid == luid && a.dedicated_bytes > 0)
                    {
                        return Ok(adapter.clone());
                    }
                }
            }
        }
    }

    // 兜底：PDH 没给出可用 LUID 时，按容量挑非软件适配器。
    crate::vendor::dxgi::primary(adapters)
        .cloned()
        .ok_or(SampleError::NoAdapter)
}

/// 读不到 GPU 计数器时的降级采样：按工作集（内存）排。
pub struct MemorySampler {
    procs: HashMap<u32, ProcessInfo>,
    procs_at: Instant,
    meta_cache: HashMap<String, Option<FileMeta>>,
}

impl MemorySampler {
    pub fn new() -> Self {
        Self {
            procs: HashMap::new(),
            procs_at: Instant::now(),
            meta_cache: HashMap::new(),
        }
    }

    pub fn sample(&mut self) -> Result<Snapshot, SampleError> {
        if self.procs.is_empty() || self.procs_at.elapsed() >= Duration::from_secs(5) {
            self.procs = process_table()?;
            self.procs_at = Instant::now();
        }
        let window_pids = super::windows::visible_window_pids();
        let mut samples = Vec::new();
        for &pid in self.procs.keys() {
            if pid == 0 || pid == 4 {
                continue;
            }
            let Some(bytes) = working_set(pid) else {
                continue;
            };
            // 太小的进程不占版面；内存模式只是降级视图。
            if bytes < 8 * 1024 * 1024 {
                continue;
            }
            let mut sample = base_sample(pid, &self.procs, &window_pids);
            sample.local_bytes = bytes;
            samples.push(sample);
        }
        samples.sort_unstable_by_key(|sample| std::cmp::Reverse(sample.local_bytes));
        let file_meta = collect_meta(&samples, &mut self.meta_cache);
        let total_local = samples.iter().map(|sample| sample.local_bytes).sum();
        let groups = build_groups(&samples);
        Ok(Snapshot {
            mode: Mode::Memory,
            adapter_name: String::new(),
            capacity_bytes: 0,
            adapter_used: None,
            total_local,
            groups,
            unattributed: Unattributed::Unavailable,
            file_meta,
        })
    }
}

enum Worker {
    Gpu(Sampler),
    Memory(MemorySampler),
}

impl Worker {
    fn sample(&mut self) -> Result<Snapshot, SampleError> {
        match self {
            Worker::Gpu(sampler) => sampler.sample(),
            Worker::Memory(sampler) => sampler.sample(),
        }
    }
}

fn working_set(pid: u32) -> Option<u64> {
    unsafe {
        let handle = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ,
            false,
            pid,
        )
        .ok()?;
        let mut counters = PROCESS_MEMORY_COUNTERS::default();
        let result = GetProcessMemoryInfo(
            handle,
            &mut counters,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        );
        let _ = CloseHandle(handle);
        result.ok()?;
        Some(counters.WorkingSetSize as u64)
    }
}

/// 每轮最多查 2 个新程序名，避免第一次采样被版本资源读取拖慢。
const META_QUERIES_PER_TICK: usize = 2;

fn collect_meta(
    samples: &[ProcSample],
    cache: &mut HashMap<String, Option<FileMeta>>,
) -> HashMap<String, FileMeta> {
    let mut out = HashMap::new();
    let mut queried = 0;
    for sample in samples {
        let key = sample.name.to_ascii_lowercase();
        if let Some(cached) = cache.get(&key) {
            if let Some(meta) = cached {
                out.insert(key, meta.clone());
            }
            continue;
        }
        if queried >= META_QUERIES_PER_TICK {
            continue;
        }
        queried += 1;
        let meta = super::filemeta::query(sample.pid);
        cache.insert(key.clone(), meta.clone());
        if let Some(meta) = meta {
            out.insert(key, meta);
        }
    }
    out
}

/// 采样线程句柄。UI 线程持有；Drop 时先发停止信号，再最多等 2 秒。
pub struct SamplerThread {
    wake: HANDLE,
    stop: HANDLE,
    done: HANDLE,
}

impl SamplerThread {
    /// 立刻采一轮，不等定时器。
    pub fn request_sample(&self) {
        unsafe {
            let _ = SetEvent(self.wake);
        }
    }
}

impl Drop for SamplerThread {
    fn drop(&mut self) {
        unsafe {
            let _ = SetEvent(self.stop);
            let _ = WaitForSingleObject(self.done, 2000);
            let _ = CloseHandle(self.wake);
            let _ = CloseHandle(self.stop);
            let _ = CloseHandle(self.done);
        }
    }
}

pub fn spawn(hwnd: HWND) -> Result<(SamplerThread, Receiver<Snapshot>), SampleError> {
    let wake =
        unsafe { CreateEventW(None, false, false, None) }.map_err(|_| SampleError::Thread)?;
    let stop =
        unsafe { CreateEventW(None, false, false, None) }.map_err(|_| SampleError::Thread)?;
    let done =
        unsafe { CreateEventW(None, false, false, None) }.map_err(|_| SampleError::Thread)?;

    let (tx, rx) = mpsc::channel::<Snapshot>();

    // SAFETY: HANDLE 是内核对象句柄，SetEvent/WaitFor* 本身跨线程安全；
    // 窗口句柄只用于 PostMessageW，同样是线程安全的跨线程调用。
    let send_wake = SendHandle(wake);
    let send_stop = SendHandle(stop);
    let send_done = SendHandle(done);
    let send_hwnd = SendHwnd(hwnd);

    std::thread::Builder::new()
        .name("vram-sampler".into())
        .spawn(move || sampler_loop(send_hwnd, send_wake, send_stop, send_done, tx))
        .map_err(|_| SampleError::Thread)?;

    Ok((SamplerThread { wake, stop, done }, rx))
}

fn sampler_loop(
    hwnd: SendHwnd,
    wake: SendHandle,
    stop: SendHandle,
    done: SendHandle,
    tx: Sender<Snapshot>,
) {
    let adapters = crate::vendor::dxgi::enumerate().unwrap_or_default();
    let adapter = choose_primary(&adapters).ok();

    let mut worker = match adapter {
        Some(adapter) => match Sampler::new(adapter) {
            Ok(sampler) => Worker::Gpu(sampler),
            // 没有显卡计数器（Win7 / 驱动不提供）→ 降级为内存视图。
            Err(SampleError::Open(_)) => Worker::Memory(MemorySampler::new()),
            Err(err) => {
                post(hwnd.0, WM_SAMPLE_FAILED, err.code(), 0);
                unsafe {
                    let _ = SetEvent(done.0);
                }
                return;
            }
        },
        None => Worker::Memory(MemorySampler::new()),
    };

    loop {
        let wait = unsafe { WaitForMultipleObjects(&[stop.0, wake.0], false, INFINITE) };
        if wait == WAIT_OBJECT_0 {
            break;
        }

        match worker.sample() {
            Ok(snapshot) => {
                if tx.send(snapshot).is_err() {
                    break;
                }
                post(hwnd.0, WM_SNAPSHOT_READY, 0, 0);
            }
            Err(err) => post(hwnd.0, WM_SAMPLE_FAILED, err.code(), 0),
        }
    }

    unsafe {
        let _ = SetEvent(done.0);
    }
}

fn post(hwnd: HWND, message: u32, wparam: usize, lparam: isize) {
    unsafe {
        let _ = PostMessageW(
            hwnd,
            message,
            windows::Win32::Foundation::WPARAM(wparam),
            windows::Win32::Foundation::LPARAM(lparam),
        );
    }
}

fn base_sample(
    pid: u32,
    procs: &HashMap<u32, ProcessInfo>,
    window_pids: &HashSet<u32>,
) -> ProcSample {
    let info = procs.get(&pid);
    let mut name = info
        .map(|i| i.name.clone())
        .unwrap_or_else(|| "?".to_string());
    if pid == 4 {
        // T3：PID 4 是内核，别让它和无权限进程一样显示成问号。
        name = "[kernel]".to_string();
    }
    ProcSample {
        pid,
        name,
        parent_pid: info.map(|i| i.parent).unwrap_or(0),
        local_bytes: 0,
        nonlocal_bytes: 0,
        gpu_pct: None,
        has_window: window_pids.contains(&pid),
        start_time: process_start_time(pid),
    }
}

/// 创建时间（FILETIME 原值）。拿不到就是 `None`——杀进程前会因此拒绝操作。
fn process_start_time(pid: u32) -> Option<u64> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let result = GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user);
        let _ = CloseHandle(handle);
        result.ok()?;
        Some(((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64)
    }
}

fn adapter_used_bytes(rows: &[(String, f64)], luid: u64) -> Option<u64> {
    let mut best: Option<f64> = None;
    for (name, value) in rows {
        let Some(instance) = parse_instance(name) else {
            continue;
        };
        if instance.luid != luid || instance.phys != 0 || *value < 0.0 {
            continue;
        }
        best = Some(best.map_or(*value, |current: f64| current.max(*value)));
    }
    best.map(|value| value as u64)
}

fn process_table() -> Result<HashMap<u32, ProcessInfo>, SampleError> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
        .map_err(|_| SampleError::ProcessSnapshot)?;

    let mut table = HashMap::new();
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };

    let mut ok = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
    while ok {
        let name = wide_z(&entry.szExeFile);
        table.insert(
            entry.th32ProcessID,
            ProcessInfo {
                name,
                parent: entry.th32ParentProcessID,
            },
        );
        ok = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
    }

    unsafe {
        let _ = CloseHandle(snapshot);
    }
    Ok(table)
}

fn wide_z(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

// SAFETY: 这两个包装只用于把"句柄 / 窗口句柄"搬进采样线程；
// 它们本身不提供内存共享，所有 Win32 调用都是线程安全的。
#[derive(Clone, Copy)]
struct SendHandle(HANDLE);
unsafe impl Send for SendHandle {}

#[derive(Clone, Copy)]
struct SendHwnd(HWND);
unsafe impl Send for SendHwnd {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "需要真实显卡与 WDDM 计数器；手动跑：cargo test -- --ignored --nocapture"]
    fn live_sample_prints_snapshot() {
        let adapters = crate::vendor::dxgi::enumerate().expect("DXGI 枚举失败");
        for adapter in &adapters {
            eprintln!(
                "适配器: {:<32} luid={:#018x} 容量={} MB 虚拟={}",
                adapter.name,
                adapter.luid,
                adapter.dedicated_bytes / 1024 / 1024,
                adapter.is_software
            );
        }
        let primary = choose_primary(&adapters).expect("没有可用显卡");
        eprintln!("主适配器: {} luid={:#018x}", primary.name, primary.luid);
        let mut sampler = Sampler::new(primary).expect("计数器打不开");
        for _ in 0..5 {
            let start = std::time::Instant::now();
            let snapshot = sampler.sample().expect("采样失败");
            eprintln!(
                "采样耗时 {:.1} ms / {} 组 / 本地合计 {} MB / 未归属 {:?}",
                start.elapsed().as_secs_f64() * 1000.0,
                snapshot.groups.len(),
                snapshot.total_local / 1024 / 1024,
                snapshot.unattributed
            );
            std::thread::sleep(Duration::from_millis(1000));
        }
        let snapshot = sampler.sample().expect("采样失败");

        eprintln!(
            "\n{} 容量 {} MB / 已用 {:?} MB / 进程合计 {} MB",
            snapshot.adapter_name,
            snapshot.capacity_bytes / 1024 / 1024,
            snapshot.adapter_used.map(|b| b / 1024 / 1024),
            snapshot.total_local / 1024 / 1024
        );
        for group in &snapshot.groups {
            eprintln!(
                "{:<30} {:>8} MB  {:?}",
                group.title,
                group.local_bytes / 1024 / 1024,
                group.members
            );
        }
        eprintln!("未归属: {:?}", snapshot.unattributed);
    }
}
