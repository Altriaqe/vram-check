//! 窗口类与窗口过程。所有消息在这里分派给 `App`。

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetSysColorBrush, SetBkMode, COLOR_WINDOW, HDC, TRANSPARENT};
use windows::Win32::UI::Controls::{NMHDR, NM_CUSTOMDRAW, NM_DBLCLK, NM_RCLICK, TCN_SELCHANGE};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, GetWindowLongPtrW, LoadCursorW, LoadIconW, RegisterClassW, SetWindowLongPtrW,
    CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, IDC_ARROW, WNDCLASSW,
};

use crate::app::{App, WM_APP_START};
use crate::data::sampler::{WM_SAMPLE_FAILED, WM_SNAPSHOT_READY};
use crate::vendor::wide;

pub const CLASS: &str = "VramCheckWnd";

// 消息常量用数值，避免把整个 WindowsAndMessaging 常量表拖进 use 列表。
const WM_CREATE: u32 = 0x0001;
const WM_DESTROY: u32 = 0x0002;
const WM_SIZE: u32 = 0x0005;
const WM_ACTIVATE: u32 = 0x0006;
const WM_COMMAND: u32 = 0x0111;
const WM_NOTIFY: u32 = 0x004E;
const WM_TIMER: u32 = 0x0113;
const WM_NCCREATE: u32 = 0x0081;
const WM_NCDESTROY: u32 = 0x0082;
const WM_CTLCOLORSTATIC: u32 = 0x0138;
const SIZE_MINIMIZED: u32 = 1;

pub fn class_wide() -> Vec<u16> {
    wide(CLASS)
}

pub fn register(hinstance: HINSTANCE) -> Result<(), String> {
    let class = class_wide();
    let cursor = unsafe { LoadCursorW(None, IDC_ARROW) }.map_err(|e| e.message().to_string())?;
    // 资源 ID 1 是 app.rc 里的图标；没有资源时退回系统默认。
    let icon = unsafe { LoadIconW(hinstance, PCWSTR(1 as *const u16)) }.unwrap_or_default();
    let window_class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wndproc),
        hInstance: hinstance,
        hIcon: icon,
        hCursor: cursor,
        hbrBackground: unsafe { GetSysColorBrush(COLOR_WINDOW) },
        lpszClassName: PCWSTR(class.as_ptr()),
        ..Default::default()
    };

    if unsafe { RegisterClassW(&window_class) } == 0 {
        return Err(windows::core::Error::from_win32().message().to_string());
    }
    Ok(())
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = &*(lparam.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        return LRESULT(1);
    }

    let user_data = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    if user_data == 0 {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    let app = &mut *(user_data as *mut App);

    match message {
        WM_CREATE => {
            app.on_create(hwnd);
            LRESULT(0)
        }
        WM_SIZE => {
            app.on_size((wparam.0 as u32 & 0xFFFF) == SIZE_MINIMIZED);
            LRESULT(0)
        }
        WM_TIMER => {
            app.on_timer();
            LRESULT(0)
        }
        WM_ACTIVATE => {
            let active = (wparam.0 & 0xFFFF) != 0;
            app.on_activate(active);
            LRESULT(0)
        }
        WM_COMMAND => {
            app.on_command(wparam.0);
            LRESULT(0)
        }
        WM_NOTIFY => {
            // SAFETY: WM_NOTIFY 的 lParam 一定是 NMHDR 开头的结构，本消息处理期间有效。
            let header = unsafe { &*(lparam.0 as *const NMHDR) };
            if header.hwndFrom == app.list_handle() {
                match header.code {
                    NM_CUSTOMDRAW => app.on_custom_draw(lparam),
                    NM_DBLCLK => {
                        app.on_double_click(lparam);
                        LRESULT(0)
                    }
                    NM_RCLICK => {
                        app.on_right_click(lparam);
                        LRESULT(0)
                    }
                    _ => DefWindowProcW(hwnd, message, wparam, lparam),
                }
            } else if header.hwndFrom == app.tab_handle() && header.code == TCN_SELCHANGE {
                app.on_page_changed();
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_CTLCOLORSTATIC => {
            let hdc = HDC(wparam.0 as *mut core::ffi::c_void);
            SetBkMode(hdc, TRANSPARENT);
            LRESULT(GetSysColorBrush(COLOR_WINDOW).0 as isize)
        }
        WM_SNAPSHOT_READY => {
            app.on_snapshot();
            LRESULT(0)
        }
        WM_SAMPLE_FAILED => {
            app.on_sample_failed(wparam.0);
            LRESULT(0)
        }
        WM_APP_START => {
            app.start_sampler();
            LRESULT(0)
        }
        WM_DESTROY => {
            app.on_destroy();
            LRESULT(0)
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Box::from_raw(user_data as *mut App));
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}
