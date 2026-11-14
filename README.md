# 显存检查 VRAM Check

> 显存不够时的急救箱——谁占着、它是干什么的、关掉能省多少、会不会出事。

一个 Windows 上的单文件小工具：打开就知道谁在吃显存，每一行都用人话解释，可以安全地关掉。

![显存检查界面](docs/assets/m4-ui.png)

---

## 现在到哪一步了

| 阶段        | 状态                                                                                                        |
| ----------- | ----------------------------------------------------------------------------------------------------------- |
| M0 技术验证 | ✅**完成**（本机实测，见 [docs/M0-MEASUREMENTS.md](docs/M0-MEASUREMENTS.md)）                            |
| 文档        | ✅ PRD / 架构 / 规范 / 决策记录                                                                             |
| M1 数据层   | ✅**完成**（三列表跑通，见 [docs/M1-MEASUREMENTS.md](docs/M1-MEASUREMENTS.md)）                          |
| M2 表达层   | ✅**完成**：256 条规则、风险三级 + 图标/颜色/文字、可展开分组、未归属固定行、口径说明                 |
| M3 行动层   | ✅**完成**：两个页签（实时 / 按程序）、右键结束任务、优雅关闭 → 3 秒 → 强制确认、关闭后实测整卡变化 |
| M4 打磨     | ✅**完成**：无计数器降级为「内存」视图、comctl32 v6 视觉样式、体积/启动守门脚本                       |
| 发布        | ⬜ 待定：代码签名、应用图标、许可证选择                                                                     |

**数据层的可行性已经没有悬念**：每进程显存可读、非提权、口径已验证。

---

## 三条硬约束

| 约束     | 目标                                              |
| -------- | ------------------------------------------------- |
| 体积小   | 单文件 exe**≤ 2 MB**，零运行时、零内嵌字体 |
| 数据快   | 冷启动**≤ 100 ms**，采样 ≤ 10 ms          |
| 不占进程 | 单进程、默认不常驻、失焦停止轮询                  |

再加一条兼容要求：**老机器也要能跑**（Win10 1709+ 完整功能；更低版本降级为"内存"列）。

---

## 文档索引

| 文档 | 内容 | 什么时候看 |
| ----------------------------------------------- | -------------------------------------------------------- | ---------------------- |
| [SYNC.md](SYNC.md) | **交接班**：当前进度、测试纪律、下载纪律、环境台账 | **每次写代码前** |
| [docs/PRD.md](docs/PRD.md) | 产品需求：做什么、不做什么、验收标准 | 想知道"这产品长什么样" |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | 架构：分层、数据流、线程、算法、目录 | 要动代码之前 |
| [docs/CONVENTIONS.md](docs/CONVENTIONS.md) | 工程规范：unsafe、错误、文案、提交、测试 | 每次写代码 |
| [docs/DECISIONS.md](docs/DECISIONS.md) | 决策记录（ADR）：为什么选 Rust / 原生控件 / Local 口径 | 想推翻某个设计之前 |
| [docs/M0-MEASUREMENTS.md](docs/M0-MEASUREMENTS.md) | M0 实测数据：口径对比、五个发现 | 怀疑某个结论时 |
| [docs/M1-MEASUREMENTS.md](docs/M1-MEASUREMENTS.md) | M1 实测：体积/启动/采样/内存、五个发现 | 验收或调性能时 |

---

## 技术栈

Rust + `windows` crate + Win32 原生控件（GDI / ListView）。数据层参考开源项目 [hypomnesis](https://github.com/PCfVW/hypomnesis)（MIT OR Apache-2.0）的 PDH/DXGI 后端思路，**M1 为按 Win32 API 自行实现，未复制其源码**（见 [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)）。

**关键改动**：上游用 `Dedicated Usage`，我们用 `Local Usage`——实测前者合计占整卡 128.5%，后者 93.9%。详见 [ADR-004](docs/DECISIONS.md)。

---

## 快速开始

> 工具链已装好（Rust 1.100 + MSVC BuildTools）。`cargo` 不在 PATH 时用 `%USERPROFILE%\.cargo\bin\cargo.exe`。

```powershell
cargo build --release
cargo run --release
```

发版前守门（体积 + 冷启动 + 依赖）：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\gate.ps1
```

## 发布产物

| 文件                                   | 说明                                        |
| -------------------------------------- | ------------------------------------------- |
| `release\vram-check.exe`             | 298 KB，带图标与版本信息（1.0.0），双击即用 |
| `release\vram-check-1.0.0-win64.zip` | exe + LICENSE + README + 第三方声明         |

重新打包：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\release.ps1
```

## 杀软误报

exe 未签名，部分杀软会对 Rust 生成的二进制报启发式误报（本项目实测卡巴斯基对 **debug** 产物报过一次 `VHO:Trojan.Win32.Convagent.gen`）。这不是下载来的东西，也不联网、不写注册表、不常驻。

- 开发时把仓库目录加入杀软排除项，或只用 release 产物；
- 分发前做代码签名（PRD 风险 R1）；
- 看到"清除并重启"的提示不要直接点，先把文件哈希和来源核对清楚。

体积守门：`cargo build --release` 后产物必须 ≤ 2 MB。

---

## 许可证

本项目使用 **MIT License**，全文见 [LICENSE](LICENSE)。

上游 hypomnesis 为 MIT OR Apache-2.0，本项目未复制其源码，署名与说明见 [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)。
