# 第三方组件声明（THIRD-PARTY NOTICES）

> 截至 2026-11-14（M1 数据层跑通），**本仓库不包含任何第三方源代码**。
> 数据层的 PDH / DXGI / Toolhelp32 调用是按 Win32 公开 API 自行实现的。
> 若后续按 ADR-003 移植上游代码，必须在本文件中补充**具体文件、上游 commit、修改说明**，并在源文件头保留 SPDX 许可标识。

---

## hypomnesis（设计参考，当前未包含其代码）

- **来源**：https://github.com/PCfVW/hypomnesis
- **许可证**：MIT OR Apache-2.0（双许可，如引用按 MIT 履约）
- **参考内容**：`src/gpu/pdh.rs` 与 `src/gpu/dxgi.rs` 的实例名解析、LUID 过滤、Toolhelp32 兜底思路；`docs/DECISIONS.md` ADR-003 记录了"精准移植"计划。
- **当前状态**：M1 的实现**未复制其源码**，因此不构成衍生作品；下列 MIT 全文预留给后续真正移植时使用。

```
MIT License

Copyright (c) hypomnesis contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

> 也可选择 Apache-2.0 履约，全文见上游仓库 `LICENSE-APACHE`。

---

## 参考但未使用代码的项目

| 项目 | 许可证 | 说明 |
|---|---|---|
| [gputaskman](https://github.com/shanselman/gputaskman) | **未确认** | 仅作**产品形态参考**。**本项目未使用其任何代码**，在许可证确认前不得复制其源码。 |
| [Pane](https://github.com/TxsharDev/Pane) | 未确认 | 仅作形态参考，未使用代码。 |
| [hypomnesis](https://github.com/PCfVW/hypomnesis) | MIT OR Apache-2.0 | 仅作数据层实现参考，M1 未复制代码。 |

---

## Windows API

本项目通过 `windows` crate 调用 Microsoft Windows 操作系统 API，不包含微软的源代码。

- `windows` crate：MIT OR Apache-2.0，https://github.com/microsoft/windows-rs

---

_若后续新增第三方代码，必须在本文件中补充来源、许可证、修改说明，并在源文件头保留 SPDX 标识。_
