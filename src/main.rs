#![windows_subsystem = "windows"]

mod action;
mod app;
mod data;
mod explain;
mod format;
mod ui;
mod vendor;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow};

use vendor::wide;

fn main() {
    unsafe {
        // 必须在创建任何窗口前设置，否则字体和布局会按 96 DPI 缩放。
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        // 单实例：已经在跑就把旧窗口叫到前面，不重复开。
        let mutex_name = wide("Local\\VramCheckSingleInstance");
        let _mutex = CreateMutexW(None, true, PCWSTR(mutex_name.as_ptr()));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let class = ui::window::class_wide();
            if let Ok(existing) = FindWindowW(PCWSTR(class.as_ptr()), PCWSTR::null()) {
                let _ = SetForegroundWindow(existing);
            }
            return;
        }

        if let Err(err) = app::run() {
            app::show_error(&format!("启动失败：{err}"));
        }
    }
}
