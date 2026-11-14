# 工程化规范

| 项       | 内容                                                |
| -------- | --------------------------------------------------- |
| 版本     | v1.0                                                |
| 适用范围 | 本仓库全部代码                                      |
| 优先级   | **本规范 > 个人偏好**；要违反请在 PR 里写理由 |

---

## 1. 工具链

| 项         | 值                                  |
| ---------- | ----------------------------------- |
| 语言       | Rust（stable）                      |
| Edition    | 2021                                |
| 目标三元组 | `x86_64-pc-windows-msvc`          |
| 最低系统   | Windows 10 1709（有 WDDM 计数器）   |
| 格式化     | `cargo fmt`（不做任何自定义配置） |
| 静态检查   | `cargo clippy -- -D warnings`     |

**不引入 nightly。** 不用任何不稳定特性。

---

## 2. 代码风格

- 遵循 `rustfmt` 默认。
- 缩进 4 空格，行宽默认 100。
- **禁止** `#[allow(...)]` 大面积压制；只允许在 `vendor/` 和 FFI 边界做**点状**压制，且必须写理由。
- 函数超过 **80 行**就要考虑拆分；`match` 超过 **8 个分支**要考虑抽函数。

### 命名

| 类型           | 规则                                                                   | 示例                                  |
| -------------- | ---------------------------------------------------------------------- | ------------------------------------- |
| 类型 / trait   | `UpperCamelCase`                                                     | `ProcSample`, `Risk`              |
| 函数 / 变量    | `snake_case`                                                         | `sample_once`, `local_bytes`      |
| 常量           | `SCREAMING_SNAKE_CASE`                                               | `MAX_GROUPS`                        |
| Win32 FFI 封装 | 保留原始名，包一层 snake_case                                          | `PdhOpenQueryW` → `open_query()` |
| 中文文案       | **不放代码里**，集中在 `explain/rules.tsv` 与 `ui/format.rs` | —                                    |

---

## 3. unsafe 政策（最重要的一条）

```rust
#![deny(unsafe_op_in_unsafe_fn)]
```

| 规则 | 说明                                                                                                 |
| ---- | ---------------------------------------------------------------------------------------------------- |
| 位置 | `unsafe` **只允许**出现在 `vendor/`、`data/` 的 FFI 层、`ui/listview.rs` 的 Win32 调用 |
| 禁止 | `explain/` 和 `action/` 的业务逻辑里出现 `unsafe`                                              |
| 注释 | 每个 `unsafe` 块**必须**有 `// SAFETY:` 注释，说明为什么这里的指针/句柄是有效的            |
| 句柄 | 所有 Win32 句柄用 RAII guard 包裹（`Drop` 里关闭），不允许裸 `CloseHandle` 散落                  |
| 指针 | 不允许把裸指针跨线程传（用 `mpsc` 传所有权）                                                       |

**SAFETY 注释模板**：

```rust
// SAFETY: `query` 由 PdhOpenQueryW 成功返回且尚未关闭（QueryGuard 保证）；
// `path` 是 NUL 结尾的 UTF-16 缓冲，生命周期覆盖本次调用。
let status = unsafe { PdhAddEnglishCounterW(query, path, 0, &raw mut h) };
```

---

## 4. 错误处理

**不引入 `anyhow` / `thiserror`**——它们会把体积顶上去，而我们只有十几种错误。

```rust
// data/mod.rs
#[derive(Debug)]
pub enum SampleError {
    NoCounterSet,        // 系统不支持
    CollectFailed(u32),  // PDH 返回码
    AdapterLost,
}
```

三条规则：

1. **错误类型要能区分"系统不支持"和"这次采样失败"**——前者要降级，后者要跳过。
2. **用户可见的错误必须是完整句子**，不许出现 `0x800007D2`。
3. **错误不吞**：`let _ = ...` 只允许用于"清理动作"（关闭句柄），不允许用于数据读取。

---

## 5. 用户可见文案规范

代码里的字符串也是产品的一部分。五条铁律（与 [PRD §6.4](PRD.md) 一致）：

1. **不出现裸缩写**：不写 `WDDM`/`PDH`/`LUID`。必须出现时后面跟括号人话。
2. **每个判断带"因为"**：`"可关闭"` ❌ → `"可关闭：无窗口、GPU 0%、装在 Temp 目录"` ✅
3. **数字要可感**：`1.6 GB` 后面补 `≈ 一部 1080p 电影`。
4. **不知道就说不知道**：显示 `—` + 原因，**绝不猜**。说错比不说伤害大 10 倍。
5. **不用恐吓词**：写 `会丢失进度`，不写 `危险`、`严重`。

**术语对照表**（对外只用左列）：

| 说                             | 不说                           |
| ------------------------------ | ------------------------------ |
| 显存                           | 专用 GPU 内存 / Dedicated VRAM |
| 共享内存（被挤到内存条的部分） | Non Local Usage                |
| 说不清谁占的                   | 未归属 / unattributed          |
| 系统保留                       | 驱动开销 / kernel-owned        |
| 关掉                           | 终止 / kill / TerminateProcess |

