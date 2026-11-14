//! 文案拼装：三列表里"程序 / 占用 / 描述"的后两段。
//!
//! 铁律：每个判断带"因为"；不知道就说不知道；不用恐吓词。

use crate::data::model::{AppGroup, FileMeta, GroupMember, ProgramTotal};
use crate::format;

use super::classify::{classify, Risk};
use super::rules::{rules, Rule};

pub struct RowText {
    pub title: String,
    pub risk: Risk,
    pub desc: String,
}

/// 分组行：标题带图标，描述带风险词和依据。`show_gpu=false` 用于内存降级模式。
pub fn render_group(group: &AppGroup, show_gpu: bool, meta: Option<&FileMeta>) -> RowText {
    let rule = rules().lookup(&group.root_name);
    let risk = classify(rule.map(|r| r.risk), group.has_window, group.gpu_pct);
    RowText {
        title: format!("{} {}", risk.glyph(), group.title),
        risk,
        desc: describe(
            rule,
            meta,
            risk,
            group.has_window,
            group.gpu_pct,
            group.nonlocal_bytes,
            1,
            show_gpu,
        ),
    }
}

/// 展开后的单进程行。
pub fn render_member(member: &GroupMember, show_gpu: bool, meta: Option<&FileMeta>) -> RowText {
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
        desc: describe(
            rule,
            meta,
            risk,
            member.has_window,
            member.gpu_pct,
            0,
            1,
            show_gpu,
        ),
    }
}

/// 第二页：按程序名汇总的一行。
pub fn render_program(program: &ProgramTotal, show_gpu: bool, meta: Option<&FileMeta>) -> RowText {
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
            meta,
            risk,
            program.has_window,
            program.gpu_pct,
            program.nonlocal_bytes,
            program.members.len(),
            show_gpu,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn describe(
    rule: Option<&Rule>,
    meta: Option<&FileMeta>,
    risk: Risk,
    has_window: bool,
    gpu_pct: Option<f32>,
    nonlocal_bytes: u64,
    process_count: usize,
    show_gpu: bool,
) -> String {
    let body = match rule {
        Some(rule) => rule.desc.clone(),
        None => unknown_body(meta),
    };

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
    if show_gpu {
        match gpu_pct {
            Some(pct) if pct >= 0.5 => evidence.push(format!("GPU {}", format::percent(pct))),
            // 没有引擎实例，或占用低于 0.5%，都按 0 说。
            _ => evidence.push("GPU 0%".to_string()),
        }
    }
    if nonlocal_bytes >= 256 * 1024 * 1024 {
        evidence.push(format!("{} 被挤到内存条", format::bytes(nonlocal_bytes)));
    }

    format!("{}：{}（{}）", risk.label(), body, evidence.join("、"))
}

/// 没有规则时，用 exe 自带的版本信息说一句实话；读不到才承认不知道。
fn unknown_body(meta: Option<&FileMeta>) -> String {
    let Some(meta) = meta else {
        return "没有这条程序的资料".to_string();
    };

    let mut parts: Vec<String> = Vec::with_capacity(2);
    if let Some(desc) = meta.description.as_deref().or(meta.product.as_deref()) {
        parts.push(format!("程序自述「{desc}」"));
    }
    if let Some(company) = meta.company.as_deref() {
        parts.push(format!("厂商 {company}"));
    }
    if !parts.is_empty() {
        return format!("{}；没有安全资料", parts.join("，"));
    }
    if let Some(version) = meta.version.as_deref() {
        return format!("程序版本 {version}，没有说明和安全资料");
    }
    "没有这条程序的资料".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_description_has_reason() {
        let rule = rules().lookup("dwm.exe");
        let text = describe(rule, None, Risk::Forbidden, false, Some(1.0), 0, 1, true);
        assert_eq!(
            text,
            "禁止：桌面合成，系统必需，不能关（无可见窗口、GPU 1.0%）"
        );
    }

    #[test]
    fn unknown_says_unknown_instead_of_guessing() {
        let text = describe(None, None, Risk::Unknown, false, None, 0, 1, true);
        assert_eq!(text, "未知：没有这条程序的资料（无可见窗口、GPU 0%）");
    }

    #[test]
    fn unknown_uses_version_info() {
        let meta = FileMeta {
            description: Some("DeepSeek Harness".to_string()),
            company: Some("Hangzhou DeepSeek Artificial Intelligence Co., Ltd.".to_string()),
            ..Default::default()
        };
        let text = describe(None, Some(&meta), Risk::Unknown, true, None, 0, 1, true);
        assert!(text.contains("程序自述「DeepSeek Harness」"));
        assert!(text.contains("厂商 Hangzhou DeepSeek"));
        assert!(text.contains("没有安全资料"));
    }

    #[test]
    fn mentions_spill_and_process_count() {
        let text = describe(
            None,
            None,
            Risk::Unknown,
            true,
            Some(0.0),
            512 * 1024 * 1024,
            3,
            true,
        );
        assert!(text.contains("3 个进程"));
        assert!(text.contains("有可见窗口"));
        assert!(text.contains("512 MB 被挤到内存条"));
    }

    #[test]
    fn memory_mode_omits_gpu_evidence() {
        let text = describe(None, None, Risk::Unknown, false, None, 0, 1, false);
        assert!(!text.contains("GPU"));
        assert!(text.contains("无可见窗口"));
    }
}
