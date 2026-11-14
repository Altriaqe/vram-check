//! 只读：当前哪些进程拥有"可见且有标题"的顶层窗口。
//!
//! 这批数据只用于风险判断（有窗口 = 谨慎），不参与显存口径。

use std::collections::HashSet;

use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextLengthW, GetWindowThreadProcessId, IsWindowVisible,
};

/// 枚举顶层窗口，返回拥有可见窗口的 pid 集合。
pub fn visible_window_pids() -> HashSet<u32> {
    let mut pids = HashSet::new();
    // SAFETY: 回调由 EnumWindows 同步调用，`lparam` 指向本函数栈上的 `pids`，
    // 生命周期覆盖整个调用过程；回调内只写这个集合。
    unsafe {
        let _ = EnumWindows(Some(enum_window), LPARAM(&mut pids as *mut _ as isize));
    }
    pids
}

unsafe extern "system" fn enum_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let pids = unsafe { &mut *(lparam.0 as *mut HashSet<u32>) };
    let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
    let has_title = unsafe { GetWindowTextLengthW(hwnd) } > 0;
    if visible && has_title {
        let mut pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
        }
        if pid != 0 {
            pids.insert(pid);
        }
    }
    // 返回 TRUE 继续枚举。
    BOOL(1)
}
