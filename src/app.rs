//! 生命周期、消息循环、UI 与采样线程的接线。

use std::ffi::c_void;
use std::sync::mpsc::Receiver;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateFontIndirectW, UpdateWindow, CLEARTYPE_QUALITY, DEFAULT_CHARSET, HFONT, LOGFONTW,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DispatchMessageW, GetClientRect, GetMessageW, KillTimer, MessageBoxW,
    MoveWindow, PostQuitMessage, SendMessageW, SetTimer, SetWindowTextW, ShowWindow,
    TranslateMessage, HMENU, MB_ICONERROR, MSG, SW_SHOW, WINDOW_EX_STYLE, WINDOW_STYLE,
};

use crate::data::model::{Snapshot, Unattributed};
use crate::data::sampler::{self, SamplerThread};
use crate::explain;
use crate::vendor::wide;

use crate::ui::{format, listview, window};

const WM_SETFONT: u32 = 0x0030;

pub struct App {
    hwnd: HWND,
    header: HWND,
    list: HWND,
    refresh: HWND,
    font: HFONT,
    title_font: HFONT,
    dpi: u32,
    rx: Option<Receiver<Snapshot>>,
    sampler: Option<SamplerThread>,
    active: bool,
    minimized: bool,
    timer_on: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            hwnd: HWND::default(),
            header: HWND::default(),
            list: HWND::default(),
            refresh: HWND::default(),
            font: HFONT::default(),
            title_font: HFONT::default(),
            dpi: 96,
            rx: None,
            sampler: None,
            active: true,
            minimized: false,
            timer_on: false,
        }
    }

    pub fn on_create(&mut self, hwnd: HWND) {
        self.hwnd = hwnd;
        let title = wide("显存哨兵");
        unsafe {
            let _ = SetWindowTextW(hwnd, PCWSTR(title.as_ptr()));
        }
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        self.dpi = if dpi == 0 { 96 } else { dpi };
        self.font = create_font(self.px(10), 400);
        self.title_font = create_font(self.px(11), 700);

        let header_class = wide("STATIC");
        let button_class = wide("BUTTON");
        let initial = wide("正在读取显存…");
        let refresh_text = wide("刷新");
        unsafe {
            self.header = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(header_class.as_ptr()),
                PCWSTR(initial.as_ptr()),
                WINDOW_STYLE(0x5000_0000), // WS_CHILD | WS_VISIBLE
                0,
                0,
                100,
                24,
                hwnd,
                HMENU::default(),
                HINSTANCE::default(),
                None,
            )
            .unwrap_or_default();

            self.refresh = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(button_class.as_ptr()),
                PCWSTR(refresh_text.as_ptr()),
                WINDOW_STYLE(0x5001_0000), // WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON
                0,
                0,
                84,
                30,
                hwnd,
                HMENU(1 as *mut c_void),
                HINSTANCE::default(),
                None,
            )
            .unwrap_or_default();
        }

        if !self.header.0.is_null() {
            set_font(self.header, self.title_font);
        }
        if !self.refresh.0.is_null() {
            set_font(self.refresh, self.font);
        }

        match listview::create(hwnd) {
            Ok(list) => {
                self.list = list;
                listview::set_font(list, self.font);
                listview::apply_style(list);
                let widths = [self.px(280), self.px(96), self.px(430)];
                listview::set_columns(
                    list,
                    &[
                        ("程序", widths[0]),
                        ("占用", widths[1]),
                        ("描述", widths[2]),
                    ],
                );
            }
            Err(err) => set_text(self.header, &format!("列表创建失败：{err}")),
        }

        match sampler::spawn(self.hwnd) {
            Ok((thread, rx)) => {
                thread.request_sample();
                self.sampler = Some(thread);
                self.rx = Some(rx);
            }
            Err(err) => set_text(self.header, err.user_text()),
        }

        unsafe {
            SetTimer(hwnd, 1, 1000, None);
        }
        self.timer_on = true;
        self.size_children();
    }

    pub fn on_size(&mut self, minimized: bool) {
        self.minimized = minimized;
        self.sync_timer();
        if !minimized {
            self.size_children();
        }
    }

    pub fn on_activate(&mut self, active: bool) {
        self.active = active;
        self.sync_timer();
        if active {
            self.request_sample();
        }
    }

    pub fn on_timer(&mut self) {
        self.request_sample();
    }

    pub fn on_command(&mut self, wparam: usize) {
        let id = (wparam & 0xFFFF) as u16;
        if id == 1 {
            self.request_sample();
        }
    }

    pub fn on_snapshot(&mut self) {
        let mut latest = None;
        if let Some(rx) = &self.rx {
            while let Ok(snapshot) = rx.try_recv() {
                latest = Some(snapshot);
            }
        }
        if let Some(snapshot) = latest {
            self.apply_snapshot(snapshot);
        }
    }

    pub fn on_sample_failed(&mut self, _code: usize) {
        set_text(
            self.header,
            "这次刷新读不到数据，下一轮会自动重试；列表里保留的是上一次的结果。",
        );
    }

    pub fn on_destroy(&mut self) {
        // 先发停止信号再等，避免采样线程在窗口销毁后还往这里投消息。
        self.sampler.take();
        unsafe {
            PostQuitMessage(0);
        }
    }

    fn apply_snapshot(&mut self, snapshot: Snapshot) {
        let used = snapshot
            .adapter_used
            .map(format::bytes)
            .unwrap_or_else(|| "—".to_string());
        let header = format!(
            "{} · 已用 {} / {}",
            snapshot.adapter_name,
            used,
            format::bytes(snapshot.capacity_bytes)
        );
        set_text(self.header, &header);

        let mut rows: Vec<(String, String, String)> = Vec::with_capacity(snapshot.groups.len() + 1);
        for group in &snapshot.groups {
            let mut desc = explain::describe(&group.title).to_string();
            if let Some(pct) = group.gpu_pct {
                if pct >= 1.0 {
                    if desc == "—" {
                        desc = format!("GPU {}", format::percent(pct));
                    } else {
                        desc.push_str(&format!(" · GPU {}", format::percent(pct)));
                    }
                }
            }
            rows.push((group.title.clone(), format::bytes(group.local_bytes), desc));
        }

        let (value, desc) = match snapshot.unattributed {
            Unattributed::Bytes(bytes) => (format::bytes(bytes), "驱动与共享资源，无法关闭"),
            Unattributed::Overcounted(_) => {
                ("—".to_string(), "口径异常：进程合计超过整卡，不显示负数")
            }
            Unattributed::Unavailable => ("—".to_string(), "整卡读数不可用"),
        };
        rows.push(("未归属".to_string(), value, desc.to_string()));

        if !self.list.0.is_null() {
            // 恒等式自检：未归属 + Σ组内本地 = 整卡已用；对不上说明聚合算法有 bug。
            debug_assert!(match snapshot.unattributed {
                Unattributed::Bytes(bytes) => snapshot
                    .adapter_used
                    .map_or(true, |used| bytes + snapshot.total_local == used),
                _ => true,
            });
            listview::set_rows(self.list, &rows);
        }
    }

    fn request_sample(&mut self) {
        if let Some(sampler) = &self.sampler {
            sampler.request_sample();
        }
    }

    fn sync_timer(&mut self) {
        let want = self.active && !self.minimized && self.sampler.is_some();
        if want && !self.timer_on {
            unsafe {
                SetTimer(self.hwnd, 1, 1000, None);
            }
            self.timer_on = true;
        } else if !want && self.timer_on {
            unsafe {
                let _ = KillTimer(self.hwnd, 1);
            }
            self.timer_on = false;
        }
    }

    fn size_children(&mut self) {
        if self.hwnd.0.is_null() || self.list.0.is_null() {
            return;
        }
        let mut rect = RECT::default();
        if unsafe { GetClientRect(self.hwnd, &mut rect) }.is_err() {
            return;
        }
        let width = rect.right;
        let height = rect.bottom;
        let margin = self.px(16);
        let header_height = self.px(26);
        let button_width = self.px(84);
        let button_height = self.px(30);
        let gap = self.px(10);

        unsafe {
            let _ = MoveWindow(
                self.header,
                margin,
                margin,
                (width - margin * 2 - button_width - gap).max(0),
                header_height,
                true,
            );
            let _ = MoveWindow(
                self.refresh,
                width - margin - button_width,
                margin - self.px(2),
                button_width,
                button_height,
                true,
            );
            let list_y = margin + header_height + gap;
            let _ = MoveWindow(
                self.list,
                margin,
                list_y,
                (width - margin * 2).max(0),
                (height - list_y - margin).max(0),
                true,
            );
        }
    }

    fn px(&self, logical: i32) -> i32 {
        (logical * self.dpi as i32 + 48) / 96
    }
}

