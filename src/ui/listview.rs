//! ListView 的最小封装：建表、设列、整表替换行。
//!
//! 直接发 `LVM_*` 消息，不引第三方控件库——体积是硬约束。

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, WPARAM};
use windows::Win32::UI::Controls::{
    HIMAGELIST, LIST_VIEW_ITEM_STATE_FLAGS, LVCF_TEXT, LVCF_WIDTH, LVCOLUMNW, LVIF_IMAGE,
    LVIF_TEXT, LVITEMW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, SendMessageW, HMENU, WINDOW_EX_STYLE, WINDOW_STYLE,
};

const LVM_DELETEALLITEMS: u32 = 0x1009;
const LVM_GETTOPINDEX: u32 = 0x1027;
const LVM_ENSUREVISIBLE: u32 = 0x1013;
const LVM_SETITEMSTATE: u32 = 0x102B;
const LVM_GETITEMSTATE: u32 = 0x102C;
const LVM_SETEXTENDEDLISTVIEWSTYLE: u32 = 0x1036;
const LVM_SETIMAGELIST: u32 = 0x1003;
const LVM_INSERTCOLUMNW: u32 = 0x1061; // LVM_FIRST + 97
const LVM_SETCOLUMNW: u32 = 4192;
const LVM_SETCOLUMNWIDTH: u32 = 0x101E;
const LVM_INSERTITEMW: u32 = 0x104D;
const LVM_SETITEMTEXTW: u32 = 0x1074;
const WM_SETFONT: u32 = 0x0030;

const LVSIL_SMALL: usize = 1;

const LVS_EX_FULLROWSELECT: usize = 0x0000_0020;
const LVS_EX_DOUBLEBUFFER: usize = 0x0001_0000;

const LVIS_SELECTED: u32 = 0x0002;
const LVIS_FOCUSED: u32 = 0x0001;

// WS_CHILD | WS_VISIBLE | WS_TABSTOP | LVS_REPORT | LVS_SHOWSELALWAYS
// 去掉 LVS_SINGLESEL，右键菜单可以一次结束多项。
const LIST_STYLE: u32 = 0x5001_0009;

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
    // 注意：LVM_SETEXTENDEDLISTVIEWSTYLE 的 wParam 是掩码、lParam 才是样式值。
    // 反过来写不会报错，但扩展样式一个都不会生效（整行选中失效，右键点空白处打不开菜单）。
    let styles = LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER;
    unsafe {
        SendMessageW(
            list,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            WPARAM(0),
            LPARAM(styles as isize),
        );
    }
}

pub fn set_font(list: HWND, font: windows::Win32::Graphics::Gdi::HFONT) {
    unsafe {
        SendMessageW(list, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
    }
}

/// 给小图标列接上 ImageList。
pub fn set_image_list(list: HWND, images: HIMAGELIST) {
    unsafe {
        SendMessageW(
            list,
            LVM_SETIMAGELIST,
            WPARAM(LVSIL_SMALL),
            LPARAM(images.0),
        );
    }
}

/// 改列宽：描述列跟随窗口宽度，别让长厂商名白截断。
pub fn set_column_width(list: HWND, index: usize, width: i32) {
    unsafe {
        SendMessageW(
            list,
            LVM_SETCOLUMNWIDTH,
            WPARAM(index),
            LPARAM(width as isize),
        );
    }
}

/// 改列标题：显存模式是"占用"，降级模式是"内存"。
pub fn set_column_text(list: HWND, index: usize, text: &str) {
    let mut buffer = crate::vendor::wide(text);
    let mut column = LVCOLUMNW {
        mask: LVCF_TEXT,
        pszText: PWSTR(buffer.as_mut_ptr()),
        cchTextMax: buffer.len() as i32 - 1,
        ..Default::default()
    };
    unsafe {
        SendMessageW(
            list,
            LVM_SETCOLUMNW,
            WPARAM(index),
            LPARAM(&mut column as *mut _ as isize),
        );
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

pub fn is_selected(list: HWND, index: usize) -> bool {
    let state = unsafe {
        SendMessageW(
            list,
            LVM_GETITEMSTATE,
            WPARAM(index),
            LPARAM(LVIS_SELECTED as isize),
        )
        .0 as usize
    };
    state & LVIS_SELECTED as usize != 0
}

pub fn clear_selection(list: HWND) {
    let mut item = LVITEMW {
        stateMask: LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED),
        state: LIST_VIEW_ITEM_STATE_FLAGS(0),
        ..Default::default()
    };
    unsafe {
        SendMessageW(
            list,
            LVM_SETITEMSTATE,
            WPARAM(usize::MAX),
            LPARAM(&mut item as *mut _ as isize),
        );
    }
}

/// 选中一行并把焦点给它；`focus=false` 用于恢复多选里的其余行。
pub fn select_item(list: HWND, index: usize, focus: bool) {
    let state = LVIS_SELECTED | if focus { LVIS_FOCUSED } else { 0 };
    let mut item = LVITEMW {
        stateMask: LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED | LVIS_FOCUSED),
        state: LIST_VIEW_ITEM_STATE_FLAGS(state),
        ..Default::default()
    };
    unsafe {
        SendMessageW(
            list,
            LVM_SETITEMSTATE,
            WPARAM(index),
            LPARAM(&mut item as *mut _ as isize),
        );
    }
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
pub fn set_rows(list: HWND, rows: &[(i32, String, String, String)]) {
    unsafe {
        SendMessageW(list, LVM_DELETEALLITEMS, WPARAM(0), LPARAM(0));
    }

    for (row_index, (image, first, second, third)) in rows.iter().enumerate() {
        // 第 0 列只放图标；文字从第 1 列开始，否则名字会被 24px 的图标列裁掉。
        let mask = LVIF_IMAGE;
        let mut item = LVITEMW {
            mask,
            iItem: row_index as i32,
            iSubItem: 0,
            iImage: *image,
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

        for (sub_item, extra) in [(1i32, first), (2i32, second), (3i32, third)] {
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
