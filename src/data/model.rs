//! 纯数据模型与聚合算法。
//!
//! 这里不允许出现 Win32、不读系统、不碰窗口——只做"把采样结果整理成人能看的分组"。

use std::collections::HashMap;

/// 进程的唯一键：PID 会被复用，杀进程前必须核对创建时间。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub pid: u32,
    pub start_time: Option<u64>,
}

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
    /// 是否有可见的顶层窗口（用于"有窗口 = 谨慎"的判断）。
    pub has_window: bool,
    /// 进程创建时间（FILETIME），用于防 PID 复用。
    pub start_time: Option<u64>,
}

/// 组内单个进程。展开分组时显示。
#[derive(Debug, Clone)]
pub struct GroupMember {
    pub pid: u32,
    pub name: String,
    pub local_bytes: u64,
    pub nonlocal_bytes: u64,
    pub gpu_pct: Option<f32>,
    pub has_window: bool,
    pub start_time: Option<u64>,
}

impl GroupMember {
    pub fn target(&self) -> Target {
        Target {
            pid: self.pid,
            start_time: self.start_time,
        }
    }
}

/// 进程树聚合后的一行。
#[derive(Debug, Clone)]
pub struct AppGroup {
    pub root_pid: u32,
    pub root_name: String,
    /// 显示用标题，如 "chrome.exe × 12"。
    pub title: String,
    pub local_bytes: u64,
    pub nonlocal_bytes: u64,
    /// 组内最忙的进程，取 max 不求和。
    pub gpu_pct: Option<f32>,
    /// 组内任一进程有可见窗口。
    pub has_window: bool,
    pub members: Vec<GroupMember>,
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
            root_name: index
                .get(&root)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "?".to_string()),
            title: String::new(),
            local_bytes: 0,
            nonlocal_bytes: 0,
            gpu_pct: None,
            has_window: false,
            members: Vec::new(),
        });
        group.local_bytes += sample.local_bytes;
        group.nonlocal_bytes += sample.nonlocal_bytes;
        group.has_window |= sample.has_window;
        if let Some(pct) = sample.gpu_pct {
            group.gpu_pct = Some(group.gpu_pct.map_or(pct, |old| old.max(pct)));
        }
        group.members.push(GroupMember {
            pid: sample.pid,
            name: sample.name.clone(),
            local_bytes: sample.local_bytes,
            nonlocal_bytes: sample.nonlocal_bytes,
            gpu_pct: sample.gpu_pct,
            has_window: sample.has_window,
            start_time: sample.start_time,
        });
    }

    let mut out: Vec<AppGroup> = groups
        .into_values()
        .map(|mut group| {
            group
                .members
                .sort_unstable_by_key(|member| std::cmp::Reverse(member.local_bytes));
            group.title = if group.members.len() > 1 {
                format!("{} × {}", group.root_name, group.members.len())
            } else {
                group.root_name.clone()
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

/// 按"程序名"汇总的一行（第二页）。
#[derive(Debug, Clone)]
pub struct ProgramTotal {
    /// 显示名，取第一次见到的写法。
    pub name: String,
    /// 小写唯一键。
    pub key: String,
    pub local_bytes: u64,
    pub nonlocal_bytes: u64,
    pub gpu_pct: Option<f32>,
    pub has_window: bool,
    pub members: Vec<GroupMember>,
}

/// 把进程树分组再按程序名合并——同一程序的多个进程/多个分组只留一行。
pub fn build_programs(groups: &[AppGroup]) -> Vec<ProgramTotal> {
    let mut map: std::collections::HashMap<String, ProgramTotal> = std::collections::HashMap::new();
    for group in groups {
        for member in &group.members {
            let key = member.name.to_ascii_lowercase();
            let entry = map.entry(key.clone()).or_insert_with(|| ProgramTotal {
                name: member.name.clone(),
                key,
                local_bytes: 0,
                nonlocal_bytes: 0,
                gpu_pct: None,
                has_window: false,
                members: Vec::new(),
            });
            entry.local_bytes = entry.local_bytes.saturating_add(member.local_bytes);
            entry.nonlocal_bytes = entry.nonlocal_bytes.saturating_add(member.nonlocal_bytes);
            entry.has_window |= member.has_window;
            if let Some(pct) = member.gpu_pct {
                entry.gpu_pct = Some(entry.gpu_pct.map_or(pct, |old| old.max(pct)));
            }
            entry.members.push(member.clone());
        }
    }

    let mut out: Vec<ProgramTotal> = map.into_values().collect();
    for program in &mut out {
        program.members.sort_unstable_by_key(|member| member.pid);
    }
    out.sort_by(|a, b| {
        b.local_bytes
            .cmp(&a.local_bytes)
            .then_with(|| a.name.cmp(&b.name))
    });
    out
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
            has_window: false,
            start_time: None,
        }
    }

    fn member_pids(group: &AppGroup) -> Vec<u32> {
        group.members.iter().map(|m| m.pid).collect()
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
        assert_eq!(member_pids(&groups[0]), vec![100, 101, 102]);
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
    fn keeps_member_split_and_window_flag() {
        let mut a = sample(10, 0, "chrome.exe", 100);
        a.has_window = true;
        let b = sample(11, 10, "chrome.exe", 300);
        let groups = build_groups(&[b, a]);
        assert!(groups[0].has_window);
        assert_eq!(groups[0].members[0].pid, 11);
        assert_eq!(groups[0].members[1].local_bytes, 100 * 1024 * 1024);
    }

    #[test]
    fn unattributed_never_negative() {
        assert_eq!(unattributed(1000, 800), Unattributed::Bytes(200));
        assert_eq!(unattributed(1000, 1000), Unattributed::Bytes(0));
        assert_eq!(unattributed(1000, 1200), Unattributed::Overcounted(200));
    }

    #[test]
    fn programs_sum_all_processes_with_same_name() {
        let samples = vec![
            sample(10, 0, "chrome.exe", 100),
            sample(11, 10, "chrome.exe", 300),
            sample(12, 0, "Code.exe", 50),
        ];
        let groups = build_groups(&samples);
        let programs = build_programs(&groups);
        assert_eq!(programs.len(), 2);
        assert_eq!(programs[0].name, "chrome.exe");
        assert_eq!(programs[0].local_bytes, 400 * 1024 * 1024);
        assert_eq!(programs[0].members.len(), 2);
        assert_eq!(programs[1].name, "Code.exe");
        assert_eq!(programs[1].local_bytes, 50 * 1024 * 1024);
    }
}
