//! 解释层：规则库 · 风险分级 · 文案拼装。
//!
//! 纯逻辑，不碰 Win32；输入是 `data` 的模型，输出是给人看的句子。

pub mod classify;
pub mod render;
pub mod rules;

pub use classify::Risk;
pub use render::{render_group, render_member};
