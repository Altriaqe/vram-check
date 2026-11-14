# 架构文档

| 项 | 内容 |
|---|---|
| 版本 | v1.0 |
| 对应 PRD | [PRD.md](PRD.md) v1.0 |
| 一句话 | 一个进程、两个线程、四层结构，所有设计都为"更小、更快、不占进程"让路 |

---

## 1. 约束怎么驱动架构

架构不是画出来的，是被三条硬约束逼出来的。

| 约束 | 直接被否掉的设计 | 我们改成 |
|---|---|---|
| ≤2 MB | Electron / Tauri / egui / .NET / Python | Win32 原生控件 + 系统字体 |
| ≤2 MB | 内嵌中文字体（+5~10MB） | `CreateFontW("Microsoft YaHei")` 用系统的 |
| ≤100 ms 启动 | 运行时初始化、字体解析、GPU 上下文 | GDI 不需要任何渲染器 |
| 不占进程 | 托盘常驻 + 全局热键 | 关窗即退，用"启动快"补偿 |
| 不占进程 | 后台服务 / 定时器轮询 | 失焦即停轮询 |
| 单进程 | 独立采样进程 | 进程内一个采样线程 |

**推论**：任何"加个库就行"的方案，先算体积；任何"后台跑着"的方案，先问能不能不用。

---

## 2. 分层

```
┌───────────────────────────────────────────────────────┐
│  ui/          原生窗口 + ListView + 文案渲染           │  只碰 Win32
├───────────────────────────────────────────────────────┤
│  explain/     规则库 · 签名 · 启发式 · 风险分级        │  纯函数，可单测
├───────────────────────────────────────────────────────┤
│  action/      优雅关闭 · 强制 · 实测回收               │  唯一的"写"操作
├───────────────────────────────────────────────────────┤
│  data/        采样 · 聚合 · 容量 · 能力探测            │  唯一的"读"操作
├───────────────────────────────────────────────────────┤
│  vendor/      hypomnesis 的 PDH/DXGI 移植（改过口径）  │  隔离区，见 §7
└───────────────────────────────────────────────────────┘
```

**依赖方向只能向下**。`ui` 不直接调 PDH，`data` 不知道有窗口存在。

---

## 3. 数据流：一次采样的生命周期

```
  WM_TIMER (UI 线程)
        │  仅当窗口可见且前台
        ▼
  [1] 唤醒采样线程 (SetEvent)
        │
        ▼
  [2] PdhCollectQueryData          ← 常驻查询，只建一次
        │
        ▼
  [3] 枚举实例名 → 解析 pid         ← 增量：新 pid 才 AddCounter
        │
        ▼
  [4] 读 Local / NonLocal → 按 pid 聚合
        │
        ▼
  [5] Toolhelp32 一次快照 → 名字 + 父 PID + 创建时间
        │
        ▼
  [6] 按进程树分组成 AppGroup
        │
        ▼
  [7] 拼描述（规则库命中 or 降级到签名/启发式）
        │
        ▼
  [8] mpsc 发送 Snapshot + PostMessageW 唤醒 UI
        │
        ▼
  [9] UI 只 InvalidateRect 变化的行
```

**关键设计**：
- 第 2 步的 PDH 查询对象**进程生命周期内只建一次**。每轮重建查询是同类工具卡顿的主因。
- 第 5 步 Toolhelp32 **一轮只做一次快照**，不是每进程一次。
- 第 8 步用 `mpsc` 传数据、`PostMessageW` 只做唤醒信号——**不传裸指针**，避免生命周期问题。

---

## 4. 线程模型

| 线程 | 职责 | 禁止 |
|---|---|---|
| **UI 线程** | 窗口消息、ListView、所有 Win32 控件调用 | 不做任何阻塞采样 |
| **采样线程** | PDH 采集、Toolhelp32、聚合 | **不碰任何 HWND** |

```
UI 线程                         采样线程
   │                               │
   │── SetEvent(wake) ────────────►│
   │                               │ 采样 (~10ms)
   │◄──── mpsc::send(Snapshot) ────│
   │◄──── PostMessageW(WM_APP) ────│ (仅唤醒)
   │                               │
   │  recv() 取快照，重绘           │ WaitForSingleObject(1s 或 stop)
```

**三条规则**：
1. 采样线程**永不**调用 `SendMessage`（会死锁）。
2. UI 线程**永不** `join` 采样线程（会卡界面）——退出时先 `SetEvent(stop)` 再等。
3. 失焦/最小化 → 采样线程 `Wait` 不超时，CPU 归零。

**不用 async**：没有网络、没有并发 IO，async 只会引入 `tokio`/`futures` 把体积顶穿。

---

## 5. 核心数据结构