pub fn run() -> Result<(), String> {
    let module = unsafe { GetModuleHandleW(None) }.map_err(|e| e.message().to_string())?;
    let hinstance = HINSTANCE(module.0);
    window::register(hinstance)?;

    let state = Box::new(App::new());
    let state_ptr = Box::into_raw(state);
    let class = window::class_wide();
    let title = wide("显存哨兵");
    let dpi = unsafe { GetDpiForSystem() };
    let width = 900 * dpi as i32 / 96;
    let height = 520 * dpi as i32 / 96;

    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()),
            WINDOW_STYLE(0x00CF_0000), // WS_OVERLAPPEDWINDOW
            0x8000_0000u32 as i32,     // CW_USEDEFAULT
            0x8000_0000u32 as i32,
            width,
            height,
            HWND::default(),
            HMENU::default(),
            hinstance,
            Some(state_ptr as *const c_void),
        )
    }
    .map_err(|e| e.message().to_string())?;

    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);
    }

    let mut message = MSG::default();
    unsafe {
        while GetMessageW(&mut message, HWND::default(), 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

fn create_font(height: i32, weight: i32) -> HFONT {
    let mut logfont = LOGFONTW {
        lfHeight: -height,
        lfWeight: weight,
        lfCharSet: DEFAULT_CHARSET,
        lfQuality: CLEARTYPE_QUALITY,
        ..Default::default()
    };
    let face: Vec<u16> = "Microsoft YaHei UI"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let len = face.len().min(logfont.lfFaceName.len());
    logfont.lfFaceName[..len].copy_from_slice(&face[..len]);
    unsafe { CreateFontIndirectW(&logfont) }
}

fn set_font(hwnd: HWND, font: HFONT) {
    unsafe {
        SendMessageW(hwnd, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
    }
}

fn set_text(hwnd: HWND, text: &str) {
    if hwnd.0.is_null() {
        return;
    }
    let text = wide(text);
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(text.as_ptr()));
    }
}

/// 启动失败时给用户一句话，而不是一串错误码。
pub fn show_error(message: &str) {
    let text = wide(message);
    let caption = wide("显存哨兵");
    unsafe {
        MessageBoxW(
            HWND::default(),
            PCWSTR(text.as_ptr()),
            PCWSTR(caption.as_ptr()),
            MB_ICONERROR,
        );
    }
}
