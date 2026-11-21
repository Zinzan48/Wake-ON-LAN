use crate::icon::generate_app_icon_rgba;
use std::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    ShowWindow,
    NotificationClicked,
    WakeAllOffline,
    CheckNow,
    TogglePause,
    Exit,
}

/// 使用 Windows 11/10 原生 DwmSetWindowAttribute 同步視窗標題列底色與文字色，
/// 解決深色系統下切換淺色模式時標題列黑底黑字的 DWM 快取異常，並讓標題列無縫融入主題配色。
pub fn apply_windows_titlebar_colors(hwnd_raw: isize, dark: bool) {
    #[cfg(windows)]
    {
        use std::ffi::c_void;
        use windows_sys::Win32::Foundation::HWND;
        use windows_sys::Win32::Graphics::Dwm::{
            DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
            DwmSetWindowAttribute,
        };

        let hwnd = hwnd_raw as HWND;
        if hwnd.is_null() {
            return;
        }
        let use_dark: i32 = if dark { 1 } else { 0 };
        // COLORREF 格式：0x00BBGGRR
        let caption_color: u32 = if dark {
            0x0017_1514 // #141517 暗黑底色
        } else {
            0x00EF_F3F4 // #F4F3EF 溫潤工程紙淺色
        };
        let text_color: u32 = if dark {
            0x00E4_EAEC // #ECEAE4 溫白字色
        } else {
            0x0018_1B1C // #1C1B18 深暖炭字色
        };

        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                &use_dark as *const i32 as *const c_void,
                std::mem::size_of::<i32>() as u32,
            );
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_CAPTION_COLOR as u32,
                &caption_color as *const u32 as *const c_void,
                std::mem::size_of::<u32>() as u32,
            );
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_TEXT_COLOR as u32,
                &text_color as *const u32 as *const c_void,
                std::mem::size_of::<u32>() as u32,
            );
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (hwnd_raw, dark);
    }
}

