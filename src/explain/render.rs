//! 文案拼装：三列表里"程序 / 占用 / 描述"的后两段。
//!
//! 铁律：每个判断带"因为"；不知道就说不知道；不用恐吓词。

use crate::data::model::{AppGroup, GroupMember, ProgramTotal};
use crate::format;

use super::classify::{classify, Risk};
use super::rules::{rules, Rule};

pub struct RowText {
    pub title: String,
    pub risk: Risk,
    pub desc: String,
}

/// 分组行：标题带图标，描述带风险词和依据。
pub fn render_group(group: &AppGroup) -> RowText {
    let rule = rules().lookup(&group.root_name);
    let risk = classify(rule.map(|r| r.risk), group.has_window, group.gpu_pct);
    RowText {
        title: format!("{} {}", risk.glyph(), group.title),
        risk,
        desc: describe(
            rule,
            risk,
            group.has_window,
            group.gpu_pct,
            group.nonlocal_bytes,
            1,
        ),
    }
}

/// 展开后的单进程行。
pub fn render_member(member: &GroupMember) -> RowText {
    let rule = rules().lookup(&member.name);
    let risk = classify(rule.map(|r| r.risk), member.has_window, member.gpu_pct);
    RowText {
        title: format!(
            "    └ {} {} (PID {})",
            risk.glyph(),
            member.name,
            member.pid
        ),
        risk,
        desc: describe(rule, risk, member.has_window, member.gpu_pct, 0, 1),
    }
}

/// 第二页：按程序名汇总的一行。
pub fn render_program(program: &ProgramTotal) -> RowText {
    let rule = rules().lookup(&program.name);
    let risk = classify(rule.map(|r| r.risk), program.has_window, program.gpu_pct);
    let title = if program.members.len() > 1 {
        format!(
            "{} {} × {}",
            risk.glyph(),
            program.name,
            program.members.len()
        )
    } else {
        format!("{} {}", risk.glyph(), program.name)
    };
    RowText {
        title,
        risk,
        desc: describe(
            rule,
            risk,
            program.has_window,
            program.gpu_pct,
            program.nonlocal_bytes,
            program.members.len(),
        ),
    }
}

fn describe(
    rule: Option<&Rule>,
    risk: Risk,
    has_window: bool,
    gpu_pct: Option<f32>,
    nonlocal_bytes: u64,
    process_count: usize,
) -> String {
    let body = rule
        .map(|rule| rule.desc.as_str())
        .unwrap_or("没有这条程序的资料");

    let mut evidence: Vec<String> = Vec::with_capacity(4);
    if process_count > 1 {
        evidence.push(format!("{process_count} 个进程"));
    }
    evidence.push(
        if has_window {
            "有可见窗口"
        } else {
            "无可见窗口"
        }
        .to_string(),
    );
    match gpu_pct {
        Some(pct) if pct >= 0.5 => evidence.push(format!("GPU {}", format::percent(pct))),
        // 没有引擎实例，或占用低于 0.5%，都按 0 说。
        _ => evidence.push("GPU 0%".to_string()),
    }
    if nonlocal_bytes >= 256 * 1024 * 1024 {
        evidence.push(format!("{} 被挤到内存条", format::bytes(nonlocal_bytes)));
    }

    format!("{}：{}（{}）", risk.label(), body, evidence.join("、"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_description_has_reason() {
        let rule = rules().lookup("dwm.exe");
        let text = describe(rule, Risk::Forbidden, false, Some(1.0), 0, 1);
        assert_eq!(
            text,
            "禁止：桌面合成，系统必需，不能关（无可见窗口、GPU 1.0%）"
        );
    }

    #[test]
    fn unknown_says_unknown_instead_of_guessing() {
        let text = describe(None, Risk::Unknown, false, None, 0, 1);
        assert_eq!(text, "未知：没有这条程序的资料（无可见窗口、GPU 0%）");
    }

    #[test]
    fn mentions_spill_and_process_count() {
        let text = describe(None, Risk::Unknown, true, Some(0.0), 512 * 1024 * 1024, 3);
        assert!(text.contains("3 个进程"));
        assert!(text.contains("有可见窗口"));
        assert!(text.contains("512 MB 被挤到内存条"));
    }
}
