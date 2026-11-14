//! 进程名 → 一句话。M1 的规则表，M2 换成 TSV 数据文件。
//!
//! 文案铁律：带"因为"、不裸缩写、不恐吓、不知道就说不知道。

/// (进程名, 描述)。匹配进程名，忽略大小写；标题里的 " × 3" 会先剥掉。
const RULES: &[(&str, &str)] = &[
    ("dwm.exe", "桌面合成，系统必需，不能关"),
    ("explorer.exe", "桌面和任务栏，关掉会黑屏"),
    ("csrss.exe", "系统核心进程，不能关"),
    ("wininit.exe", "系统核心进程，不能关"),
    ("winlogon.exe", "登录会话管理，不能关"),
    ("services.exe", "Windows 服务管理，不能关"),
    ("lsass.exe", "登录凭据服务，不能关"),
    ("smss.exe", "会话管理，不能关"),
    ("svchost.exe", "Windows 服务宿主，不能关"),
    ("System", "内核与驱动持有的显存，不能关"),
    ("[kernel]", "内核与驱动持有的显存，不能关"),
    ("python.exe", "Python 程序，可能是训练或推理脚本"),
    ("pythonw.exe", "后台 Python 程序，可能是训练或推理脚本"),
    ("chrome.exe", "浏览器，多半是后台标签页"),
    ("msedge.exe", "浏览器，多半是后台标签页"),
    ("firefox.exe", "浏览器，多半是后台标签页"),
    ("msedgewebview2.exe", "应用内嵌的网页界面"),
    ("Code.exe", "VS Code 编辑器，可能挂着终端任务"),
    ("idea64.exe", "JetBrains IDE，可能正在跑构建"),
    ("pycharm64.exe", "PyCharm，可能正在跑训练"),
    ("node.exe", "Node.js 进程，可能是开发服务器"),
    ("WindowsTerminal.exe", "终端窗口，里面可能在跑任务"),
    ("powershell.exe", "PowerShell 窗口，里面可能在跑任务"),
    ("pwsh.exe", "PowerShell 窗口，里面可能在跑任务"),
    ("cmd.exe", "命令行窗口，里面可能在跑任务"),
    ("MsMpEng.exe", "Windows Defender 扫描，系统必需"),
    ("NVIDIA Overlay.exe", "NVIDIA 游戏内覆盖，一般可关"),
];

/// 给标题（可能是 "python.exe × 3"）配一句描述；不认识返回 `—`。
pub fn describe(title: &str) -> &'static str {
    let name = title.split(" × ").next().unwrap_or(title).trim();
    RULES
        .iter()
        .find(|(rule_name, _)| name.eq_ignore_ascii_case(rule_name))
        .map(|(_, desc)| *desc)
        .unwrap_or("—")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_exact_and_grouped_titles() {
        assert_eq!(describe("dwm.exe"), "桌面合成，系统必需，不能关");
        assert_eq!(
            describe("python.exe × 3"),
            "Python 程序，可能是训练或推理脚本"
        );
        assert_eq!(describe("CHROME.EXE × 12"), "浏览器，多半是后台标签页");
    }

    #[test]
    fn unknown_says_unknown() {
        assert_eq!(describe("mystery.exe"), "—");
    }
}
