//! 生命周期、消息循环、UI 与采样线程的接线。

use std::collections::HashSet;
use std::ffi::c_void;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateFontIndirectW, ScreenToClient, SetTextColor, UpdateWindow, CLEARTYPE_QUALITY,
    DEFAULT_CHARSET, HFONT, LOGFONTW,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    CDDS_ITEMPREPAINT, CDDS_PREPAINT, CDRF_DODEFAULT, CDRF_NEWFONT, CDRF_NOTIFYITEMDRAW,
    LVHITTESTINFO, NMITEMACTIVATE, NMLVCUSTOMDRAW, TCIF_TEXT, TCITEMW,
};
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DestroyMenu, DispatchMessageW, GetClientRect,
    GetCursorPos, GetMessageW, KillTimer, MessageBoxW, MoveWindow, PostQuitMessage, SendMessageW,
    SetTimer, SetWindowTextW, ShowWindow, TrackPopupMenuEx, TranslateMessage, HMENU, IDYES,
    MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MB_YESNO, MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG,
    SW_SHOW, TPM_RETURNCMD, TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WINDOW_STYLE,
};

use crate::action;
use crate::data::model::{Mode, Snapshot, Target, Unattributed};
use crate::data::sampler::{self, SamplerThread};
use crate::explain::{self, Risk};
use crate::format;
use crate::ui::{listview, window};
use crate::vendor::wide;

const WM_SETFONT: u32 = 0x0030;
const TCM_INSERTITEMW: u32 = 4926;
const TCM_GETCURSEL: u32 = 4875;
const LVM_HITTEST: u32 = 4114;
const MENU_CLOSE: usize = 1001;
const MENU_REFRESH: usize = 1002;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Live,
    Programs,
}

#[derive(Clone, PartialEq, Eq)]
enum RowKey {
    Group(u32),
    Member(u32),
    Program(String),
}

struct DisplayRow {
    key: RowKey,
    title: String,
    bytes: u64,
    desc: String,
    risk: Risk,
    targets: Vec<Target>,
}

struct PendingClose {
    targets: Vec<Target>,
    before_used: Option<u64>,
    started: Instant,
    force_asked: bool,
}

pub struct App {
    hwnd: HWND,
    header: HWND,
    list: HWND,
    tab: HWND,
    refresh: HWND,
    help: HWND,
    footer: HWND,
    font: HFONT,
    title_font: HFONT,
    small_font: HFONT,
    dpi: u32,
    rx: Option<Receiver<Snapshot>>,
    sampler: Option<SamplerThread>,
    snapshot: Option<Snapshot>,
    rows: Vec<DisplayRow>,
    expanded: HashSet<u32>,
    page: Page,
    status: Option<(String, Instant)>,
    pending: Option<PendingClose>,
    column_is_memory: bool,
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
            tab: HWND::default(),
            refresh: HWND::default(),
            help: HWND::default(),
            footer: HWND::default(),
            font: HFONT::default(),
            title_font: HFONT::default(),
            small_font: HFONT::default(),
            dpi: 96,
            rx: None,
            sampler: None,
            snapshot: None,
            rows: Vec::new(),
            expanded: HashSet::new(),
            page: Page::Live,
            status: None,
            pending: None,
            column_is_memory: false,
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
        self.small_font = create_font(self.px(9), 400);

        let header_class = wide("STATIC");
        let button_class = wide("BUTTON");
        let tab_class = wide("SysTabControl32");
        let initial = wide("正在读取显存…");
        let refresh_text = wide("刷新");
        let help_text = wide("口径说明");
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

