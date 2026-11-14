//! 解释层：给每一行一句人话。
//!
//! M1 只带最小规则集；M2 再扩成 `rules.tsv`（200 条）+ 签名 + 启发式。
//! 不认识就返回 `—`——说错比不解释伤害大 10 倍。

mod rules;

pub use rules::describe;