```rust
// data/model.rs
pub struct AdapterInfo {
    pub luid: (i32, u32),
    pub name: String,
    pub dedicated_bytes: u64,   // DXGI DedicatedVideoMemory —— 唯一的容量来源
    pub is_software: bool,      // 虚拟显卡（ToDesk/向日葵…）标记为 true
}

pub struct ProcSample {
    pub pid: u32,
    pub name: String,           // Toolhelp32；SYSTEM 进程也能拿到
    pub local_bytes: u64,       // 主口径
    pub nonlocal_bytes: u64,    // 共享内存
    pub dedicated_bytes: Option<u64>, // 仅"技术详情"用
    pub gpu_pct: Option<f32>,   // max(engine)，不是 sum
    pub start_time: Option<u64>,
}

pub struct AppGroup {
    pub root_pid: u32,
    pub title: String,          // "python.exe × 3"
    pub local_bytes: u64,
    pub members: Vec<u32>,
    pub risk: Risk,
    pub desc: String,
    pub evidence: Evidence,     // 描述依据，供"点开看为什么"
}

pub enum Risk { Forbidden, Caution, Safe, Unknown }

pub struct Snapshot {
    pub taken_at: Instant,
    pub capacity_bytes: u64,
    pub adapter_used: u64,
    pub groups: Vec<AppGroup>,
    pub unattributed: Unattributed,
}

pub enum Unattributed {
    Bytes(u64),
    Overcounted(u64),   // Σ > 整卡：口径异常，不显示负数
    Unavailable,        // 整卡读不到
}
```

> **M1 实现差异（2026-11-14）**：`ProcSample` 暂无 `dedicated_bytes` / `start_time`（技术详情与 PID 复用检测留到 M2/M3），增加 `parent_pid` 用于分组；`AppGroup` 暂只保留 `root_pid/title/local_bytes/gpu_pct/members`，`risk/desc` 由 UI 层调用 `explain` 现算（避免 `data` 反向依赖 `explain`）；`Snapshot` 的 `taken_at` 等做历史曲线时再加，`adapter_used` 改为 `Option<u64>`——读不到时是 `None`，不是 `0`。

---

## 6. 算法

### 6.1 进程树聚合

单看 PID 没有意义——Chrome 有 12 个进程，用户要的是"Chrome 占了 1.6GB"。

```
对每个持有显存的 pid：
  沿 parent 链向上走，直到遇到「父进程不持有显存」或「父进程是系统进程」
  该父节点的子节点即组根
若整条链都持有显存 → 用自身作为组根
```

**陷阱**：PID 会被复用。必须用 `(pid, start_time)` 做联合键，不只靠 pid。

### 6.2 GPU%

```
gpu_pct(pid) = max( 该 pid 所有引擎实例的 Utilization Percentage )
```

**不能求和**：一个进程同时有 `3d` 和 `copy` 引擎，相加会超过 100%。这符合 Task Manager 与 gputaskman 的口径。

注意 `CUDA` 的负载常被计入 `3d` 引擎——不要假设"训练任务应该显示在 compute 引擎上"。

### 6.3 未归属

```
unattributed = adapter_used - Σ(local_bytes)
```

三种输出，**永不给负数**：

| 情况 | 显示 |
|---|---|
| `>= 0` 且占比 > 5% | `0.4 GB`（正常一行） |
| `>= 0` 但占比 ≤ 5% | 弱化/隐藏（空闲时它就 43 MiB，不占版面） |
| `< 0` | "口径异常"，并在技术详情里给出两个口径的原始值 |
| 整卡读不到 | `—` |

### 6.4 容量

**只能**取 `DXGI_ADAPTER_DESC1.DedicatedVideoMemory`。

不要用 WMI `Win32_VideoController.AdapterRAM`——实测在 8GB 卡上报 4.00 GB（32 位字段截断）。

### 6.5 适配器筛选

一台普通机器上可能枚举出 5 个适配器，其中 4 个是虚拟显卡（ToDesk / MuMu / 向日葵 / GameViewer）。

```
保留条件：dedicated_bytes > 0
排序：dedicated_bytes 降序（独显优先）
虚拟显卡：不进"整卡"求和，但仍在列表里可见（标注"虚拟显示器"）
```

---

## 7. vendor 边界（最重要的纪律）

`vendor/` 是别人写的代码，我们的原则是**最小接触面**。

| 项 | 规则 |
|---|---|
| 移什么 | 只移 `pdh.rs` 的实例名解析 / LUID 过滤 / 两段式枚举，`dxgi.rs` 的适配器枚举 |
| 不移什么 | NVML / Metal / CLI / spill / JSON / nvidia-smi 子进程 |
| FFI 部分 | **自己写**。`probe/src/main.rs` 已验证过 `windows` crate 的调用形状，比移植更可靠 |
| 改动标注 | 每个改过的文件头加 `// Modified by VRAM Sentinel: <改了什么>` |
| 版权 | 保留原作者 copyright + `SPDX-License-Identifier: MIT OR Apache-2.0` |
| 同步 | `vendor/VENDOR.md` 记录上游 commit；每季度 diff 一次 |
| 目标 | **最终不再 fork**——把 Local 口径补丁提回上游，之后可切回依赖 |

---

## 8. 目录结构

