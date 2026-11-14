//! 采样：常驻 PDH 查询 + 一轮一次进程快照，产出 `Snapshot`。
//!
//! 线程模型见 `docs/ARCHITECTURE.md` §4：采样线程不碰任何窗口；
//! 数据经 `mpsc` 传回 UI，`PostMessageW` 只做唤醒。

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, WAIT_OBJECT_0};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    CreateEventW, SetEvent, WaitForMultipleObjects, WaitForSingleObject,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::vendor::dxgi::AdapterInfo;
use crate::vendor::pdh::{parse_instance, CounterSet};

use super::model::{build_groups, unattributed, ProcSample, Snapshot, Unattributed};
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
}

impl Sampler {
    pub fn new(adapter: AdapterInfo) -> Result<Self, SampleError> {
        let counters = CounterSet::open(&[PATH_LOCAL, PATH_NONLOCAL, PATH_ENGINE, PATH_ADAPTER])
            .map_err(SampleError::Open)?;
        // 速率型计数器（GPU%）需要两个采样点才有值；先采一次，稍后 sample() 再采。
        let _ = counters.collect();
        std::thread::sleep(Duration::from_millis(120));
        Ok(Self { counters, adapter })
    }

    /// 采一轮。任何一条数据读不到都宁可缺失，也不写 0。
    pub fn sample(&mut self) -> Result<Snapshot, SampleError> {
        self.counters.collect().map_err(SampleError::Collect)?;
        let local = self.counters.read_array(0).map_err(SampleError::Read)?;
        let nonlocal = self.counters.read_array(1).map_err(SampleError::Read)?;
        let engine = self.counters.read_array(2).map_err(SampleError::Read)?;
        let adapter_rows = self.counters.read_array(3).map_err(SampleError::Read)?;

        let procs = process_table()?;
        let want_luid = self.adapter.luid;

        // pid -> 采样值；只保留有显存的进程（M0：314 个进程里只有 32 个）。
        let mut per_pid: HashMap<u32, ProcSample> = HashMap::new();

        for (name, value) in &local {
            let Some(instance) = parse_instance(name) else {
                continue;
            };
            if instance.luid != want_luid || *value < 0.5 {
                continue;
            }
            let Some(pid) = instance.pid else { continue };
            let entry = per_pid
                .entry(pid)
                .or_insert_with(|| base_sample(pid, &procs));
            entry.local_bytes = entry.local_bytes.saturating_add(*value as u64);
        }

        for (name, value) in &nonlocal {
            let Some(instance) = parse_instance(name) else {
                continue;
            };
            if instance.luid != want_luid || *value < 0.5 {
                continue;
            }
            let Some(pid) = instance.pid else { continue };
            let entry = per_pid
                .entry(pid)
                .or_insert_with(|| base_sample(pid, &procs));
            entry.nonlocal_bytes = entry.nonlocal_bytes.saturating_add(*value as u64);
        }

        // GPU% 取最忙引擎，不能求和。
        for (name, value) in &engine {
            let Some(instance) = parse_instance(name) else {
                continue;
            };
            if instance.luid != want_luid {
                continue;
            }
            let Some(pid) = instance.pid else { continue };
            let pct = (*value as f32).clamp(0.0, 100.0);
            let entry = per_pid
                .entry(pid)
                .or_insert_with(|| base_sample(pid, &procs));
            entry.gpu_pct = Some(entry.gpu_pct.map_or(pct, |old| old.max(pct)));
        }

        let mut samples: Vec<ProcSample> = per_pid
            .into_values()
            .filter(|s| s.local_bytes > 0)
            .collect();
        samples.sort_unstable_by_key(|s| std::cmp::Reverse(s.local_bytes));

        let total_local: u64 = samples.iter().map(|s| s.local_bytes).sum();
        let groups = build_groups(&samples);
        let adapter_used = adapter_used_bytes(&adapter_rows, want_luid);
        let unattributed = match adapter_used {
            Some(used) => unattributed(used, total_local),
            None => Unattributed::Unavailable,
        };

        Ok(Snapshot {
            adapter_name: self.adapter.name.clone(),
            capacity_bytes: self.adapter.dedicated_bytes,
            adapter_used,
            total_local,
            groups,
            unattributed,
        })
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
    let adapter = match choose_primary(&adapters) {
        Ok(adapter) => adapter,
        Err(err) => {
            post(hwnd.0, WM_SAMPLE_FAILED, err.code(), 0);
            unsafe {
                let _ = SetEvent(done.0);
            }
            return;
        }
    };

    let mut sampler = match Sampler::new(adapter) {
        Ok(sampler) => sampler,
        Err(err) => {
            post(hwnd.0, WM_SAMPLE_FAILED, err.code(), 0);
            unsafe {
                let _ = SetEvent(done.0);
            }
            return;
        }
    };

    loop {
        let wait = unsafe { WaitForMultipleObjects(&[stop.0, wake.0], false, INFINITE) };
        if wait == WAIT_OBJECT_0 {
            break;
        }

        match sampler.sample() {
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

fn base_sample(pid: u32, procs: &HashMap<u32, ProcessInfo>) -> ProcSample {
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
