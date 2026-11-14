# M0 实测报告

| 项 | 内容 |
|---|---|
| 日期 | 2026-11-14 |
| 环境 | Windows 11 家庭版 中文版 · RTX 4070 Laptop 8GB · 驱动 596.49 |
| 方法 | PowerShell `Get-Counter` 直接读 WDDM 性能计数器 |
| 结论 | **数据源成立；口径必须用 `Local Usage`；体积/启动/杀进程回收仍未验证** |

---

## 0. 一句话结论

每进程显存读得到，而且 `Local Usage` 只有 6% 未归属，`Dedicated Usage` 直接爆表到 128.5%——**ADR-004 从"抄来的经验"变成了"本机实测事实"。**

---

## 1. 假设 vs 实测

| # | 假设 | 实测 | 判定 |
|---|---|---|---|
| 1 | 每进程显存可读，免管理员 | 32 个实例，26 个非零；`Elevated = False` | ✅ 成立 |
| 2 | `Dedicated` 会重复计，`Local` 才自洽 | Local 93.9% / **Dedicated 128.5%** | ✅ **强证实** |
| 3 | 中文系统要 `PdhAddEnglishCounter` 绕过本地化 | 中文 Win11 上英文名**直接可用** | ⚠️ 本机不必需，仍建议保留 |
| 4 | 进程多时采样会慢 | 314 进程里只有 **32 个**有显存实例 | ✅ 风险不存在 |
| 5 | `dwm.exe` 可能占掉几 GB | 空闲态 200 MB（占 8GB 的 2.5%） | ⚠️ 只在浏览器/视频场景才吃满 |
| 6 | WMI 能拿到显存容量 | `AdapterRAM` 报 **4.00 GB**，实际 **8188 MiB** | ❌ **必须换 DXGI** |

---

## 2. 关键数据（同一时刻）

```
整卡（nvidia-smi）         8188 MiB 总量 / 713 MiB 已用
整卡（PDH Adapter Memory） 718 MiB        ← 与 nvidia-smi 差 0.7%

进程合计  Local Usage      675 MiB   归因率  93.9%   ← 可用
进程合计  Dedicated Usage  923 MiB   归因率 128.5%   ← 不可能，重复计入
进程合计  Non Local Usage   64 MiB

未归属 = 718 − 675 = 43 MiB（6.0%）
```

**两条独立通路互相验证**：PDH 适配器总量 718 MiB vs `nvidia-smi` 713 MiB。这说明 PDH 的整卡读数可信，"未归属"算法有锚。

---

## 3. 五个发现

### 发现 1：`Dedicated` 爆表，`Local` 干净

| 口径 | 合计 | 占整卡 | 能不能给用户看 |
|---|---|---|---|
| `Local Usage` | 675 MiB | 93.9% | ✅ **主口径** |
| `Dedicated Usage` | 923 MiB | **128.5%** | ❌ 用户一加就穿帮 |

**影响**：界面永不展示 Dedicated，只在"技术详情"里出现。

**副作用**：hypomnesis 用的正是 Dedicated（为了对齐任务管理器）。这成了我们和它的真实分歧点——见 ADR-003 / ADR-004。

### 发现 2：中文 Windows 不需要 `PdhAddEnglishCounter`

英文计数器名 `GPU Process Memory` / `Local Usage` 在中文 Win11 上直接命中。

**影响**：T5 从"必需"降级为"保险"。仍建议用（成本为零），但不必为它设计降级链路。

### 发现 3：只有 32 个进程有显存，不是 314 个

- 采样成本极低，≤10ms 大概率轻松达标；
- **UI 只需展示这 32 行**，"三列 + 一屏 Top 5"的设计完全站得住；
- 不需要虚拟滚动、不需要分页（支撑 ADR-006）。

### 发现 4：`dwm.exe` 是空闲态第一名（200MB），但没那么吓人

Top 5：`dwm` 200MB / `Code` 104MB / `DeepSeek Harness` 102MB / `msedgewebview2` 44MB / `explorer` 41MB。

**影响**：gputaskman 提到的"dwm 15.3 GiB"是浏览器+视频的极端场景，**不能当通用叙事**。文案不能说"系统偷了你一半显存"——空闲态它只占 2.5%。

