//! 数字格式化：界面上只说人话。

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
    fn formats_percent() {
        assert_eq!(percent(92.4), "92%");
        assert_eq!(percent(3.45), "3.5%");
    }
}
