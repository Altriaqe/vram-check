//! 读 exe 自带的版本信息，给"规则库没有"的程序一句人话。
//!
//! 只读文件元数据，不执行目标程序；拿不到就返回 `None`，绝不猜。

use std::ffi::c_void;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

use crate::data::model::FileMeta;
use crate::vendor::wide;

/// 读一次进程 exe 的版本信息。调用方负责按程序名缓存，别每帧都调。
pub fn query(pid: u32) -> Option<FileMeta> {
    let path = process_path(pid)?;
    let path = wide(&path);
    if path.is_empty() {
        return None;
    }

    unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(path.as_ptr()), None);
        if size == 0 {
            return None;
        }

        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(
            PCWSTR(path.as_ptr()),
            0,
            size,
            data.as_mut_ptr() as *mut c_void,
        )
        .is_err()
        {
            return None;
        }

        let lang = translation(&data);
        let (lang, codepage) = lang.unwrap_or((0x0409, 0x04b0));
        let get = |field: &str| string_value(&data, lang, codepage, field);

        let meta = FileMeta {
            description: get("FileDescription"),
            product: get("ProductName"),
            company: get("CompanyName"),
            version: get("FileVersion"),
        };
        (!meta.is_empty()).then_some(meta)
    }
}

pub fn process_path(pid: u32) -> Option<String> {
    unsafe {
        let handle: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0u16; 1024];
        let mut len = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(handle);
        result.ok()?;
        Some(String::from_utf16_lossy(&buffer[..len as usize]))
    }
}

fn translation(data: &[u8]) -> Option<(u16, u16)> {
    let key = wide(r"\VarFileInfo\Translation");
    let mut ptr: *mut c_void = std::ptr::null_mut();
    let mut len = 0u32;
    unsafe {
        if !VerQueryValueW(
            data.as_ptr() as *const c_void,
            PCWSTR(key.as_ptr()),
            &mut ptr,
            &mut len,
        )
        .as_bool()
            || ptr.is_null()
            || len < 4
        {
            return None;
        }
        let lang = *(ptr as *const u16);
        let codepage = *((ptr as *const u16).add(1));
        Some((lang, codepage))
    }
}

fn string_value(data: &[u8], lang: u16, codepage: u16, field: &str) -> Option<String> {
    let key = wide(&format!(
        r"\StringFileInfo\{lang:04x}{codepage:04x}\{field}"
    ));
    let mut ptr: *mut c_void = std::ptr::null_mut();
    let mut len = 0u32;
    unsafe {
        if !VerQueryValueW(
            data.as_ptr() as *const c_void,
            PCWSTR(key.as_ptr()),
            &mut ptr,
            &mut len,
        )
        .as_bool()
            || ptr.is_null()
            || len == 0
        {
            return None;
        }
        let slice = std::slice::from_raw_parts(ptr as *const u16, len as usize);
        let text = String::from_utf16_lossy(slice)
            .trim_matches('\0')
            .trim()
            .to_string();
        (!text.is_empty()).then_some(text)
    }
}