### 发现 5：这台机器有 4 个虚拟显示适配器

`ToDesk` / `MuMu`（模拟器）/ `OrayIddDriver`（向日葵）/ `GameViewer`。

**影响**：
- 多适配器不是可选项，是默认情况（T4 的 LUID 过滤必须做对）；
- 这些虚拟适配器显存容量为 0，不能参与"整卡"求和；
- 反向印证 ADR-002：这台机器就是拿来做远程串流的，**OpenGL 在 RDP 下不可靠**。

---

## 4. P0 其余数据源验证

| 数据源 | 用途 | 实测结果 | 判定 |
|---|---|---|---|
| `GPU Engine(*)\Utilization Percentage` | GPU% 列 | 437 个实例，仅 7 个 >0 | ✅ |
| `Get-AuthenticodeSignature` | 解释层 L2 | explorer=Microsoft Windows、msedge=Microsoft Corporation | ✅ |
| `Win32_Process.CommandLine` | 解释层 L3 | 非提权可读其他进程命令行 | ✅ |
| 全程权限 | G4 零管理员 | `Elevated = False` 下完成全部读数 | ✅ **实测成立** |

**两个实现坑**：

1. **`GPU Engine` 实例名格式不同**
   ```
   GPU Process Memory: pid_12408_luid_0x..._phys_0
   GPU Engine        : pid_12408_luid_0x..._phys_0_eng_0_engtype_3d
   ```
   多出 `_eng_N_engtype_XXX` 两段，**解析器不能复用**。

2. **GPU% 必须取"最忙引擎"，不能求和**
   437 个实例对应 32 个进程；同进程多引擎相加会超 100%。正确做法 `max(engine)`。

**性能设计结论**：命令行查询（`Win32_Process`）比计数器慢一个量级，**不要放进 1Hz 轮询**——只在用户选中某行时按需查。

---

## 5. 竞品参考：hypomnesis 教了三件事

**① 它替我们把数据层的坑踩完了**
`[kernel]` 标 PID 4、Toolhelp32 兜底进程名、`n/a` 永不写成 `0`、计数器缺失要降级——和我们的 T2/T3/T6/T7 完全一致，**说明方向对了**。

**② 它明确拒绝做"关闭进程"，理由正好是我们的存在理由**
README 有一节 *Why no `hmn kill`?*：

> "Process termination is not a measurement operation — it's a control operation."

而且它给的替代方案是 `hmn ps --json | jq | taskkill`——**用户已经在自己拼这个流程了，这就是需求存在的证据**。

**③ 它的口径是 `Dedicated`，和我们的 `Local` 冲突**
结论：不直接依赖它的 crate，自研/移植 PDH，把它当教科书。

---

## 6. 仍未验证的三件事

| 项 | 为什么没验 | 怎么验 |
|---|---|---|
| **体积 ≤2MB** | 本机没有 Rust 工具链 | 装 rustup + MSVC 后 `cargo build --release` |
| **冷启动 ≤100ms** | 同上 | `Measure-Command { .\vram-sentinel.exe }` |
| **杀进程后显存回收** | 需要构造占用场景 | 起一个占 2GB 的进程，杀掉，测 10 秒回收曲线 |

`Get-Counter` 单次 1008–1209 ms 是 **PowerShell 的开销**（每次新建 PDH 查询 + 1 秒采样间隔），不是 PDH 的真实成本。Rust 版用常驻查询，预期 5–15ms——**这条必须实测，不能拿 PS 的数字下结论**。

---

## 7. 复现方法

```powershell
# 每进程显存 Top N（主口径）
(Get-Counter '\GPU Process Memory(*)\Local Usage').CounterSamples |
    Where-Object { $_.InstanceName -match '^pid_' } |
    Sort-Object CookedValue -Descending | Select-Object -First 10 InstanceName, CookedValue

# 整卡已用
Get-Counter '\GPU Adapter Memory(*)\Dedicated Usage'

# 显卡真实容量（WMI 会截断，用这个）
nvidia-smi --query-gpu=name,memory.total,memory.used --format=csv
```
