//! 从进程 exe 里抽 16px 小图标，塞进 ListView 的 ImageList。
//!
//! 只认 exe 自己带的图标；没有图标组的 exe 返回 `None`，界面继续显示原来的风险符号。

use windows::core::PCWSTR;
use windows::Win32::UI::Controls::{
    ImageList_Create, ImageList_ReplaceIcon, HIMAGELIST, ILC_COLOR32, ILC_MASK,
};
use windows::Win32::UI::Shell::ExtractIconExW;
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON};

use crate::vendor::wide;

pub fn create(size: i32) -> HIMAGELIST {
    unsafe { ImageList_Create(size, size, ILC_COLOR32 | ILC_MASK, 32, 32) }
}

/// 提取成功返回 ImageList 里的索引；没有图标就返回 `None`。
pub fn add(list: HIMAGELIST, pid: u32) -> Option<i32> {
    if list.0 == 0 {
        return None;
    }
    let path = crate::data::filemeta::process_path(pid)?;
    let path = wide(&path);
    let mut icon = HICON::default();
    let extracted = unsafe { ExtractIconExW(PCWSTR(path.as_ptr()), 0, None, Some(&mut icon), 1) };
    if extracted == 0 || icon.0.is_null() {
        return None;
    }

    let index = unsafe { ImageList_ReplaceIcon(list, -1, icon) };
    let _ = unsafe { DestroyIcon(icon) };
    (index >= 0).then_some(index)
}
