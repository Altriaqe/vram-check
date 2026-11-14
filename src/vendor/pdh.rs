//! PDH（性能计数器）封装。
//!
//! 只做三件事：常驻查询、一次采集、把「计数器数组」读成 `(实例名, 数值)`。
//! 实例名解析单独放本文件底部，纯函数、可单测。
//!
//! 上游参考：hypomnesis 的 `src/gpu/pdh.rs`（MIT OR Apache-2.0）。
//! FFI 调用形状按 `windows` crate 重写，未整段复制。

use windows::core::PCWSTR;
use windows::Win32::System::Performance::{
    PdhAddCounterW, PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData,
    PdhGetFormattedCounterArrayW, PdhOpenQueryW, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE,
    PDH_MORE_DATA,
};

/// 一个常驻查询 + 一组计数器。进程生命周期内只建一次。
///
/// `windows` crate 把 PDH 句柄建模为 `isize`，0 表示无效。
pub struct CounterSet {
    query: isize,
    counters: Vec<isize>,
}

impl CounterSet {
    /// 按顺序添加计数器。任何一条失败都整体失败——宁可降级，不可半残。
    pub fn open(paths: &[&str]) -> Result<Self, u32> {
        let mut query: isize = 0;
        let status = unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) };
        if status != 0 {
            return Err(status);
        }

        let mut counters = Vec::with_capacity(paths.len());
        for path in paths {
            let wide = super::wide(path);
            let mut counter: isize = 0;
            // 英文计数器名在中文系统上实测直接可用；失败再退回本地化 API 作保险。
            let mut status =
                unsafe { PdhAddEnglishCounterW(query, PCWSTR(wide.as_ptr()), 0, &mut counter) };
            if status != 0 {
                status = unsafe { PdhAddCounterW(query, PCWSTR(wide.as_ptr()), 0, &mut counter) };
            }
            if status != 0 {
                unsafe {
                    let _ = PdhCloseQuery(query);
                }
                return Err(status);
            }
            counters.push(counter);
        }

        Ok(Self { query, counters })
    }

    /// 采集一次。之后才能读值。
    pub fn collect(&self) -> Result<(), u32> {
        let status = unsafe { PdhCollectQueryData(self.query) };
        if status != 0 {
            return Err(status);
        }
        Ok(())
    }

    /// 读第 `index` 个计数器的全部实例。
    ///
    /// 读失败返回错误；单条实例状态无效的**跳过**，绝不当作 0。
    pub fn read_array(&self, index: usize) -> Result<Vec<(String, f64)>, u32> {
        let counter = self.counters[index];

        let mut size = 0u32;
        let mut count = 0u32;
        let mut status = unsafe {
            PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, None)
        };
        // 第一次调用按惯例返回 PDH_MORE_DATA 并给出所需字节数。
        if status != PDH_MORE_DATA && status != 0 {
            return Err(status);
        }
        if size == 0 || count == 0 {
            return Ok(Vec::new());
        }

        // 用 u64 缓冲保证 8 字节对齐（PDH 条目在 x64 上是 8 字节对齐）。
        let words = (size as usize).div_ceil(8) + 8;
        let mut buffer = vec![0u64; words];
        let mut got_size = size;
        let mut got_count = count;
        status = unsafe {
            PdhGetFormattedCounterArrayW(
                counter,
                PDH_FMT_DOUBLE,
                &mut got_size,
                &mut got_count,
                Some(buffer.as_mut_ptr().cast()),
            )
        };
        if status != 0 {
            return Err(status);
        }

        let mut out = Vec::with_capacity(got_count as usize);
        let items = buffer.as_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
        for i in 0..got_count as usize {
            // SAFETY: PDH 在 `buffer` 内按 PDH_FMT_COUNTERVALUE_ITEM_W 连续写入 got_count 条，
            // 缓冲长度已按 API 返回的字节数分配并留有余量；`i < got_count`。
            let item = unsafe { &*items.add(i) };
            // PDH_CSTATUS_VALID_DATA(0) / PDH_CSTATUS_NEW_DATA(1) 之外一律丢弃。
            if item.FmtValue.CStatus > 1 {
                continue;
            }
            let name = unsafe { pwstr_to_string(item.szName) };
            if name.is_empty() {
                continue;
            }
            // SAFETY: 请求的格式是 PDH_FMT_DOUBLE，联合体里就是 doubleValue。
            let value = unsafe { item.FmtValue.Anonymous.doubleValue };
            out.push((name, value));
        }
        Ok(out)
    }
}

