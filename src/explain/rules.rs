//! 规则库：启动时解析一次，热路径只读。
//!
//! 数据在 `rules.tsv`，一行一条：进程名、风险、一句人话。
//! 解析失败的行直接跳过——规则库缺一条，不能让整个程序起不来。

use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleRisk {
    Forbidden,
    Caution,
    Safe,
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub risk: RuleRisk,
    pub desc: String,
}

pub struct Rules {
    map: HashMap<String, Rule>,
}

impl Rules {
    pub fn from_tsv(text: &str) -> Self {
        let mut map = HashMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut fields = line.split('\t');
            let (Some(name), Some(risk), Some(desc)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            let name = name.trim().to_ascii_lowercase();
            let desc = desc.trim();
            if name.is_empty() || desc.is_empty() {
                continue;
            }
            let Some(risk) = parse_risk(risk.trim()) else {
                continue;
            };
            map.entry(name).or_insert(Rule {
                risk,
                desc: desc.to_string(),
            });
        }
        Self { map }
    }

    pub fn lookup(&self, name: &str) -> Option<&Rule> {
        self.map.get(&name.to_ascii_lowercase())
    }

    #[cfg(test)]
    pub fn count(&self) -> usize {
        self.map.len()
    }
}

fn parse_risk(token: &str) -> Option<RuleRisk> {
    match token {
        "forbidden" => Some(RuleRisk::Forbidden),
        "caution" => Some(RuleRisk::Caution),
        "safe" => Some(RuleRisk::Safe),
        _ => None,
    }
}

static RULES: OnceLock<Rules> = OnceLock::new();

/// 全局规则库（首次访问时解析，约 0.2 ms）。
pub fn rules() -> &'static Rules {
    RULES.get_or_init(|| Rules::from_tsv(include_str!("rules.tsv")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# 注释不算
dwm.exe\tforbidden\t桌面合成，系统必需
python.exe\tcaution\tPython 程序
nahan.exe\tsafe\t后台工具
bad.exe\tmaybe\t风险写错了
only-two\tcolumns
dup.exe\tsafe\t第一次生效
dup.exe\tforbidden\t第二次应被忽略
";

    #[test]
    fn parses_valid_lines_and_skips_bad_ones() {
        let rules = Rules::from_tsv(SAMPLE);
        assert_eq!(rules.count(), 4);
        let dwm = rules.lookup("DWM.EXE").unwrap();
        assert_eq!(dwm.risk, RuleRisk::Forbidden);
        assert_eq!(dwm.desc, "桌面合成，系统必需");
        assert_eq!(rules.lookup("nahan.exe").unwrap().risk, RuleRisk::Safe);
        assert!(rules.lookup("bad.exe").is_none());
        assert!(rules.lookup("only-two").is_none());
    }

    #[test]
    fn duplicate_keeps_first() {
        let rules = Rules::from_tsv(SAMPLE);
        assert_eq!(rules.lookup("dup.exe").unwrap().risk, RuleRisk::Safe);
    }

    #[test]
    fn real_rules_file_has_enough_coverage() {
        let rules = rules();
        assert!(rules.count() >= 200, "规则只有 {} 条", rules.count());
        assert_eq!(rules.lookup("dwm.exe").unwrap().risk, RuleRisk::Forbidden);
        assert_eq!(rules.lookup("python.exe").unwrap().risk, RuleRisk::Caution);
        assert!(rules.lookup("no-such-process.exe").is_none());
    }
}
