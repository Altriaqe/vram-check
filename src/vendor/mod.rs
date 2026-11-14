//! 隔离区：第三方实现与平台 FFI 的落点。
//!
//! 规则见 `docs/ARCHITECTURE.md` §7：最小接触面、保留许可、改动写清楚。

pub mod dxgi;
pub mod pdh;

/// UTF-16 + NUL。Win32 的 `*W` API 只认这个。
pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
