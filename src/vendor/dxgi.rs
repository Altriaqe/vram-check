//! DXGI 适配器枚举。
//!
//! 只做一件事：拿到适配器名字、LUID、**显存容量**。
//! 容量只能来自 `DXGI_ADAPTER_DESC1.DedicatedVideoMemory`，WMI 在 >4GB 的卡上会截断。
//!
//! 上游参考：hypomnesis 的 `src/gpu/dxgi.rs`（MIT OR Apache-2.0），FFI 形状重写。

use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ERROR_NOT_FOUND};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterInfo {
    pub luid: u64,
    pub name: String,
    pub dedicated_bytes: u64,
    pub is_software: bool,
}

/// 枚举全部适配器。虚拟显卡（容量 0）也会返回，由调用方决定怎么显示。
pub fn enumerate() -> Result<Vec<AdapterInfo>, String> {
    let mut out = Vec::new();
    unsafe {
        let factory: IDXGIFactory1 =
            CreateDXGIFactory1().map_err(|e| format!("DXGI 初始化失败：{}", e.message()))?;
        let mut index = 0u32;
        loop {
            match factory.EnumAdapters1(index) {
                Ok(adapter) => {
                    let desc = adapter
                        .GetDesc1()
                        .map_err(|e| format!("适配器信息读取失败：{}", e.message()))?;
                    let name = wide_array_to_string(&desc.Description);
                    let luid = ((desc.AdapterLuid.HighPart as u32 as u64) << 32)
                        | desc.AdapterLuid.LowPart as u64;
                    let dedicated_bytes = desc.DedicatedVideoMemory as u64;
                    out.push(AdapterInfo {
                        luid,
                        name,
                        dedicated_bytes,
                        is_software: dedicated_bytes == 0,
                    });
                    index += 1;
                }
                Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(e) => return Err(format!("适配器枚举失败：{}", e.message())),
            }
        }
    }
    Ok(out)
}

/// 主适配器：有显存、非软件渲染，容量最大者优先。
pub fn primary(adapters: &[AdapterInfo]) -> Option<&AdapterInfo> {
    adapters
        .iter()
        .filter(|a| !a.is_software && a.dedicated_bytes > 0)
        .max_by_key(|a| a.dedicated_bytes)
}

fn wide_array_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len]).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_at_nul() {
        let mut buf = [0u16; 8];
        for (i, c) in "RTX".encode_utf16().enumerate() {
            buf[i] = c;
        }
        assert_eq!(wide_array_to_string(&buf), "RTX");
    }

    #[test]
    fn picks_largest_non_software() {
        let adapters = vec![
            AdapterInfo {
                luid: 1,
                name: "虚拟显卡".into(),
                dedicated_bytes: 0,
                is_software: true,
            },
            AdapterInfo {
                luid: 2,
                name: "RTX 4070".into(),
                dedicated_bytes: 8_580_000_000,
                is_software: false,
            },
        ];
        assert_eq!(primary(&adapters).unwrap().luid, 2);
    }
}