/// 使用 Windows 原生 GetLocalTime 取得本地時間字串 (HH:MM:SS)
pub fn local_time_hhmmss() -> String {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::SYSTEMTIME;
        use windows_sys::Win32::System::SystemInformation::GetLocalTime;
        let mut st = SYSTEMTIME {
            wYear: 0,
            wMonth: 0,
            wDayOfWeek: 0,
            wDay: 0,
            wHour: 0,
            wMinute: 0,
            wSecond: 0,
            wMilliseconds: 0,
        };
        unsafe {
            GetLocalTime(&mut st);
        }
        format!("{:02}:{:02}:{:02}", st.wHour, st.wMinute, st.wSecond)
    }
    #[cfg(not(windows))]
    {
        "00:00:00".to_string()
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::ffi::c_void;
    use std::ptr;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, LRESULT, POINT,
        WPARAM,
    };
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
        DeleteObject,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::UI::Shell::{
        NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY,
        NIN_BALLOONUSERCLICK, NOTIFYICONDATAW, Shell_NotifyIconW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
        DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, FindWindowW, GWLP_USERDATA,
        GetCursorPos, GetMessageW, GetWindowLongPtrW, HICON, ICONINFO, IDI_APPLICATION, LoadIconW,
        MF_SEPARATOR, MF_STRING, MSG, PostMessageW, PostQuitMessage, RegisterClassW,
        RegisterWindowMessageW, SetForegroundWindow, SetWindowLongPtrW, TPM_BOTTOMALIGN,
        TPM_LEFTALIGN, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_COMMAND, WM_DESTROY,
        WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_RBUTTONUP, WM_USER, WNDCLASSW, WS_OVERLAPPED,
    };

    const TRAY_WINDOW_CLASS: &str = "WakeOnLanToolTrayWindowClass";
    const TRAY_WINDOW_TITLE: &str = "WakeOnLanToolTray";
    const SINGLE_INSTANCE_MUTEX_NAME: &str = "Local\\WakeOnLanTool_SingleInstance_v1";

    const WM_TRAY_CALLBACK: u32 = WM_USER + 1;
    const WM_TRAY_CMD_WAKEUP: u32 = WM_USER + 2;
    const WM_TRAY_SHOW_EXISTING: u32 = WM_USER + 3;
    const TRAY_UID: u32 = 1001;

    const CMD_SHOW: usize = 101;
    const CMD_WAKE_ALL: usize = 102;
    const CMD_CHECK_NOW: usize = 103;
    const CMD_TOGGLE_PAUSE: usize = 104;
    const CMD_EXIT: usize = 105;

    /// Windows 原生具名 Mutex 單一實例守護物件
    pub struct SingleInstanceGuard {
        handle: HANDLE,
    }

    impl Drop for SingleInstanceGuard {
        fn drop(&mut self) {
            if !self.handle.is_null() {
                unsafe {
                    CloseHandle(self.handle);
                }
            }
        }
    }

    /// 取得單一實例鎖；若程式已開啟（含縮在系統匣），直接發送原生訊息喚回既有視窗並回傳 None
    pub fn acquire_single_instance_or_activate_existing() -> Option<SingleInstanceGuard> {
        acquire_named_mutex(SINGLE_INSTANCE_MUTEX_NAME, true)
    }

    pub(crate) fn acquire_named_mutex(
        name: &str,
        activate_existing_window: bool,
    ) -> Option<SingleInstanceGuard> {
        let mutex_name = to_wide_null(name);
        let handle = unsafe { CreateMutexW(ptr::null(), 0, mutex_name.as_ptr()) };
        if handle.is_null() {
            // 無法建立 Mutex 時不阻擋啟動
            return Some(SingleInstanceGuard {
                handle: ptr::null_mut(),
            });
        }

        let last_err = unsafe { GetLastError() };
        if last_err == ERROR_ALREADY_EXISTS {
            unsafe {
                CloseHandle(handle);
            }
            if activate_existing_window {
                let class_w = to_wide_null(TRAY_WINDOW_CLASS);
                let title_w = to_wide_null(TRAY_WINDOW_TITLE);
                let existing_hwnd = unsafe { FindWindowW(class_w.as_ptr(), title_w.as_ptr()) };
                if !existing_hwnd.is_null() {
                    unsafe {
                        PostMessageW(existing_hwnd, WM_TRAY_SHOW_EXISTING, 0, 0);
                    }
                }
            }
            return None;
        }

        Some(SingleInstanceGuard { handle })
    }

    enum OutgoingCmd {
        Tooltip(String),
        Toast { title: String, body: String },
        SetPaused(bool),
        Shutdown,
    }

    struct TrayWindowState {
        event_tx: mpsc::Sender<TrayEvent>,
        on_wake_ui: Arc<dyn Fn(TrayEvent) + Send + Sync>,
        cmd_rx: mpsc::Receiver<OutgoingCmd>,
        icon: HICON,
        owns_icon: bool,
        taskbar_created_msg: u32,
        paused: bool,
        tooltip: String,
    }

    #[derive(Clone)]
    pub struct TrayHandle {
        hwnd: Arc<Mutex<Option<isize>>>,
        cmd_tx: mpsc::Sender<OutgoingCmd>,
    }

    impl TrayHandle {
        pub fn spawn(
            on_wake_ui: Arc<dyn Fn(TrayEvent) + Send + Sync>,
        ) -> (Self, mpsc::Receiver<TrayEvent>) {
            let (event_tx, event_rx) = mpsc::channel();
            let (cmd_tx, cmd_rx) = mpsc::channel();
            let hwnd_slot: Arc<Mutex<Option<isize>>> = Arc::new(Mutex::new(None));
            let hwnd_slot_thread = Arc::clone(&hwnd_slot);

            thread::spawn(move || {
                run_tray_loop(hwnd_slot_thread, event_tx, cmd_rx, on_wake_ui);
            });

            (
                Self {
                    hwnd: hwnd_slot,
                    cmd_tx,
                },
                event_rx,
            )
        }

        pub fn update_tooltip(&self, text: impl Into<String>) {
            let _ = self.cmd_tx.send(OutgoingCmd::Tooltip(text.into()));
            self.poke();
        }

        pub fn show_offline_toast(&self, title: impl Into<String>, body: impl Into<String>) {
            let _ = self.cmd_tx.send(OutgoingCmd::Toast {
                title: title.into(),
                body: body.into(),
            });
            self.poke();
        }

        pub fn set_paused(&self, paused: bool) {
            let _ = self.cmd_tx.send(OutgoingCmd::SetPaused(paused));
            self.poke();
        }

        pub fn shutdown(&self) {
            let _ = self.cmd_tx.send(OutgoingCmd::Shutdown);
            self.poke();
        }

        fn poke(&self) {
            if let Ok(guard) = self.hwnd.lock()
                && let Some(raw_hwnd) = *guard
            {
                unsafe {
                    PostMessageW(raw_hwnd as HWND, WM_TRAY_CMD_WAKEUP, 0, 0);
                }
            }
        }
    }

    fn to_wide_null(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn copy_wide_trunc(dst: &mut [u16], src: &str) {
        dst.fill(0);
        let max_chars = dst.len().saturating_sub(1);
        for (i, ch) in src.encode_utf16().take(max_chars).enumerate() {
            dst[i] = ch;
        }
    }

    /// 使用 32-bit Alpha DIB + CreateIconIndirect 建立平滑反鋸齒的系統匣圖示
    fn create_native_power_icon() -> (HICON, bool) {
        const SIZE: i32 = 32;
        let rgba = generate_app_icon_rgba(SIZE as u32);

        let mut bmi: BITMAPINFO = unsafe { std::mem::zeroed() };
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: SIZE,
            biHeight: -SIZE, // Top-down DIB
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            biSizeImage: (SIZE * SIZE * 4) as u32,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        };

        let mut bits_ptr: *mut c_void = ptr::null_mut();
        let hbm_color = unsafe {
            CreateDIBSection(
                ptr::null_mut(),
                &bmi,
                DIB_RGB_COLORS,
                &mut bits_ptr,
                ptr::null_mut(),
                0,
            )
        };

        if !hbm_color.is_null() && !bits_ptr.is_null() {
            let dst =
                unsafe { std::slice::from_raw_parts_mut(bits_ptr as *mut u8, (SIZE * SIZE * 4) as usize) };
            for i in (0..rgba.len()).step_by(4) {
                dst[i] = rgba[i + 2]; // B
                dst[i + 1] = rgba[i + 1]; // G
                dst[i + 2] = rgba[i]; // R
                dst[i + 3] = rgba[i + 3]; // A
            }

            let hbm_mask = unsafe { CreateBitmap(SIZE, SIZE, 1, 1, ptr::null()) };
            if !hbm_mask.is_null() {
                let icon_info = ICONINFO {
                    fIcon: 1,
                    xHotspot: 0,
                    yHotspot: 0,
                    hbmMask: hbm_mask,
                    hbmColor: hbm_color,
                };
                let hicon = unsafe { CreateIconIndirect(&icon_info) };
                unsafe {
                    DeleteObject(hbm_mask);
                    DeleteObject(hbm_color);
                }
                if !hicon.is_null() {
                    return (hicon, true);
                }
            } else {
                unsafe {
                    DeleteObject(hbm_color);
                }
            }
        }

        let fallback = unsafe { LoadIconW(ptr::null_mut(), IDI_APPLICATION) };
        (fallback, false)
    }

    fn add_tray_icon(hwnd: HWND, icon: HICON, tooltip: &str) {
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = TRAY_UID;
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY_CALLBACK;
        nid.hIcon = icon;
        copy_wide_trunc(&mut nid.szTip, tooltip);
        unsafe {
            Shell_NotifyIconW(NIM_ADD, &nid);
        }
    }

    fn modify_tray_tooltip(hwnd: HWND, tooltip: &str) {
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = TRAY_UID;
        nid.uFlags = NIF_TIP;
        copy_wide_trunc(&mut nid.szTip, tooltip);
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    fn show_tray_balloon_toast(hwnd: HWND, title: &str, body: &str) {
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = TRAY_UID;
        nid.uFlags = NIF_INFO;
        nid.dwInfoFlags = NIIF_WARNING;
        copy_wide_trunc(&mut nid.szInfoTitle, title);
        copy_wide_trunc(&mut nid.szInfo, body);
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    fn delete_tray_icon(hwnd: HWND) {
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = TRAY_UID;
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &nid);
        }
    }

    fn show_context_menu(hwnd: HWND, paused: bool) {
        unsafe {
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return;
            }
            let show_txt = to_wide_null("顯示主視窗");
            let wake_all_txt = to_wide_null("喚醒全部關機設備");
            let check_txt = to_wide_null("重新檢測");
            let pause_txt = to_wide_null(if paused {
                "開始監控"
            } else {
                "暫停監控"
            });
            let exit_txt = to_wide_null("結束程式");

            AppendMenuW(menu, MF_STRING, CMD_SHOW, show_txt.as_ptr());
            AppendMenuW(menu, MF_STRING, CMD_WAKE_ALL, wake_all_txt.as_ptr());
            AppendMenuW(menu, MF_STRING, CMD_CHECK_NOW, check_txt.as_ptr());
            AppendMenuW(menu, MF_STRING, CMD_TOGGLE_PAUSE, pause_txt.as_ptr());
            AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
            AppendMenuW(menu, MF_STRING, CMD_EXIT, exit_txt.as_ptr());

            let mut pt = POINT { x: 0, y: 0 };
            GetCursorPos(&mut pt);
            SetForegroundWindow(hwnd);
            TrackPopupMenu(
                menu,
                TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_LEFTALIGN,
                pt.x,
                pt.y,
                0,
                hwnd,
                ptr::null(),
            );
            DestroyMenu(menu);
        }
    }

    fn emit_event(state: &TrayWindowState, event: TrayEvent) {
        let _ = state.event_tx.send(event);
        (state.on_wake_ui)(event);
    }

    unsafe extern "system" fn tray_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut TrayWindowState;
        if !state_ptr.is_null() {
            let state = unsafe { &mut *state_ptr };

            if msg == state.taskbar_created_msg && state.taskbar_created_msg != 0 {
                add_tray_icon(hwnd, state.icon, &state.tooltip);
                return 0;
            }

            match msg {
                WM_TRAY_SHOW_EXISTING => {
                    emit_event(state, TrayEvent::ShowWindow);
                    return 0;
                }
                WM_TRAY_CALLBACK => {
                    let event_code = (lparam & 0xFFFF) as u32;
                    match event_code {
                        WM_LBUTTONUP | WM_LBUTTONDBLCLK => {
                            emit_event(state, TrayEvent::ShowWindow);
                        }
                        NIN_BALLOONUSERCLICK => {
                            emit_event(state, TrayEvent::NotificationClicked);
                        }
                        WM_RBUTTONUP => {
                            show_context_menu(hwnd, state.paused);
                        }
                        _ => {}
                    }
                    return 0;
                }
                WM_COMMAND => {
                    let cmd_id = wparam & 0xFFFF;
                    match cmd_id {
                        CMD_SHOW => emit_event(state, TrayEvent::ShowWindow),
                        CMD_WAKE_ALL => emit_event(state, TrayEvent::WakeAllOffline),
                        CMD_CHECK_NOW => emit_event(state, TrayEvent::CheckNow),
                        CMD_TOGGLE_PAUSE => emit_event(state, TrayEvent::TogglePause),
                        CMD_EXIT => emit_event(state, TrayEvent::Exit),
                        _ => {}
                    }
                    return 0;
                }
                WM_TRAY_CMD_WAKEUP => {
                    while let Ok(cmd) = state.cmd_rx.try_recv() {
                        match cmd {
                            OutgoingCmd::Tooltip(text) => {
                                state.tooltip = text;
                                modify_tray_tooltip(hwnd, &state.tooltip);
                            }
                            OutgoingCmd::Toast { title, body } => {
                                show_tray_balloon_toast(hwnd, &title, &body);
                            }
                            OutgoingCmd::SetPaused(p) => {
                                state.paused = p;
                            }
                            OutgoingCmd::Shutdown => {
                                delete_tray_icon(hwnd);
                                unsafe {
                                    DestroyWindow(hwnd);
                                }
                                return 0;
                            }
                        }
                    }
                    return 0;
                }
                WM_DESTROY => {
                    delete_tray_icon(hwnd);
                    unsafe {
                        PostQuitMessage(0);
                    }
                    return 0;
                }
                _ => {}
            }
        }

        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    fn run_tray_loop(
        hwnd_slot: Arc<Mutex<Option<isize>>>,
        event_tx: mpsc::Sender<TrayEvent>,
        cmd_rx: mpsc::Receiver<OutgoingCmd>,
        on_wake_ui: Arc<dyn Fn(TrayEvent) + Send + Sync>,
    ) {
        let hinstance = unsafe { GetModuleHandleW(ptr::null()) };
        let class_name = to_wide_null(TRAY_WINDOW_CLASS);
        let title_name = to_wide_null(TRAY_WINDOW_TITLE);
        let taskbar_msg_name = to_wide_null("TaskbarCreated");

        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(tray_wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: ptr::null_mut(),
            hCursor: ptr::null_mut(),
            hbrBackground: ptr::null_mut(),
            lpszMenuName: ptr::null(),
            lpszClassName: class_name.as_ptr(),
        };

        unsafe {
            RegisterClassW(&wc);
        }

        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                title_name.as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                hinstance,
                ptr::null(),
            )
        };

        if hwnd.is_null() {
            return;
        }

        let (icon, owns_icon) = create_native_power_icon();
        let taskbar_created_msg = unsafe { RegisterWindowMessageW(taskbar_msg_name.as_ptr()) };
        let initial_tooltip = "Wake-on-LAN - 待命中".to_string();

        let boxed_state = Box::new(TrayWindowState {
            event_tx,
            on_wake_ui,
            cmd_rx,
            icon,
            owns_icon,
            taskbar_created_msg,
            paused: false,
            tooltip: initial_tooltip.clone(),
        });
        let raw_state = Box::into_raw(boxed_state);

        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw_state as isize);
        }
        add_tray_icon(hwnd, icon, &initial_tooltip);

        if let Ok(mut guard) = hwnd_slot.lock() {
            *guard = Some(hwnd as isize);
        }
        unsafe {
            PostMessageW(hwnd, WM_TRAY_CMD_WAKEUP, 0, 0);
        }

        let mut msg: MSG = unsafe { std::mem::zeroed() };
        while unsafe { GetMessageW(&mut msg, ptr::null_mut(), 0, 0) } > 0 {
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        if let Ok(mut guard) = hwnd_slot.lock() {
            *guard = None;
        }

        let state = unsafe { Box::from_raw(raw_state) };
        if state.owns_icon && !state.icon.is_null() {
            unsafe {
                DestroyIcon(state.icon);
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    use std::sync::Arc;

    pub struct SingleInstanceGuard;

    pub fn acquire_single_instance_or_activate_existing() -> Option<SingleInstanceGuard> {
        Some(SingleInstanceGuard)
    }

    #[derive(Clone)]
    pub struct TrayHandle;

    impl TrayHandle {
        pub fn spawn(
            _on_wake_ui: Arc<dyn Fn(TrayEvent) + Send + Sync>,
        ) -> (Self, mpsc::Receiver<TrayEvent>) {
            let (_tx, rx) = mpsc::channel();
            (Self, rx)
        }
        pub fn update_tooltip(&self, _text: impl Into<String>) {}
        pub fn show_offline_toast(&self, _title: impl Into<String>, _body: impl Into<String>) {}
        pub fn set_paused(&self, _paused: bool) {}
        pub fn shutdown(&self) {}
    }
}

pub use platform::{TrayHandle, acquire_single_instance_or_activate_existing};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_time_and_icon_generation() {
        let t = local_time_hhmmss();
        assert_eq!(t.len(), 8);
        assert_eq!(&t[2..3], ":");
        assert_eq!(&t[5..6], ":");

        let rgba32 = generate_app_icon_rgba(32);
        assert_eq!(rgba32.len(), 32 * 32 * 4);
        assert!(rgba32.iter().any(|&b| b > 0));

        let rgba64 = generate_app_icon_rgba(64);
        assert_eq!(rgba64.len(), 64 * 64 * 4);
        assert!(rgba64.iter().any(|&b| b > 0));
    }

    #[cfg(windows)]
    #[test]
    fn single_instance_mutex_prevents_second_lock() {
        let test_name = format!(
            "Local\\WakeOnLanTool_UnitTest_{}",
            crate::model::generate_id("mtx")
        );
        let first = platform::acquire_named_mutex(&test_name, false);
        assert!(first.is_some(), "第一次取得單一實例鎖應成功");
        let second = platform::acquire_named_mutex(&test_name, false);
        assert!(second.is_none(), "重複開啟時應偵測到已有實例並回傳 None");
        drop(first);
        let third = platform::acquire_named_mutex(&test_name, false);
        assert!(third.is_some(), "釋放後應可重新取得單一實例鎖");
    }
}