impl Drop for CounterSet {
    fn drop(&mut self) {
        unsafe {
            let _ = PdhCloseQuery(self.query);
        }
    }
}

unsafe fn pwstr_to_string(ptr: windows::core::PWSTR) -> String {
    if ptr.0.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    // 实例名长度上限 4096，纯防御，正常不超过 128。
    while len < 4096 && unsafe { *ptr.0.add(len) } != 0 {
        len += 1;
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(ptr.0, len) })
}

// ---------------------------------------------------------------------------
// 实例名解析
// ---------------------------------------------------------------------------

/// `GPU Process Memory` / `GPU Engine` / `GPU Adapter Memory` 的实例名。
///
/// 形如：
/// - `pid_12408_luid_0x00000000_0x0000A1B2_phys_0`
/// - `pid_12408_luid_0x00000000_0x0000A1B2_phys_0_eng_0_engtype_3d`
/// - `luid_0x00000000_0x0000A1B2_phys_0`（Adapter Memory，无 pid）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceName {
    pub pid: Option<u32>,
    pub luid: u64,
    pub phys: u32,
    pub eng: Option<u32>,
    pub eng_type: Option<String>,
}

/// 解析失败（`_Total`、畸形、未知字段）返回 `None`——调用方跳过这一条。
pub fn parse_instance(name: &str) -> Option<InstanceName> {
    let mut tokens = name.split('_');
    let mut pid = None;
    let mut luid_hi: Option<u32> = None;
    let mut luid_lo: Option<u32> = None;
    let mut phys = None;
    let mut eng = None;
    let mut eng_type = None;

    while let Some(key) = tokens.next() {
        match key.to_ascii_lowercase().as_str() {
            "pid" => pid = Some(tokens.next()?.parse::<u32>().ok()?),
            // LUID 拆成两个 32 位十六进制段：`luid_0x<hi>_0x<lo>`。
            "luid" => {
                luid_hi = Some(parse_hex32(tokens.next()?)?);
                luid_lo = Some(parse_hex32(tokens.next()?)?);
            }
            "phys" => phys = Some(tokens.next()?.parse::<u32>().ok()?),
            "eng" => eng = Some(tokens.next()?.parse::<u32>().ok()?),
            "engtype" => eng_type = Some(tokens.next()?.to_string()),
            _ => return None,
        }
    }

    let luid = ((luid_hi? as u64) << 32) | luid_lo? as u64;
    Some(InstanceName {
        pid,
        luid,
        phys: phys.unwrap_or(0),
        eng,
        eng_type,
    })
}

fn parse_hex32(s: &str) -> Option<u32> {
    let s = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))?;
    u32::from_str_radix(s, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_process_memory_instance() {
        let inst = parse_instance("pid_24168_luid_0x00000000_0x0000A1B2_phys_0").unwrap();
        assert_eq!(inst.pid, Some(24168));
        assert_eq!(inst.luid, 0xA1B2);
        assert_eq!(inst.phys, 0);
        assert!(inst.eng.is_none());
        assert!(inst.eng_type.is_none());
    }

    #[test]
    fn parses_engine_instance_with_high_luid() {
        let inst =
            parse_instance("pid_7_luid_0x00000001_0x00000002_phys_1_eng_3_engtype_3d").unwrap();
        assert_eq!(inst.pid, Some(7));
        assert_eq!(inst.luid, (1u64 << 32) | 2);
        assert_eq!(inst.phys, 1);
        assert_eq!(inst.eng, Some(3));
        assert_eq!(inst.eng_type.as_deref(), Some("3d"));
    }

    #[test]
    fn parses_adapter_instance_without_pid() {
        let inst = parse_instance("luid_0x00000000_0x0000ABCD_phys_0").unwrap();
        assert_eq!(inst.pid, None);
        assert_eq!(inst.luid, 0xABCD);
    }

    #[test]
    fn rejects_total_malformed_and_unknown() {
        assert!(parse_instance("_Total").is_none());
        assert!(parse_instance("pid_abc").is_none());
        assert!(parse_instance("pid_1_luid_0x1").is_none());
        assert!(parse_instance("pid_1_luid_0x00000000_0x00000001_bogus_2").is_none());
        assert!(parse_instance("").is_none());
    }
}
