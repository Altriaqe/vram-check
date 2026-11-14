//! ListView 的最小封装：建表、设列、整表替换行。
//!
//! 直接发 `LVM_*` 消息，不引第三方控件库——体积是硬约束。

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, WPARAM};
use windows::Win32::UI::Controls::{LVCF_TEXT, LVCF_WIDTH, LVCOLUMNW, LVIF_TEXT, LVITEMW};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, SendMessageW, HMENU, WINDOW_EX_STYLE, WINDOW_STYLE,
};

const LVM_DELETEALLITEMS: u32 = 0x1009;
const LVM_GETTOPINDEX: u32 = 0x1027;
const LVM_ENSUREVISIBLE: u32 = 0x1013;
const LVM_SETEXTENDEDLISTVIEWSTYLE: u32 = 0x1036;
const LVM_INSERTCOLUMNW: u32 = 0x1061; // LVM_FIRST + 97
const LVM_INSERTITEMW: u32 = 0x104D;
const LVM_SETITEMTEXTW: u32 = 0x1074;
const WM_SETFONT: u32 = 0x0030;

const LVS_EX_FULLROWSELECT: usize = 0x0000_0020;
const LVS_EX_DOUBLEBUFFER: usize = 0x0001_0000;

// WS_CHILD | WS_VISIBLE | WS_TABSTOP | LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS
const LIST_STYLE: u32 = 0x5001_000D;

pub fn create(parent: HWND) -> Result<HWND, String> {
    let class = crate::vendor::wide("SysListView32");
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0x0000_0200), // WS_EX_CLIENTEDGE
            PCWSTR(class.as_ptr()),
            PCWSTR::null(),
            WINDOW_STYLE(LIST_STYLE),
            0,
            0,
            100,
            100,
            parent,
            HMENU::default(),
            HINSTANCE::default(),
            None,
        )
        .map_err(|e| e.message().to_string())
    }
}

pub fn apply_style(list: HWND) {
    unsafe {
        SendMessageW(
            list,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            WPARAM(LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER),
            LPARAM(0),
        );
    }
}

pub fn set_font(list: HWND, font: windows::Win32::Graphics::Gdi::HFONT) {
    unsafe {
        SendMessageW(list, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
    }
}

pub fn set_columns(list: HWND, columns: &[(&str, i32)]) {
    for (index, (label, width)) in columns.iter().enumerate() {
        let mut text = crate::vendor::wide(label);
        let mut column = LVCOLUMNW {
            mask: LVCF_TEXT | LVCF_WIDTH,
            cx: *width,
            pszText: PWSTR(text.as_mut_ptr()),
            cchTextMax: text.len() as i32 - 1,
            ..Default::default()
        };
        unsafe {
            SendMessageW(
                list,
                LVM_INSERTCOLUMNW,
                WPARAM(index),
                LPARAM(&mut column as *mut _ as isize),
            );
        }
    }
}

/// 当前滚到最上面的行号。
pub fn top_index(list: HWND) -> i32 {
    unsafe { SendMessageW(list, LVM_GETTOPINDEX, WPARAM(0), LPARAM(0)).0 as i32 }
}

/// 整表重建后把滚动位置放回原处，别让列表每秒跳回顶部。
pub fn ensure_visible(list: HWND, index: i32) {
    if index > 0 {
        unsafe {
            SendMessageW(list, LVM_ENSUREVISIBLE, WPARAM(index as usize), LPARAM(0));
        }
    }
}

/// 整表替换。行数很少（一屏 Top 10+），不做增量。
pub fn set_rows(list: HWND, rows: &[(String, String, String)]) {
    unsafe {
        SendMessageW(list, LVM_DELETEALLITEMS, WPARAM(0), LPARAM(0));
    }

    for (row_index, (first, second, third)) in rows.iter().enumerate() {
        let mut first_text = crate::vendor::wide(first);
        let mut item = LVITEMW {
            mask: LVIF_TEXT,
            iItem: row_index as i32,
            iSubItem: 0,
            pszText: PWSTR(first_text.as_mut_ptr()),
            cchTextMax: first_text.len() as i32 - 1,
            ..Default::default()
        };
        let inserted = unsafe {
            SendMessageW(
                list,
                LVM_INSERTITEMW,
                WPARAM(0),
                LPARAM(&mut item as *mut _ as isize),
            )
        };

        for (sub_item, extra) in [(1i32, second), (2i32, third)] {
            let mut extra_text = crate::vendor::wide(extra);
            let mut cell = LVITEMW {
                mask: LVIF_TEXT,
                iSubItem: sub_item,
                pszText: PWSTR(extra_text.as_mut_ptr()),
                cchTextMax: extra_text.len() as i32 - 1,
                ..Default::default()
            };
            unsafe {
                SendMessageW(
                    list,
                    LVM_SETITEMTEXTW,
                    WPARAM(inserted.0 as usize),
                    LPARAM(&mut cell as *mut _ as isize),
                );
            }
        }
    }
}
