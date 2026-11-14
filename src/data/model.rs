//! 纯数据模型与聚合算法。
//!
//! 这里不允许出现 Win32、不读系统、不碰窗口——只做"把采样结果整理成人能看的分组"。

use std::collections::HashMap;

/// 单个进程的原始采样值。显存单位一律字节。
#[derive(Debug, Clone)]
pub struct ProcSample {
    pub pid: u32,
    pub name: String,
    pub parent_pid: u32,
    /// 主口径：`Local Usage`。
    pub local_bytes: u64,
    /// 被挤到内存条里的部分：`Non Local Usage`。
    pub nonlocal_bytes: u64,
    /// 最忙引擎的占用率，不是求和。
    pub gpu_pct: Option<f32>,
}

/// 进程树聚合后的一行。
#[derive(Debug, Clone)]
pub struct AppGroup {
    pub root_pid: u32,
    pub title: String,
    pub local_bytes: u64,
    /// 组内最忙的进程，取 max 不求和。
    pub gpu_pct: Option<f32>,
    pub members: Vec<u32>,
}

/// 未归属（整卡 − Σ本地）的三种状态，**永不为负**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unattributed {
    Bytes(u64),
    Overcounted(u64),
    Unavailable,
}

/// 一次采样。
#[derive(Debug)]
pub struct Snapshot {
    pub adapter_name: String,
    pub capacity_bytes: u64,
    /// 整卡已用。读不到时是 `None`——不是 0。
    pub adapter_used: Option<u64>,
    pub total_local: u64,
    pub groups: Vec<AppGroup>,
    pub unattributed: Unattributed,
}

/// 进程树里作为"墙"的系统进程：它们自己成组，不把子进程并进来。
const SYSTEM_PARENT_BARRIER: &[&str] = &[
    "explorer.exe",
    "svchost.exe",
    "services.exe",
    "wininit.exe",
    "csrss.exe",
    "smss.exe",
    "winlogon.exe",
    "dwm.exe",
];

/// 按进程树把持有显存的进程聚成 AppGroup。
///
/// 规则（ARCHITECTURE §6.1）：沿父链向上，直到父进程不持有显存、或父进程是系统进程；
/// 到达的最上层即组根。
pub fn build_groups(samples: &[ProcSample]) -> Vec<AppGroup> {
    let index: HashMap<u32, &ProcSample> = samples.iter().map(|s| (s.pid, s)).collect();
    let mut root_of: HashMap<u32, u32> = HashMap::with_capacity(samples.len());

    for sample in samples {
        let mut root = sample.pid;
        let mut cursor = sample;
        for _ in 0..64 {
            let Some(parent) = index.get(&cursor.parent_pid).copied() else {
                break;
            };
            if parent.pid == 0 || parent.pid == cursor.pid || is_barrier(&parent.name) {
                break;
            }
            root = parent.pid;
            cursor = parent;
        }
        root_of.insert(sample.pid, root);
    }

    let mut groups: HashMap<u32, AppGroup> = HashMap::new();
    for sample in samples {
        let root = root_of[&sample.pid];
        let group = groups.entry(root).or_insert_with(|| AppGroup {
            root_pid: root,
            title: String::new(),
            local_bytes: 0,
            gpu_pct: None,
            members: Vec::new(),
        });
        group.local_bytes += sample.local_bytes;
        if let Some(pct) = sample.gpu_pct {
            group.gpu_pct = Some(group.gpu_pct.map_or(pct, |old| old.max(pct)));
        }
        group.members.push(sample.pid);
    }

    let mut out: Vec<AppGroup> = groups
        .into_values()
        .map(|mut group| {
            group.members.sort_unstable();
            let name = index
                .get(&group.root_pid)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "?".to_string());
            group.title = if group.members.len() > 1 {
                format!("{name} × {}", group.members.len())
            } else {
                name
            };
            group
        })
        .collect();
    out.sort_by(|a, b| {
        b.local_bytes
            .cmp(&a.local_bytes)
            .then_with(|| a.title.cmp(&b.title))
    });
    out
}

fn is_barrier(name: &str) -> bool {
    SYSTEM_PARENT_BARRIER
        .iter()
        .any(|barrier| name.eq_ignore_ascii_case(barrier))
}

/// 整卡已用 − Σ(本地)。负值不是"负数"，是口径异常。
pub fn unattributed(adapter_used: u64, total_local: u64) -> Unattributed {
    if total_local <= adapter_used {
        Unattributed::Bytes(adapter_used - total_local)
    } else {
        Unattributed::Overcounted(total_local - adapter_used)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(pid: u32, parent_pid: u32, name: &str, mb: u64) -> ProcSample {
        ProcSample {
            pid,
            parent_pid,
            name: name.to_string(),
            local_bytes: mb * 1024 * 1024,
            nonlocal_bytes: 0,
            gpu_pct: None,
        }
    }

    #[test]
    fn groups_chrome_tree_into_one_row() {
        let samples = vec![
            sample(100, 50, "chrome.exe", 1000),
            sample(101, 100, "chrome.exe", 400),
            sample(102, 100, "chrome.exe", 200),
        ];
        let groups = build_groups(&samples);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].root_pid, 100);
        assert_eq!(groups[0].title, "chrome.exe × 3");
        assert_eq!(groups[0].local_bytes, 1600 * 1024 * 1024);
        assert_eq!(groups[0].members, vec![100, 101, 102]);
    }

    #[test]
    fn system_process_parent_is_a_barrier() {
        // 50 是 explorer.exe（系统进程），100 自己成组，101 并进 100。
        let samples = vec![
            sample(100, 50, "chrome.exe", 1000),
            sample(101, 100, "chrome.exe", 400),
        ];
        let groups = build_groups(&samples);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].root_pid, 100);
        assert_eq!(groups[0].title, "chrome.exe × 2");
    }

    #[test]
    fn dwm_stays_alone_even_with_children() {
        // 父进程是系统进程时，子进程不再往上并——否则任何程序都会并进 dwm。
        let samples = vec![
            sample(200, 4, "dwm.exe", 200),
            sample(201, 200, "child.exe", 50),
        ];
        let groups = build_groups(&samples);
        assert_eq!(groups.len(), 2);
        assert!(groups.iter().any(|g| g.root_pid == 200));
        assert!(groups.iter().any(|g| g.root_pid == 201));
    }

    #[test]
    fn missing_parent_does_not_merge() {
        let samples = vec![sample(300, 999, "python.exe", 512)];
        let groups = build_groups(&samples);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].root_pid, 300);
        assert_eq!(groups[0].title, "python.exe");
    }

    #[test]
    fn sorts_by_local_bytes_desc() {
        let samples = vec![sample(1, 0, "small.exe", 10), sample(2, 0, "big.exe", 900)];
        let groups = build_groups(&samples);
        assert_eq!(groups[0].title, "big.exe");
    }

    #[test]
    fn unattributed_never_negative() {
        assert_eq!(unattributed(1000, 800), Unattributed::Bytes(200));
        assert_eq!(unattributed(1000, 1000), Unattributed::Bytes(0));
        assert_eq!(unattributed(1000, 1200), Unattributed::Overcounted(200));
    }
}
