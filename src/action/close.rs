//! 唯一的"写"操作：优雅关闭窗口、必要时强制结束进程。
//!
//! 规则：
//! 1. 默认只发 `WM_CLOSE`，不直接杀；
//! 2. 强制结束只能由用户在二次确认后触发；
//! 3. 每次强制结束前核对进程创建时间，防止 PID 被复用后杀错人。

use std::collections::HashSet;

use windows::Win32::Foundation::{CloseHandle, BOOL, FILETIME, HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessTimes, OpenProcess, TerminateProcess, PROCESS_ACCESS_RIGHTS,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextLengthW, GetWindowThreadProcessId, IsWindowVisible, PostMessageW,
    WM_CLOSE,
};

use crate::data::model::Target;

const STILL_ACTIVE: u32 = 259;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseError {
    /// 进程已经退出。
    Exited,
    /// PID 还在，但已经是另一个进程（创建时间对不上）。
    Changed,
    /// 系统保护，普通权限动不了。
    AccessDenied,
    /// 其它失败。
    Failed,
}

impl CloseError {
    pub fn user_text(self) -> &'static str {
        match self {
            CloseError::Exited => "它已经退出了。",
            CloseError::Changed => "这个 PID 已经换成了别的程序，为了安全没有继续。",
            CloseError::AccessDenied => "它被系统保护，普通权限结束不了；这里不会自动提权。",
            CloseError::Failed => "结束失败，系统没有给出更多原因。",
        }
    }
}

struct CloseContext {
    pids: HashSet<u32>,
    posted: usize,
}

/// 给目标进程的所有可见窗口发 `WM_CLOSE`。返回成功投递的窗口数。
pub fn post_close(targets: &[Target]) -> usize {
    let mut context = CloseContext {
        pids: targets.iter().map(|target| target.pid).collect(),
        posted: 0,
    };
    // SAFETY: 回调由 EnumWindows 同步调用，lparam 指向本函数栈上的 context。
    unsafe {
        let _ = EnumWindows(Some(enum_close), LPARAM(&mut context as *mut _ as isize));
    }
    context.posted
}

unsafe extern "system" fn enum_close(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let context = unsafe { &mut *(lparam.0 as *mut CloseContext) };
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if context.pids.contains(&pid)
        && unsafe { IsWindowVisible(hwnd) }.as_bool()
        && unsafe { GetWindowTextLengthW(hwnd) } > 0
    {
        unsafe {
            let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        context.posted += 1;
    }
    BOOL(1)
}

/// 进程是否还活着。权限不足或 PID 已变化都按"不在了"处理——
/// 反正这种情况我们也不会去动它。
pub fn is_alive(target: &Target) -> bool {
    let Ok(handle) = open_verified(target, PROCESS_QUERY_LIMITED_INFORMATION) else {
        return false;
    };
    let mut code = 0u32;
    let alive = unsafe { GetExitCodeProcess(handle, &mut code) }.is_ok() && code == STILL_ACTIVE;
    unsafe {
        let _ = CloseHandle(handle);
    }
    alive
}

/// 强制结束。调用方必须先拿到用户的确认。
pub fn terminate(target: &Target) -> Result<(), CloseError> {
    let handle = open_verified(
        target,
        PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
    )?;
    let result = unsafe { TerminateProcess(handle, 1) };
    unsafe {
        let _ = CloseHandle(handle);
    }
    result.map_err(|_| CloseError::Failed)
}

fn open_verified(target: &Target, access: PROCESS_ACCESS_RIGHTS) -> Result<HANDLE, CloseError> {
    // SAFETY: OpenProcess 只拿句柄；成功返回的句柄由调用方关闭。
    let handle = unsafe { OpenProcess(access, false, target.pid) }.map_err(map_open_error)?;

    if let Some(expected) = target.start_time {
        match creation_time(handle) {
            Some(actual) if actual == expected => {}
            Some(_) => {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                return Err(CloseError::Changed);
            }
            None => {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                return Err(CloseError::Failed);
            }
        }
    }
    Ok(handle)
}

fn map_open_error(error: windows::core::Error) -> CloseError {
    const ERROR_ACCESS_DENIED: u32 = 0x8007_0005;
    const ERROR_INVALID_PARAMETER: u32 = 0x8007_0057;
    match error.code().0 as u32 {
        ERROR_ACCESS_DENIED => CloseError::AccessDenied,
        ERROR_INVALID_PARAMETER => CloseError::Exited,
        _ => CloseError::Failed,
    }
}

fn creation_time(handle: HANDLE) -> Option<u64> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: handle 由 OpenProcess 成功返回且仍有效；四个 FILETIME 都是本地可写缓冲。
    unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) }.ok()?;
    Some(((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64)
}