```
vram-sentinel/
├── Cargo.toml
├── README.md
├── THIRD-PARTY-NOTICES.md
├── docs/
│   ├── PRD.md
│   ├── ARCHITECTURE.md
│   ├── CONVENTIONS.md
│   ├── DECISIONS.md
│   └── M0-MEASUREMENTS.md
└── src/
    ├── main.rs              入口、单实例互斥、DPI 感知
    ├── app.rs               生命周期、消息循环、托盘(可选)
    ├── vendor/
    │   ├── VENDOR.md        上游 commit / 改动清单
    │   ├── pdh.rs           改：Dedicated → Local
    │   └── dxgi.rs          原样
    ├── data/
    │   ├── probe.rs         启动时能力探测、降级判定
    │   ├── sampler.rs       常驻查询、增量 counter、采样线程
    │   └── model.rs         上面的数据结构 + 聚合算法
    ├── explain/
    │   ├── rules.tsv        200 条规则（数据）
    │   ├── rules.rs         加载与匹配
    │   ├── classify.rs      风险分级
    │   └── render.rs        文案拼装（铁律在这里落地）
    ├── action/
    │   └── close.rs         WM_CLOSE → 等待 → TerminateProcess → 实测回收
    └── ui/
        ├── window.rs        主窗口、头部总览条
        ├── listview.rs      owner-draw、排序、多选、键盘
        └── format.rs        数字格式化（GB/MB、可感类比）
```

---

## 9. 依赖与体积

```toml
[dependencies]
windows = { version = "0.58", features = [
    "Win32_Foundation",
    "Win32_System_Performance",      # PDH
    "Win32_Graphics_Dxgi",           # 适配器 + 容量
    "Win32_System_Diagnostics_ToolHelp",
    "Win32_System_Threading",
    "Win32_UI_WindowsAndMessaging",
    "Win32_UI_Controls",             # ListView
    "Win32_Graphics_Gdi",
    "Win32_System_LibraryLoader",
] }
```

**除此之外不引任何 crate。** 具体地：

| 想引 | 替代方案 | 省下 |
|---|---|---|
| `serde` / `serde_json` | 手写 TSV 解析（200 行内） | ~300 KB |
| `anyhow` / `thiserror` | 自定义小 enum | ~50 KB |
| `log` / `env_logger` | `#[cfg(feature = "debug")] eprintln!` | ~100 KB |
| `clap` | 没有命令行参数 | ~400 KB |
| `chrono` | 只需要时长，用 `Instant` | ~200 KB |

```toml
[profile.release]
opt-level = "z"
lto = "fat"
codegen-units = 1
panic = "abort"
strip = "symbols"
```

外加 `#![windows_subsystem = "windows"]`（不弹控制台）。

---

## 10. 性能预算

| 操作 | 预算 | 依据 |
|---|---|---|
| 进程启动到窗口可见 | ≤ 100 ms | Win32 无渲染器初始化 |
| 首次采样 | ≤ 30 ms | 32 个实例，PDH 全量采集 |
| 稳态单次采样 | ≤ 10 ms | 增量维护：只读值不重建 |
| Toolhelp32 快照 | ≤ 5 ms | 一轮一次 |
| 界面重绘 | ≤ 5 ms | 只失效变化行 |
| 常驻内存 | ≤ 30 MB | Rust 无 GC |
| 失焦时 CPU | 0% | 采样线程阻塞等待 |

**实测备注**：`Get-Counter` 单次 1008–1209 ms 是 PowerShell 的开销（每次新建查询 + 1 秒采样间隔），**不代表 PDH 的真实成本**。Rust 版用常驻查询，预期 5–15 ms，但这条必须实测，不能拿 PS 的数字下结论。

---

## 11. 错误与空状态

**总原则：宁可显示"不知道"，也不显示一个错的数。**

| 场景 | 错误处理 | 界面 |
|---|---|---|
| 计数器组不存在 | `ProbeResult::NoCounters` | 切"内存"列，顶部一句话说明 |
| 单个 counter 读取失败 | 跳过该行，不影响其他 | 该行显存列 `—` |
| 进程已退出 | 从快照移除 | 不显示 |
| 无权限 | 记为 `Unknown` | 名字可见，详情 `—` |
| 整卡读不到 | `Unattributed::Unavailable` | 未归属行 `—` |
| Σ > 整卡 | `Overcounted` | "口径异常" + 技术详情 |

**禁止**：任何"读失败就当 0"的写法。0 是一个合法测量值，混用会直接误导用户决策。

---

## 12. 未来扩展点（不现在做）

| 能力 | 挂在哪 | 代价 |
|---|---|---|
| spill 检测（显存溢出到内存） | `data/sampler` 增加 adapter shared 采样 | 上游有现成算法，但要状态机 |
| 历史曲线 | `data` 加环形缓冲 | +内存，但可与"不占进程"共存（只存 5 分钟） |
| 快照导出 | `explain/render` 复用 | 低 |
| 多语言 | 文案集中在一处即可 | 低，但当前只做中文 |