            self.help = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(button_class.as_ptr()),
                PCWSTR(help_text.as_ptr()),
                WINDOW_STYLE(0x5001_0000),
                0,
                0,
                104,
                30,
                hwnd,
                HMENU(2 as *mut c_void),
                HINSTANCE::default(),
                None,
            )
            .unwrap_or_default();

            self.footer = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(header_class.as_ptr()),
                PCWSTR(initial.as_ptr()),
                WINDOW_STYLE(0x5000_0000),
                0,
                0,
                100,
                22,
                hwnd,
                HMENU::default(),
                HINSTANCE::default(),
                None,
            )
            .unwrap_or_default();

            self.tab = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(tab_class.as_ptr()),
                PCWSTR::null(),
                WINDOW_STYLE(0x5001_0000), // WS_CHILD | WS_VISIBLE | WS_TABSTOP
                0,
                0,
                100,
                28,
                hwnd,
                HMENU::default(),
                HINSTANCE::default(),
                None,
            )
            .unwrap_or_default();
        }

        if !self.tab.0.is_null() {
            set_font(self.tab, self.font);
            insert_tab(self.tab, "实时", 0);
            insert_tab(self.tab, "按程序", 1);
        }

        if !self.header.0.is_null() {
            set_font(self.header, self.title_font);
        }
        if !self.footer.0.is_null() {
            set_font(self.footer, self.small_font);
        }
        if !self.refresh.0.is_null() {
            set_font(self.refresh, self.font);
        }
        if !self.help.0.is_null() {
            set_font(self.help, self.font);
        }

        match listview::create(hwnd) {
            Ok(list) => {
                self.list = list;
                listview::set_font(list, self.font);
                listview::apply_style(list);
                let widths = [self.px(250), self.px(88), self.px(470)];
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
        self.check_status_expiry();
        self.check_pending_close();
        self.request_sample();
    }

    pub fn on_command(&mut self, wparam: usize) {
        let id = (wparam & 0xFFFF) as u16;
        match id {
            1 => self.request_sample(),
            2 => self.show_help(),
            _ => {}
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
            self.check_pending_close();
        }
    }

    pub fn list_handle(&self) -> HWND {
        self.list
    }

    pub fn tab_handle(&self) -> HWND {
        self.tab
    }

    pub fn on_sample_failed(&mut self, _code: usize) {
        set_text(
            self.header,
            "这次刷新读不到数据，下一轮会自动重试；列表里保留的是上一次的结果。",
        );
    }

    pub fn on_custom_draw(&mut self, lparam: LPARAM) -> LRESULT {
        // SAFETY: lParam 是 ListView 发来的 NMLVCUSTOMDRAW，只在本消息处理期间有效。
        let custom = unsafe { &mut *(lparam.0 as *mut NMLVCUSTOMDRAW) };
        if custom.nmcd.dwDrawStage == CDDS_PREPAINT {
            return LRESULT(CDRF_NOTIFYITEMDRAW as isize);
        }
        if custom.nmcd.dwDrawStage == CDDS_ITEMPREPAINT {
            // 选中行让系统自己画（蓝底白字），不覆盖颜色；用 LVM_GETITEMSTATE 查，
            // 不依赖 NMCUSTOMDRAW.uItemState —— 本机 v5 控件对每一项都报 0x201。
            let selected = unsafe {
                SendMessageW(
                    self.list,
                    0x102C, // LVM_GETITEMSTATE
                    WPARAM(custom.nmcd.dwItemSpec),
                    LPARAM(0x0002), // LVIS_SELECTED
                )
                .0 & 0x0002
                    != 0
            };
            if !selected {
                if let Some(row) = self.rows.get(custom.nmcd.dwItemSpec) {
                    let color = risk_color(row.risk);
                    custom.clrText = COLORREF(color);
                    unsafe {
                        SetTextColor(custom.nmcd.hdc, COLORREF(color));
                    }
                }
            }
            return LRESULT(CDRF_NEWFONT as isize);
        }
        LRESULT(CDRF_DODEFAULT as isize)
    }

    pub fn on_double_click(&mut self, lparam: LPARAM) {
        // SAFETY: lParam 是 ListView 发来的 NMITEMACTIVATE，只在本消息处理期间有效。
        let item = unsafe { &*(lparam.0 as *const NMITEMACTIVATE) };
        if item.iItem < 0 {
            return;
        }
        let Some(row) = self.rows.get(item.iItem as usize) else {
            return;
        };
        let RowKey::Group(root) = row.key else {
            return;
        };
        let has_children = self.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot
                .groups
                .iter()
                .any(|group| group.root_pid == root && group.members.len() > 1)
        });
        if !has_children {
            return;
        }
        if !self.expanded.remove(&root) {
            self.expanded.insert(root);
        }
        self.rebuild_rows();
    }

    pub fn on_destroy(&mut self) {
        // 先发停止信号再等，避免采样线程在窗口销毁后还往这里投消息。
        self.sampler.take();
        unsafe {
            PostQuitMessage(0);
        }
    }

    fn apply_snapshot(&mut self, snapshot: Snapshot) {
        let memory_mode = snapshot.mode == Mode::Memory;
        let header = if memory_mode {
            format!(
                "内存模式 · 进程合计 {}（读不到显存计数器，已切换「内存」列）",
                format::bytes_friendly(snapshot.total_local)
            )
        } else {
            let used = snapshot
                .adapter_used
                .map(format::bytes)
                .unwrap_or_else(|| "—".to_string());
            format!(
                "{} · 已用 {} / {}",
                snapshot.adapter_name,
                used,
                format::bytes(snapshot.capacity_bytes)
            )
        };
        set_text(self.header, &header);

        if !self.list.0.is_null() && memory_mode != self.column_is_memory {
            listview::set_column_text(self.list, 1, if memory_mode { "内存" } else { "占用" });
            self.column_is_memory = memory_mode;
        }

        // 恒等式自检：未归属 + Σ本地 = 整卡已用；对不上说明聚合算法有 bug。
        debug_assert!(match snapshot.unattributed {
            Unattributed::Bytes(bytes) => snapshot
                .adapter_used
                .map_or(true, |used| bytes + snapshot.total_local == used),
            _ => true,
        });

        self.snapshot = Some(snapshot);
        // 已经消失的分组不必记住展开状态。
        if let Some(snapshot) = &self.snapshot {
            self.expanded
                .retain(|root| snapshot.groups.iter().any(|group| group.root_pid == *root));
        }
        self.rebuild_rows();
    }

    fn rebuild_rows(&mut self) {
        let selected = self.selected_keys();
        let rows = {
            let Some(snapshot) = &self.snapshot else {
                return;
            };
            let mut rows = Vec::new();
            let show_gpu = snapshot.mode == Mode::Gpu;
            match self.page {
                Page::Live => {
                    for group in &snapshot.groups {
                        let text = explain::render_group(group, show_gpu);
                        let expanded = self.expanded.contains(&group.root_pid);
                        let arrow = if group.members.len() > 1 {
                            if expanded {
                                "▼ "
                            } else {
                                "▶ "
                            }
                        } else {
                            ""
                        };
                        rows.push(DisplayRow {
                            key: RowKey::Group(group.root_pid),
                            title: format!("{arrow}{}", text.title),
                            bytes: group.local_bytes,
                            desc: text.desc,
                            risk: text.risk,
                            targets: group.members.iter().map(|member| member.target()).collect(),
                        });

                        if expanded {
                            for member in &group.members {
                                let member_text = explain::render_member(member, show_gpu);
                                rows.push(DisplayRow {
                                    key: RowKey::Member(member.pid),
                                    title: member_text.title,
                                    bytes: member.local_bytes,
                                    desc: member_text.desc,
                                    risk: member_text.risk,
                                    targets: vec![member.target()],
                                });
                            }
                        }
                    }
                }
                Page::Programs => {
                    for program in crate::data::model::build_programs(&snapshot.groups) {
                        let text = explain::render_program(&program, show_gpu);
                        rows.push(DisplayRow {
                            key: RowKey::Program(program.key.clone()),
                            title: text.title,
                            bytes: program.local_bytes,
                            desc: text.desc,
                            risk: text.risk,
                            targets: program
                                .members
                                .iter()
                                .map(|member| member.target())
                                .collect(),
                        });
                    }
                }
            }
            rows
        };

        let changed = self.rows.len() != rows.len()
            || self.rows.iter().zip(&rows).any(|(old, new)| {
                old.key != new.key
                    || old.title != new.title
                    || old.bytes != new.bytes
                    || old.desc != new.desc
                    || old.risk != new.risk
            });
        self.rows = rows;

        if changed && !self.list.0.is_null() {
            let top = listview::top_index(self.list);
            let tuples: Vec<(String, String, String)> = self
                .rows
                .iter()
                .map(|row| {
                    (
                        row.title.clone(),
                        format::bytes(row.bytes),
                        row.desc.clone(),
                    )
                })
                .collect();
            listview::set_rows(self.list, &tuples);
            listview::ensure_visible(self.list, top);
            self.restore_selection(&selected);
        }
        self.update_footer();
    }

    fn selected_keys(&self) -> Vec<RowKey> {
        if self.list.0.is_null() {
            return Vec::new();
        }
        self.rows
            .iter()
            .enumerate()
            .filter(|(index, _)| listview::is_selected(self.list, *index))
            .map(|(_, row)| row.key.clone())
            .collect()
    }

    /// 刷新后按 key 把选中状态放回去——列表每秒重建，不能把用户的选择弄丢。
    fn restore_selection(&self, keys: &[RowKey]) {
        if keys.is_empty() || self.list.0.is_null() {
            return;
        }
        let mut focused = false;
        for key in keys {
            if let Some(index) = self.rows.iter().position(|row| row.key == *key) {
                listview::select_item(self.list, index, !focused);
                focused = true;
            }
        }
    }

    pub fn on_page_changed(&mut self) {
        let current = unsafe { SendMessageW(self.tab, TCM_GETCURSEL, WPARAM(0), LPARAM(0)) };
        self.page = if current.0 == 1 {
            Page::Programs
        } else {
            Page::Live
        };
        self.rebuild_rows();
    }

    pub fn on_right_click(&mut self, _lparam: LPARAM) {
        // 不信任通知里带的命中结果：直接按光标位置自己 HitTest。
        let mut point = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut point);
            let _ = ScreenToClient(self.list, &mut point);
        }
        let mut info = LVHITTESTINFO {
            pt: point,
            ..Default::default()
        };
        unsafe {
            SendMessageW(
                self.list,
                LVM_HITTEST,
                WPARAM(0),
                LPARAM(&mut info as *mut _ as isize),
            );
        }
        if info.iItem < 0 || info.iItem as usize >= self.rows.len() {
            return;
        }
        let index = info.iItem as usize;
        if !listview::is_selected(self.list, index) {
            listview::clear_selection(self.list);
            listview::select_item(self.list, index, true);
        }

        let keys = self.selected_keys();
        let mut targets: Vec<Target> = Vec::new();
        let mut forbidden = false;
        for key in &keys {
            if let Some(row) = self.rows.iter().find(|row| row.key == *key) {
                forbidden |= row.risk == Risk::Forbidden;
                for target in &row.targets {
                    if !targets.iter().any(|existing| existing.pid == target.pid) {
                        targets.push(*target);
                    }
                }
            }
        }
        if targets.is_empty() {
            return;
        }

        let menu = match unsafe { CreatePopupMenu() } {
            Ok(menu) => menu,
            Err(_) => return,
        };
        let close_label = wide(if forbidden {
            "系统进程，不能结束"
        } else {
            "结束任务"
        });
        let close_flags = if forbidden {
            windows::Win32::UI::WindowsAndMessaging::MENU_ITEM_FLAGS(MF_STRING.0 | MF_GRAYED.0)
        } else {
            MF_STRING
        };
        unsafe {
            let _ = AppendMenuW(menu, close_flags, MENU_CLOSE, PCWSTR(close_label.as_ptr()));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        }
        let refresh_label = wide("刷新");
        unsafe {
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                MENU_REFRESH,
                PCWSTR(refresh_label.as_ptr()),
            );
        }

        let mut point = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut point);
        }
        let command = unsafe {
            TrackPopupMenuEx(
                menu,
                TPM_RETURNCMD.0 | TPM_RIGHTBUTTON.0,
                point.x,
                point.y,
                self.hwnd,
                None,
            )
        }
        .0 as u32;
        unsafe {
            let _ = DestroyMenu(menu);
        }

        match command as usize {
            MENU_CLOSE if !forbidden => self.begin_close(targets),
            MENU_REFRESH => self.request_sample(),
            _ => {}
        }
    }

    fn begin_close(&mut self, targets: Vec<Target>) {
        let verified: Vec<Target> = targets.into_iter().filter(action::is_alive).collect();
        if verified.is_empty() {
            self.set_status(
                "这些进程已经退出（或 PID 已变化），没有执行。".to_string(),
                8,
            );
            return;
        }
        let windows = action::post_close(&verified);
        let count = verified.len();
        let before_used = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.adapter_used);
        self.pending = Some(PendingClose {
            targets: verified,
            before_used,
            started: Instant::now(),
            force_asked: false,
        });
        let text = if windows > 0 {
            format!("正在关闭 {count} 个进程（已发送关闭请求）…")
        } else {
            format!("正在关闭 {count} 个进程（没有可见窗口，3 秒后会询问是否强制）…")
        };
        self.set_status(text, 120);
        self.request_sample();
    }

    fn check_pending_close(&mut self) {
        let (alive, elapsed, force_asked) = match &self.pending {
            Some(pending) => (
                pending
                    .targets
                    .iter()
                    .filter(|target| action::is_alive(target))
                    .copied()
                    .collect::<Vec<Target>>(),
                pending.started.elapsed(),
                pending.force_asked,
            ),
            None => return,
        };

        if alive.is_empty() {
            let pending = self.pending.take().expect("pending 刚检查过");
            let count = pending.targets.len();
            let after = self
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.adapter_used);
            let text = match (pending.before_used, after) {
                (Some(before), Some(after)) => format!(
                    "已结束 {count} 个进程 · 整卡已用 {} → {}",
                    format::bytes(before),
                    format::bytes(after)
                ),
                _ => format!("已结束 {count} 个进程"),
            };
            self.set_status(text, 15);
            return;
        }

        if elapsed >= Duration::from_secs(3) && !force_asked {
            // 先置位再弹窗：MessageBox 会泵消息，别让下一轮 timer 再弹一个。
            if let Some(pending) = &mut self.pending {
                pending.force_asked = true;
            }
            let count = alive.len();
            let text = wide(&format!(
                "还有 {count} 个进程没有退出。\n\n强制结束会让它们来不及保存，继续进行吗？"
            ));
            let caption = wide("强制结束");
            let answer = unsafe {
                MessageBoxW(
                    self.hwnd,
                    PCWSTR(text.as_ptr()),
                    PCWSTR(caption.as_ptr()),
                    MB_YESNO | MB_ICONINFORMATION,
                )
            };
            if answer == IDYES {
                let mut failed = 0usize;
                let mut first_error = None;
                for target in &alive {
                    if let Err(error) = action::terminate(target) {
                        failed += 1;
                        first_error.get_or_insert(error);
                    }
                }
                let message = if failed == 0 {
                    format!("已请求强制结束 {count} 个进程…")
                } else {
                    let reason = first_error
                        .map(|error| error.user_text())
                        .unwrap_or("系统没有给出更多原因。");
                    format!("{count} 个进程里有 {failed} 个结束失败：{reason}")
                };
                self.set_status(message, 15);
            } else {
                self.pending = None;
                self.set_status("已取消；进程还在运行。".to_string(), 10);
            }
            return;
        }

        if !force_asked {
            self.set_status(format!("还有 {} 个进程没退出…", alive.len()), 60);
        }
    }

    fn set_status(&mut self, text: String, seconds: u64) {
        self.status = Some((text, Instant::now() + Duration::from_secs(seconds)));
        self.update_footer();
    }

    fn check_status_expiry(&mut self) {
        let expired = self
            .status
            .as_ref()
            .is_some_and(|(_, until)| Instant::now() >= *until);
        if expired {
            self.status = None;
            self.update_footer();
        }
    }

    fn update_footer(&mut self) {
        if let Some((text, _)) = &self.status {
            set_text(self.footer, text);
            return;
        }
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let memory_mode = snapshot.mode == Mode::Memory;
        let base = if memory_mode {
            "这里显示的是内存占用，不是显存（读不到显存计数器）".to_string()
        } else {
            match snapshot.unattributed {
                Unattributed::Bytes(bytes) => format!(
                    "未归属 {}（驱动与共享资源，无法关闭）",
                    format::bytes_friendly(bytes)
                ),
                Unattributed::Overcounted(_) => {
                    "未归属 —（口径异常：进程合计超过整卡）".to_string()
                }
                Unattributed::Unavailable => "未归属 —（整卡读数不可用）".to_string(),
            }
        };
        let extra = match self.page {
            Page::Live => format!(
                "共 {} 组 · 双击分组可展开 · 右键结束",
                snapshot.groups.len()
            ),
            Page::Programs => format!(
                "共 {} 个程序 · 右键结束",
                crate::data::model::build_programs(&snapshot.groups).len()
            ),
        };
        set_text(self.footer, &format!("{base} · {extra}"));
    }

    fn show_help(&self) {
        let text = "为什么和任务管理器不一样？\n\n\
显存哨兵统计的是「本地显存」：只算真正落在这块显卡上的部分，同一台机器上各进程相加不会超过整卡。\n\
任务管理器默认显示「专用显存」：它会把跨进程共享的画面重复算给多个进程，空闲时相加也可能超过整卡，所以不能直接相加。\n\n\
本机空闲时，本地显存相加约占整卡的 94%，剩下的是驱动和共享资源，显示为「未归属」。\n\
容量也有差别：这里显示的是驱动报告的可用显存，比包装上的 8GB 略少，差额被驱动预留。\n\n\
两边数字不一样，是口径不同，不是谁读错了。";
        let text = wide(text);
        let caption = wide("口径说明");
        unsafe {
            MessageBoxW(
                self.hwnd,
                PCWSTR(text.as_ptr()),
                PCWSTR(caption.as_ptr()),
                MB_OK | MB_ICONINFORMATION,
            );
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
        let button_height = self.px(30);
        let refresh_width = self.px(84);
        let help_width = self.px(104);
        let gap = self.px(10);
        let footer_height = self.px(22);

        let help_x = width - margin - refresh_width - gap - help_width;
        let refresh_x = width - margin - refresh_width;
        unsafe {
            let _ = MoveWindow(
                self.header,
                margin,
                margin,
                (help_x - gap - margin).max(0),
                header_height,
                true,
            );
            let _ = MoveWindow(
                self.help,
                help_x,
                margin - self.px(2),
                help_width,
                button_height,
                true,
            );
            let _ = MoveWindow(
                self.refresh,
                refresh_x,
                margin - self.px(2),
                refresh_width,
                button_height,
                true,
            );
            let footer_y = height - margin - footer_height;
            let tab_y = margin + header_height + self.px(4);
            let tab_height = self.px(28);
            let _ = MoveWindow(
                self.tab,
                margin,
                tab_y,
                (width - margin * 2).max(0),
                tab_height,
                true,
            );
            let list_y = tab_y + tab_height + self.px(6);
            let _ = MoveWindow(
                self.list,
                margin,
                list_y,
                (width - margin * 2).max(0),
                (footer_y - self.px(6) - list_y).max(0),
                true,
            );
            let _ = MoveWindow(
                self.footer,
                margin,
                footer_y,
                (width - margin * 2).max(0),
                footer_height,
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
            let _ = DispatchMessageW(&message);
        }
    }
    Ok(())
}

fn insert_tab(tab: HWND, text: &str, index: usize) {
    let mut buffer = wide(text);
    let mut item = TCITEMW {
        mask: TCIF_TEXT,
        pszText: PWSTR(buffer.as_mut_ptr()),
        cchTextMax: buffer.len() as i32 - 1,
        ..Default::default()
    };
    unsafe {
        SendMessageW(
            tab,
            TCM_INSERTITEMW,
            WPARAM(index),
            LPARAM(&mut item as *mut _ as isize),
        );
    }
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

fn risk_color(risk: Risk) -> u32 {
    match risk {
        Risk::Forbidden => 0x001C_2BC4,
        Risk::Caution => 0x0000_679A,
        Risk::Safe => 0x0010_7C10,
        Risk::Unknown => 0x0020_2020,
    }
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
