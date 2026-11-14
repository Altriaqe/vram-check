//! 数字格式化：界面上只说人话。
//!
//! 纯函数、无 Win32，`ui` 与 `explain` 都可以用。

const KB: u64 = 1024;
const MB: u64 = 1024 * KB;
const GB: u64 = 1024 * MB;

/// 显存字节数 → "8.00 GB" / "700 MB" / "512 KB"。
pub fn bytes(value: u64) -> String {
    if value >= GB {
        format!("{:.2} GB", value as f64 / GB as f64)
    } else if value >= MB {
        format!("{:.0} MB", value as f64 / MB as f64)
    } else {
        format!("{:.0} KB", value as f64 / KB as f64)
    }
}

/// 大数字补一个人能感知的类比；没有合适的就不硬加。
pub fn bytes_friendly(value: u64) -> String {
    let base = bytes(value);
    if (GB..2 * GB).contains(&value) {
        format!("{base}（约一部 1080p 电影）")
    } else {
        base
    }
}

/// GPU 占用率 → "92%" / "3.5%"。
pub fn percent(value: f32) -> String {
    if value >= 10.0 {
        format!("{:.0}%", value)
    } else {
        format!("{:.1}%", value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_gibibytes() {
        assert_eq!(bytes(8188 * MB), "8.00 GB");
        assert_eq!(bytes(700 * MB), "700 MB");
        assert_eq!(bytes(43 * MB), "43 MB");
        assert_eq!(bytes(512 * KB), "512 KB");
    }

    #[test]
    fn adds_analogy_only_when_it_helps() {
        assert_eq!(bytes_friendly(1536 * MB), "1.50 GB（约一部 1080p 电影）");
        assert_eq!(bytes_friendly(8 * GB), "8.00 GB");
    }

    #[test]
    fn formats_percent() {
        assert_eq!(percent(92.4), "92%");
        assert_eq!(percent(3.45), "3.5%");
    }
}