**提交前自检**：把新增的字符串读一遍，问自己"我妈能看懂吗"。

---

## 6. 日志与调试

- **不引日志库**。
- 调试输出用编译期开关：

```rust
macro_rules! dbg_log {
    ($($arg:tt)*) => {
        #[cfg(feature = "debug")]
        eprintln!($($arg)*);
    };
}
```

- 发布版（默认 feature）**不输出任何东西**。
- 不允许把进程路径、命令行写进任何持久化文件。

---

## 7. 测试

| 层                           | 策略                                                                                   |
| ---------------------------- | -------------------------------------------------------------------------------------- |
| `explain/`                 | **必须**有单元测试。纯函数，覆盖规则匹配、风险分级、文案拼装                     |
| `data/model.rs` 的聚合算法 | **必须**有单元测试。用构造的 `ProcSample` 验证分组、GPU% 取 max、未归属计算    |
| 实例名解析                   | **必须**有单元测试。覆盖 `phys_N`、`eng_N_engtype_XXX`、`_Total`、畸形输入 |
| FFI / Win32                  | **不 mock**。写 `#[ignore]` 的 live test，手动在有显卡的机器上跑               |
| UI                           | 不做自动化。靠 5 人盲测（PRD §9）                                                     |

**规则**：凡是"能只靠输入输出判断对错"的逻辑，都必须是纯函数，都必须有测试。

> **注意**：测试只保护"算错会静默骗人"的纯逻辑——**不写冒烟测试、不写灰度测试、不为每次改动重复补测试**。

---

## 8. 依赖政策

新增任何 crate，PR 里必须回答：

1. 它解决什么问题？
2. **它让二进制大多少？**（给出实测数字，不是估计）
3. 能不能用 100 行以内的代码替代？

体积是硬约束，**默认答案是"不能引"**。

---

## 9. 体积与性能守门

每次合并到 `main` 前必须跑：

```powershell
cargo build --release
$size = (Get-Item .\target\release\vram-check.exe).Length
"体积: {0:N0} KB" -f ($size / 1KB)
if ($size -gt 2MB) { throw "体积超标，拒绝合并" }
```

启动时间：

```powershell
(Measure-Command { Start-Process .\target\release\vram-check.exe -PassThru | Out-Null }).TotalMilliseconds
```

| 指标     | 红线             |
| -------- | ---------------- |
| exe 体积 | **2 MB**   |
| 冷启动   | **100 ms** |
| 常驻内存 | **30 MB**  |

超红线不允许合并，除非在 PR 里说明理由并更新本文件。

---

## 10. 提交规范

采用 Conventional Commits，**正文用中文**：

```
feat(data): 增加 Local Usage 采样，修正未归属算法

上游 hypomnesis 使用 Dedicated Usage，实测合计占整卡 128.5%，
无法向用户展示。改为 Local Usage 后为 93.9%。

Refs: ADR-004
```

| 类型         | 含义             |
| ------------ | ---------------- |
| `feat`     | 新功能           |
| `fix`      | 修复             |
| `docs`     | 文档             |
| `refactor` | 重构（不改行为） |
| `perf`     | 性能             |
| `test`     | 测试             |
| `chore`    | 杂务             |

**提交粒度**：一个提交只做一件事。特别是**格式化改动不要和逻辑改动混在一个提交里**。

---

## 11. 分支与发布

| 项       | 规则                                              |
| -------- | ------------------------------------------------- |
| 主分支   | `main`，始终可编译、可发布                      |
| 功能分支 | `feat/<简短描述>`                               |
| 版本号   | `0.MINOR.PATCH`，M1 阶段保持 `0.x`            |
| 发布产物 | 单个 exe +`README` + `THIRD-PARTY-NOTICES.md` |

M1–M3 阶段不做 CI，靠本地守门；M4 再加。

---

## 12. 许可证头

新增的**自己的**源文件不需要加版权头（仓库根有 LICENSE 即可）。

`vendor/` 下的文件**必须**保留：

```rust
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) <上游作者>
// Modified by VRAM Check: Dedicated Usage -> Local Usage
```

---

## 13. PR 检查清单

提交前逐条打勾：

- [ ] `cargo fmt` 无改动
- [ ] `cargo clippy -- -D warnings` 通过
- [ ] 新增 `unsafe` 块都有 `SAFETY:` 注释
- [ ] 新增用户可见字符串符合 §5 五条铁律
- [ ] 新逻辑有单元测试（如果是纯逻辑）
- [ ] 体积 ≤ 2 MB
- [ ] 没有引入新依赖，或已按 §8 说明体积代价
- [ ] 改动了 `vendor/` 的话，已在 `VENDOR.md` 记录
- [ ] 没有把"读失败"写成 `0`
