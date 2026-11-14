//! 风险分级：规则给底线，运行状态只做"升级"，不做"降级"。
//!
//! - 规则说禁止，永远禁止；
//! - 规则说可关，但有窗口或正在用 GPU，就升为谨慎；
//! - 规则说谨慎，保持谨慎；
//! - 没规则，就是不知道——不猜。

use super::rules::RuleRisk;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    Forbidden,
    Caution,
    Safe,
    Unknown,
}

impl Risk {
    /// 图标：与颜色、文字一起构成三重编码，黑白截图也能读。
    pub fn glyph(self) -> &'static str {
        match self {
            Risk::Forbidden => "⛔",
            Risk::Caution => "⚠",
            Risk::Safe => "✓",
            Risk::Unknown => "?",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Risk::Forbidden => "禁止",
            Risk::Caution => "谨慎",
            Risk::Safe => "可关",
            Risk::Unknown => "未知",
        }
    }
}

/// GPU 超过这个值，就不再说"可关"。
const BUSY_GPU_PCT: f32 = 5.0;

pub fn classify(rule: Option<RuleRisk>, has_window: bool, gpu_pct: Option<f32>) -> Risk {
    match rule {
        Some(RuleRisk::Forbidden) => Risk::Forbidden,
        Some(RuleRisk::Caution) => Risk::Caution,
        Some(RuleRisk::Safe) => {
            let busy = has_window || gpu_pct.is_some_and(|pct| pct > BUSY_GPU_PCT);
            if busy {
                Risk::Caution
            } else {
                Risk::Safe
            }
        }
        None => Risk::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_always_wins() {
        assert_eq!(
            classify(Some(RuleRisk::Forbidden), true, Some(99.0)),
            Risk::Forbidden
        );
    }

    #[test]
    fn safe_escalates_when_busy() {
        assert_eq!(classify(Some(RuleRisk::Safe), false, Some(0.0)), Risk::Safe);
        assert_eq!(
            classify(Some(RuleRisk::Safe), true, Some(0.0)),
            Risk::Caution
        );
        assert_eq!(
            classify(Some(RuleRisk::Safe), false, Some(30.0)),
            Risk::Caution
        );
    }

    #[test]
    fn caution_stays_and_missing_is_unknown() {
        assert_eq!(
            classify(Some(RuleRisk::Caution), false, Some(0.0)),
            Risk::Caution
        );
        assert_eq!(classify(None, false, Some(0.0)), Risk::Unknown);
    }
}
