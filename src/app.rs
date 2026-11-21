use crate::model::{
    AppConfig, Device, DeviceRuntime, LocalSubnet, PendingWakeAlert, PowerStatus, SavedSubnet,
    ScannedHost, ThemeMode, generate_id,
};
use crate::monitor::{MonitorController, MonitorEvent};
use crate::storage;
use crate::ui_icons::{
    ButtonStyle, Palette, UiIcon, hairline_h, icon_only_ghost, paint_icon, segment_choice,
    status_pill, styled_button,
};
use crate::win_net::{
    MAX_SCAN_HOSTS, QueryMode, ScanEvent, detect_local_subnets, format_mac,
    ipv4_network_and_broadcast, parse_mac, parse_subnet_spec, send_wol, start_subnet_scan,
};
use crate::win_tray::{
    TrayEvent, TrayHandle, apply_windows_titlebar_colors, local_time_hhmmss,
};
use eframe::egui::{self, Color32, Margin, RichText, Stroke, pos2, vec2};
use eframe::wgpu::rwh::{HasWindowHandle, RawWindowHandle};
use std::collections::{HashMap, HashSet};
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

/// 執行期載入 Windows 系統繁體中文字型（維持單一執行檔輕量化）
pub fn configure_chinese_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let candidates = [
        r"C:\Windows\Fonts\msjh.ttc",
        r"C:\Windows\Fonts\mingliu.ttc",
        r"C:\Windows\Fonts\msyh.ttc",
    ];

    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(
                "zh_tw_sys".to_owned(),
                Arc::new(egui::FontData::from_owned(bytes)),
            );
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "zh_tw_sys".to_owned());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push("zh_tw_sys".to_owned());
            break;
        }
    }
    ctx.set_fonts(fonts);
}

fn apply_theme_mode(ctx: &egui::Context, mode: ThemeMode) {
    let pref = match mode {
        ThemeMode::System => egui::ThemePreference::System,
        ThemeMode::Light => egui::ThemePreference::Light,
        ThemeMode::Dark => egui::ThemePreference::Dark,
    };
    ctx.set_theme(pref);
}

/// 建構精密網路儀表板專屬 egui::Visuals（預先註冊給 Light / Dark，不干擾原生標題列主題同步）
fn build_instrument_visuals(dark: bool) -> egui::Visuals {
    let pal = Palette::for_mode(dark);
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    visuals.panel_fill = pal.canvas;
    visuals.window_fill = pal.surface;
    visuals.window_stroke = Stroke::new(1.0, pal.hairline);
    visuals.extreme_bg_color = pal.input_bg;
    visuals.faint_bg_color = pal.sidebar;
    visuals.override_text_color = Some(pal.ink_primary);
    visuals.selection.bg_fill = pal.accent_soft;
    visuals.selection.stroke = Stroke::new(1.0, pal.accent);

    visuals.widgets.noninteractive.bg_fill = pal.surface;
    visuals.widgets.noninteractive.weak_bg_fill = pal.sidebar;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, pal.hairline);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, pal.ink_primary);

    visuals.widgets.inactive.bg_fill = pal.surface;
    visuals.widgets.inactive.weak_bg_fill = pal.surface;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, pal.hairline);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, pal.ink_primary);

    visuals.widgets.hovered.bg_fill = pal.surface_hover;
    visuals.widgets.hovered.weak_bg_fill = pal.surface_hover;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, pal.accent);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, pal.ink_primary);

    visuals.widgets.active.bg_fill = pal.accent_soft;
    visuals.widgets.active.weak_bg_fill = pal.accent_soft;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, pal.accent);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, pal.ink_primary);

    visuals
}

/// 單行文字排版（超出寬度自動以省略號 `…` 截斷，絕不換行造成重疊）
fn layout_single_line_ellipsis(
    painter: &egui::Painter,
    text: String,
    font_id: egui::FontId,
    color: Color32,
    max_width: f32,
) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text, font_id, color);
    job.wrap = egui::text::TextWrapping {
        max_width: max_width.max(16.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    painter.layout_job(job)
}

struct DeviceForm {
    editing_id: Option<String>,
    name: String,
    ip: String,
    mac: String,
    broadcast_ip: String,
    wol_port: String,
    tcp_probe_port: String,
    monitored: bool,
    error: Option<String>,
}

impl DeviceForm {
    fn new_empty(default_broadcast: &str) -> Self {
        Self {
            editing_id: None,
            name: String::new(),
            ip: String::new(),
            mac: String::new(),
            broadcast_ip: default_broadcast.to_string(),
            wol_port: "9".to_string(),
            tcp_probe_port: String::new(),
            monitored: true,
            error: None,
        }
    }

    fn from_device(dev: &Device) -> Self {
        Self {
            editing_id: Some(dev.id.clone()),
            name: dev.name.clone(),
            ip: dev.ip.clone(),
            mac: dev.mac.clone(),
            broadcast_ip: dev.broadcast_ip.clone(),
            wol_port: dev.wol_port.to_string(),
            tcp_probe_port: dev
                .tcp_probe_port
                .map(|p| p.to_string())
                .unwrap_or_default(),
            monitored: dev.monitored,
            error: None,
        }
    }
}

pub struct WakeOnLanApp {
    config: AppConfig,
    local_subnets: Vec<LocalSubnet>,

    // 網段／特定 IP 輸入與儲存（預帶但不主動查詢）
    subnet_input: String,
    subnet_label_input: String,
    show_save_subnet_popover: bool,

    // 手動掃描狀態
    is_scanning: bool,
    scan_progress: (usize, usize),
    scan_cancel: Option<Arc<AtomicBool>>,
    scan_rx: Option<mpsc::Receiver<ScanEvent>>,
    scanned_hosts: Vec<ScannedHost>,

    // 背景監控與提醒狀態
    monitor: MonitorController,
    monitor_rx: mpsc::Receiver<MonitorEvent>,
    is_probing: bool,
    runtimes: HashMap<String, DeviceRuntime>,
    pending_alerts: Vec<PendingWakeAlert>,

    // 系統匣與關閉控制
    tray: TrayHandle,
    tray_rx: mpsc::Receiver<TrayEvent>,
    allow_exit: Arc<AtomicBool>,

    // 介面互動彈窗與訊息
    show_settings_window: bool,
    device_form: Option<DeviceForm>,
    status_banner: Option<(String, bool)>,
    last_titlebar_dark: Option<bool>,
    titlebar_sync_passes: u8,
}

impl WakeOnLanApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        config: AppConfig,
        local_subnets: Vec<LocalSubnet>,
    ) -> Self {
        configure_chinese_fonts(&cc.egui_ctx);
        cc.egui_ctx
            .set_visuals_of(egui::Theme::Light, build_instrument_visuals(false));
        cc.egui_ctx
            .set_visuals_of(egui::Theme::Dark, build_instrument_visuals(true));
        apply_theme_mode(&cc.egui_ctx, config.settings.theme);

        // 啟動預帶網段（絕不主動開始掃描）
        let initial_subnet = config
            .settings
            .last_subnet_input
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                config
                    .saved_subnets
                    .iter()
                    .find(|s| s.is_default)
                    .or_else(|| config.saved_subnets.first())
                    .map(|s| s.cidr.clone())
            })
            .or_else(|| local_subnets.first().map(LocalSubnet::preferred_scan_cidr))
            .unwrap_or_else(|| "192.168.1.0/24".to_string());

        let allow_exit = Arc::new(AtomicBool::new(false));
        let egui_ctx_for_tray = cc.egui_ctx.clone();
        let allow_exit_for_tray = Arc::clone(&allow_exit);

        let monitor_slot: Arc<std::sync::Mutex<Option<MonitorController>>> =
            Arc::new(std::sync::Mutex::new(None));
        let monitor_slot_for_tray = Arc::clone(&monitor_slot);

        let (tray, tray_rx) = TrayHandle::spawn(Arc::new(move |ev| match ev {
            TrayEvent::ShowWindow | TrayEvent::NotificationClicked => {
                egui_ctx_for_tray.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                egui_ctx_for_tray.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                egui_ctx_for_tray.send_viewport_cmd(egui::ViewportCommand::Focus);
                egui_ctx_for_tray.request_repaint();
            }
            TrayEvent::WakeAllOffline => {
                if let Ok(guard) = monitor_slot_for_tray.lock()
                    && let Some(mon) = guard.as_ref()
                {
                    let _ = mon.wake_all_offline_now();
                }
                egui_ctx_for_tray.request_repaint();
            }
            TrayEvent::CheckNow => {
                if let Ok(guard) = monitor_slot_for_tray.lock()
                    && let Some(mon) = guard.as_ref()
                {
                    mon.trigger_check_now();
                }
                egui_ctx_for_tray.request_repaint();
            }
            TrayEvent::TogglePause => {
                if let Ok(guard) = monitor_slot_for_tray.lock()
                    && let Some(mon) = guard.as_ref()
                {
                    let _ = mon.toggle_pause();
                }
                egui_ctx_for_tray.request_repaint();
            }
            TrayEvent::Exit => {
                allow_exit_for_tray.store(true, Ordering::SeqCst);
                egui_ctx_for_tray.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                egui_ctx_for_tray.send_viewport_cmd(egui::ViewportCommand::Close);
                egui_ctx_for_tray.request_repaint();
            }
        }));

        tray.set_paused(config.settings.monitor_paused);

        let egui_ctx_for_mon = cc.egui_ctx.clone();
        let (monitor, monitor_rx) = MonitorController::start(
            config.devices.clone(),
            config.settings.clone(),
            tray.clone(),
            Arc::new(move || {
                egui_ctx_for_mon.request_repaint();
            }),
        );

        if let Ok(mut guard) = monitor_slot.lock() {
            *guard = Some(monitor.clone());
        }

        Self {
            config,
            local_subnets,
            subnet_input: initial_subnet,
            subnet_label_input: String::new(),
            show_save_subnet_popover: false,
            is_scanning: false,
            scan_progress: (0, 0),
            scan_cancel: None,
            scan_rx: None,
            scanned_hosts: Vec::new(),
            monitor,
            monitor_rx,
            is_probing: false,
            runtimes: HashMap::new(),
            pending_alerts: Vec::new(),
            tray,
            tray_rx,
            allow_exit,
            show_settings_window: false,
            device_form: None,
            status_banner: None,
            last_titlebar_dark: None,
            titlebar_sync_passes: 3,
        }
    }

    fn persist_and_sync(&mut self) {
        self.config.settings.last_subnet_input = Some(self.subnet_input.trim().to_string());
        if let Err(err) = storage::save_config(&self.config) {
            self.set_banner(format!("儲存設定失敗：{err}"), true);
        }
        self.tray.set_paused(self.config.settings.monitor_paused);
        self.monitor
            .sync_config(self.config.devices.clone(), self.config.settings.clone());
    }

    fn set_banner(&mut self, msg: impl Into<String>, is_error: bool) {
        self.status_banner = Some((format!("[{}] {}", local_time_hhmmss(), msg.into()), is_error));
    }

    fn default_broadcast_hint(&self) -> String {
        if let Ok(target) = parse_subnet_spec(&self.subnet_input) {
            return target.broadcast_ip.to_string();
        }
        self.local_subnets
            .first()
            .map(|s| s.broadcast_ip.to_string())
            .unwrap_or_else(|| "192.168.1.255".to_string())
    }

    fn start_manual_scan(&mut self, ctx: &egui::Context) {
        if self.is_scanning {
            return;
        }
        match parse_subnet_spec(&self.subnet_input) {
            Ok(target) => {
                self.subnet_input = target.normalized.clone();
                self.persist_and_sync();

                let total = target.hosts.len();
                let cancel = Arc::new(AtomicBool::new(false));
                let (tx, rx) = mpsc::channel();
                let repaint_ctx = ctx.clone();

                self.is_scanning = true;
                self.scan_progress = (0, total);
                self.scan_cancel = Some(Arc::clone(&cancel));
                self.scan_rx = Some(rx);
                self.scanned_hosts.clear();

                let action_msg = match target.mode {
                    QueryMode::SingleIp => format!("正在查詢 {}", target.normalized),
                    QueryMode::SpecificIps => {
                        format!("正在查詢 {} 個指定 IP", total)
                    }
                    QueryMode::Range => {
                        format!("正在掃描 {}（{total} 個 IP）", target.normalized)
                    }
                    QueryMode::Subnet => {
                        if let Some(orig) = target.auto_narrowed_from {
                            format!(
                                "已由 /{orig} 縮減為 {}，開始掃描（{total} 個 IP）",
                                target.normalized
                            )
                        } else {
                            format!("正在掃描 {}（{total} 個 IP）", target.normalized)
                        }
                    }
                };
                self.set_banner(action_msg, false);

                start_subnet_scan(
                    target,
                    cancel,
                    tx,
                    Arc::new(move || repaint_ctx.request_repaint()),
                );
            }
            Err(err) => {
                self.set_banner(err, true);
            }
        }
    }

    fn cancel_scan(&mut self) {
        if let Some(flag) = &self.scan_cancel {
            flag.store(true, Ordering::Relaxed);
        }
    }

    fn save_current_subnet(&mut self) {
        match parse_subnet_spec(&self.subnet_input) {
            Ok(target) => {
                let cidr = target.normalized;
                let label = if self.subnet_label_input.trim().is_empty() {
                    cidr.clone()
                } else {
                    self.subnet_label_input.trim().to_string()
                };

                if let Some(existing) = self
                    .config
                    .saved_subnets
                    .iter_mut()
                    .find(|s| s.cidr == cidr)
                {
                    existing.label = label.clone();
                    self.set_banner(format!("已更新儲存網段：{label} ({cidr})"), false);
                } else {
                    let is_first = self.config.saved_subnets.is_empty();
                    self.config.saved_subnets.push(SavedSubnet {
                        id: generate_id("subnet"),
                        label: label.clone(),
                        cidr: cidr.clone(),
                        is_default: is_first,
                    });
                    self.set_banner(format!("已儲存網段：{label} ({cidr})"), false);
                }
                self.subnet_input = cidr;
                self.subnet_label_input.clear();
                self.show_save_subnet_popover = false;
                self.persist_and_sync();
            }
            Err(err) => self.set_banner(err, true),
        }
    }

    fn wake_device_by_id(&mut self, device_id: &str) {
        let Some(dev) = self.config.devices.iter().find(|d| d.id == device_id).cloned() else {
            return;
        };
        let bcast = dev.broadcast_ip.parse::<Ipv4Addr>().ok();
        match parse_mac(&dev.mac).and_then(|mac| send_wol(mac, bcast, dev.wol_port)) {
            Ok(targets) => {
                self.monitor.mark_device_waking(&dev.id);
                self.pending_alerts.retain(|a| a.device_id != dev.id);
                self.set_banner(
                    format!(
                        "已發送 WoL 封包給「{}」({}) → {}",
                        dev.name,
                        dev.mac,
                        targets.join(", ")
                    ),
                    false,
                );
            }
            Err(err) => {
                self.set_banner(format!("喚醒「{}」失敗：{err}", dev.name), true);
            }
        }
    }

    fn add_scanned_host_to_devices(&mut self, host: &ScannedHost) {
        let Some(mac) = &host.mac else {
            self.set_banner(
                format!(
                    "主機 {} 未取得 MAC 位址（非同 L2 區網），請改用手動新增填入 MAC。",
                    host.ip
                ),
                true,
            );
            return;
        };

        let ip_str = host.ip.to_string();
        if self
            .config
            .devices
            .iter()
            .any(|d| d.mac.eq_ignore_ascii_case(mac) || d.ip == ip_str)
        {
            self.set_banner(format!("設備 {ip_str} ({mac}) 已在監控清單中"), false);
            return;
        }

        let dev = Device {
            id: generate_id("dev"),
            name: format!("主機 {}", host.ip),
            ip: ip_str.clone(),
            mac: mac.clone(),
            broadcast_ip: host.broadcast_ip.to_string(),
            wol_port: 9,
            monitored: true,
            tcp_probe_port: None,
            last_seen_text: Some(local_time_hhmmss()),
        };
        self.config.devices.push(dev);
        self.persist_and_sync();
        self.monitor.trigger_check_now();
        self.set_banner(format!("已將 {ip_str} ({mac}) 加入監控清單"), false);
    }

    fn drain_background_events(&mut self) {
        let mut scan_done = false;
        let scan_events: Vec<ScanEvent> = self
            .scan_rx
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for ev in scan_events {
            match ev {
                ScanEvent::Progress { done, total } => {
                    self.scan_progress = (done, total);
                }
                ScanEvent::HostFound(host) => {
                    self.scanned_hosts.push(host);
                    self.scanned_hosts.sort_by_key(|h| u32::from(h.ip));
                }
                ScanEvent::Finished { cancelled } => {
                    scan_done = true;
                    let online_count = self.scanned_hosts.iter().filter(|h| h.online).count();
                    let offline_count = self.scanned_hosts.len().saturating_sub(online_count);
                    if cancelled {
                        self.set_banner(
                            format!("已中止掃描（找到 {online_count} 台在線）"),
                            false,
                        );
                    } else if offline_count > 0 {
                        self.set_banner(
                            format!(
                                "查詢完成：{online_count} 台在線，{offline_count} 台無回應"
                            ),
                            false,
                        );
                    } else {
                        self.set_banner(
                            format!("掃描完成，找到 {online_count} 台在線設備"),
                            false,
                        );
                    }
                }
            }
        }
        if scan_done {
            self.is_scanning = false;
            self.scan_rx = None;
            self.scan_cancel = None;
        }

        let mut need_save_last_seen = false;
        while let Ok(ev) = self.monitor_rx.try_recv() {
            match ev {
                MonitorEvent::Probing(active) => {
                    self.is_probing = active;
                }
                MonitorEvent::StateUpdated { device_id, runtime } => {
                    if let Some(seen) = &runtime.last_seen_text
                        && let Some(dev) =
                            self.config.devices.iter_mut().find(|d| d.id == device_id)
                        && dev.last_seen_text.as_deref() != Some(seen.as_str())
                    {
                        dev.last_seen_text = Some(seen.clone());
                        need_save_last_seen = true;
                    }
                    if matches!(runtime.status, PowerStatus::Online { .. }) {
                        self.pending_alerts.retain(|a| a.device_id != device_id);
                    }
                    self.runtimes.insert(device_id, runtime);
                }
                MonitorEvent::OfflineAlert(alert) => {
                    if !self
                        .pending_alerts
                        .iter()
                        .any(|a| a.device_id == alert.device_id)
                    {
                        self.pending_alerts.push(alert);
                    }
                }
            }
        }
        if need_save_last_seen {
            let _ = storage::save_config(&self.config);
        }

        while let Ok(ev) = self.tray_rx.try_recv() {
            match ev {
                TrayEvent::ShowWindow | TrayEvent::NotificationClicked => {}
                TrayEvent::WakeAllOffline => {
                    self.pending_alerts.clear();
                    self.set_banner("已送出全部關機設備的喚醒封包", false);
                }
                TrayEvent::CheckNow => {
                    self.set_banner("開始重新檢測", false);
                }
                TrayEvent::TogglePause => {
                    self.config.settings.monitor_paused = !self.config.settings.monitor_paused;
                    self.tray.set_paused(self.config.settings.monitor_paused);
                    let _ = storage::save_config(&self.config);
                    self.set_banner(
                        if self.config.settings.monitor_paused {
                            "已暫停監控"
                        } else {
                            "已恢復監控"
                        },
                        false,
                    );
                }
                TrayEvent::Exit => {
                    self.allow_exit.store(true, Ordering::SeqCst);
                }
            }
        }
    }

    fn render_top_command_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, pal: &Palette) {
        let frame = egui::Frame::NONE
            .fill(pal.canvas)
            .inner_margin(Margin {
                left: 18,
                right: 18,
                top: 10,
                bottom: 10,
            })
            .stroke(Stroke::new(1.0, pal.hairline));

        egui::Panel::top("instrument_top_bar")
            .frame(frame)
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // 品牌識別 + 狀態膠囊
                    ui.label(
                        RichText::new("WAKE-ON-LAN")
                            .size(15.5)
                            .strong()
                            .color(pal.ink_primary),
                    );
                    ui.add_space(8.0);

                    let monitored_count = self.config.devices.iter().filter(|d| d.monitored).count();
                    let online_count = self
                        .config
                        .devices
                        .iter()
                        .filter(|d| {
                            d.monitored
                                && self
                                    .runtimes
                                    .get(&d.id)
                                    .is_some_and(|r| matches!(r.status, PowerStatus::Online { .. }))
                        })
                        .count();

                    let (pill_text, pill_fg, pill_bg, dot_filled) = if monitored_count == 0 {
                        ("尚無監控設備".to_string(), pal.unknown, pal.unknown_bg, false)
                    } else if self.config.settings.monitor_paused {
                        ("已暫停監控".to_string(), pal.waking, pal.waking_bg, false)
                    } else if self.is_probing {
                        ("檢測中".to_string(), pal.info, pal.unknown_bg, true)
                    } else {
                        (
                            format!("監控中 · {online_count}/{monitored_count} 在線"),
                            pal.online,
                            pal.online_bg,
                            true,
                        )
                    };
                    status_pill(ui, &pill_text, pill_fg, pill_bg, dot_filled);

                    // 右側操作列（明確層級：1 顆狀態連動控制按鈕 + 2 顆低調 Ghost 按鈕）
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;

                        if styled_button(
                            ui,
                            pal,
                            ButtonStyle::Ghost,
                            Some(UiIcon::Settings),
                            "設定",
                            false,
                        )
                        .clicked()
                        {
                            self.show_settings_window = !self.show_settings_window;
                        }

                        if styled_button(
                            ui,
                            pal,
                            ButtonStyle::Ghost,
                            Some(UiIcon::MinimizeTray),
                            "系統匣",
                            false,
                        )
                        .on_hover_text("縮到系統匣在背景執行")
                        .clicked()
                        {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                        }

                        // 單一狀態連動監控控制鈕
                        if self.config.settings.monitor_paused {
                            if styled_button(
                                ui,
                                pal,
                                ButtonStyle::Primary,
                                Some(UiIcon::Play),
                                "開始監控",
                                false,
                            )
                            .on_hover_text("繼續監控並立刻檢測一次")
                            .clicked()
                            {
                                self.config.settings.monitor_paused = false;
                                self.persist_and_sync();
                                self.monitor.trigger_check_now();
                                self.set_banner("已開始監控", false);
                            }
                        } else if styled_button(
                            ui,
                            pal,
                            ButtonStyle::Secondary,
                            Some(UiIcon::Pause),
                            "暫停監控",
                            false,
                        )
                        .on_hover_text("暫停定時開關機檢測")
                        .clicked()
                        {
                            self.config.settings.monitor_paused = true;
                            self.persist_and_sync();
                            self.set_banner("已暫停監控", false);
                        }
                    });
                });
            });
    }

    fn render_left_rail(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        let rail_frame = egui::Frame::NONE
            .fill(pal.sidebar)
            .inner_margin(Margin {
                left: 14,
                right: 14,
                top: 14,
                bottom: 12,
            })
            .stroke(Stroke::new(1.0, pal.hairline));

        egui::Panel::left("subnet_sidebar")
            .resizable(false)
            .exact_size(236.0)
            .frame(rail_frame)
            .show_separator_line(false)
            .show(ui, |ui| {
                // 區塊 A：已儲存網段
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("已儲存網段")
                            .size(11.5)
                            .strong()
                            .color(pal.ink_secondary),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("{}", self.config.saved_subnets.len()))
                                .size(11.0)
                                .color(pal.ink_muted),
                        );
                    });
                });
                ui.add_space(6.0);

                if self.config.saved_subnets.is_empty() {
                    ui.label(
                        RichText::new("尚無儲存網段\n可於右側輸入後點「儲存」")
                            .size(11.5)
                            .color(pal.ink_muted),
                    );
                } else {
                    let mut select_cidr: Option<String> = None;
                    let mut set_default_id: Option<String> = None;
                    let mut delete_id: Option<String> = None;

                    for item in &self.config.saved_subnets {
                        let is_active = self.subnet_input.trim() == item.cidr;
                        let row_w = ui.available_width();
                        let row_h = if item.label != item.cidr { 44.0 } else { 32.0 };
                        let (row_rect, row_resp) =
                            ui.allocate_exact_size(vec2(row_w, row_h), egui::Sense::hover());

                        let bg = if is_active {
                            pal.surface
                        } else if row_resp.hovered() {
                            pal.surface_hover
                        } else {
                            Color32::TRANSPARENT
                        };
                        let border = if is_active {
                            Stroke::new(1.0, pal.hairline)
                        } else {
                            Stroke::NONE
                        };
                        ui.painter()
                            .rect(row_rect, 6.0, bg, border, egui::StrokeKind::Inside);

                        if is_active {
                            ui.painter().line_segment(
                                [
                                    pos2(row_rect.left() + 2.0, row_rect.top() + 5.0),
                                    pos2(row_rect.left() + 2.0, row_rect.bottom() - 5.0),
                                ],
                                Stroke::new(2.5, pal.accent),
                            );
                        }

                        // 左側文字點擊區（預留右側 56px 給星標與刪除按鈕）
                        let btns_w = if item.is_default { 30.0 } else { 58.0 };
                        let click_rect = egui::Rect::from_min_max(
                            pos2(row_rect.left() + 8.0, row_rect.top()),
                            pos2((row_rect.right() - btns_w).max(row_rect.left() + 40.0), row_rect.bottom()),
                        );
                        let click_resp = ui
                            .interact(
                                click_rect,
                                ui.scope_id().with(("saved_subnet", &item.id)),
                                egui::Sense::click(),
                            )
                            .on_hover_text(format!("{} ({})\n點此帶入查詢欄", item.label, item.cidr));
                        if click_resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if click_resp.clicked() {
                            select_cidr = Some(item.cidr.clone());
                        }

                        // 繪製標題（單行截斷）+ CIDR
                        let mut text_x = click_rect.left();
                        let line1_y = if item.label != item.cidr {
                            row_rect.top() + 6.0
                        } else {
                            row_rect.center().y - 7.0
                        };
                        if item.is_default {
                            let star_rect = egui::Rect::from_center_size(
                                pos2(text_x + 5.5, line1_y + 7.0),
                                vec2(11.0, 11.0),
                            );
                            paint_icon(ui.painter(), star_rect, UiIcon::StarFilled, pal.accent);
                            text_x += 15.0;
                        }
                        let g_label = layout_single_line_ellipsis(
                            ui.painter(),
                            item.label.clone(),
                            egui::FontId::proportional(12.5),
                            if is_active {
                                pal.ink_primary
                            } else {
                                pal.ink_secondary
                            },
                            (click_rect.right() - text_x - 4.0).max(20.0),
                        );
                        ui.painter().galley(
                            pos2(text_x, line1_y),
                            g_label,
                            pal.ink_primary,
                        );
                        if item.label != item.cidr {
                            let g_cidr = layout_single_line_ellipsis(
                                ui.painter(),
                                item.cidr.clone(),
                                egui::FontId::monospace(11.0),
                                if is_active { pal.accent } else { pal.ink_muted },
                                (click_rect.width() - 4.0).max(20.0),
                            );
                            ui.painter().galley(
                                pos2(click_rect.left(), row_rect.top() + 24.0),
                                g_cidr,
                                pal.ink_muted,
                            );
                        }

                        // 右側操作按鈕（垂直置中）
                        let btn_rect = egui::Rect::from_min_max(
                            pos2(click_rect.right(), row_rect.top()),
                            pos2(row_rect.right() - 4.0, row_rect.bottom()),
                        );
                        ui.scope_builder(
                            egui::UiBuilder::new()
                                .max_rect(btn_rect)
                                .layout(egui::Layout::right_to_left(egui::Align::Center)),
                            |ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                if icon_only_ghost(ui, pal, UiIcon::Close, true)
                                    .on_hover_text("刪除網段")
                                    .clicked()
                                {
                                    delete_id = Some(item.id.clone());
                                }
                                if !item.is_default
                                    && icon_only_ghost(ui, pal, UiIcon::StarFilled, false)
                                        .on_hover_text("設為預設")
                                        .clicked()
                                {
                                    set_default_id = Some(item.id.clone());
                                }
                            },
                        );

                        ui.add_space(3.0);
                    }

                    if let Some(cidr) = select_cidr {
                        self.subnet_input = cidr;
                        self.persist_and_sync();
                    }
                    if let Some(id) = set_default_id {
                        for s in &mut self.config.saved_subnets {
                            s.is_default = s.id == id;
                        }
                        self.persist_and_sync();
                    }
                    if let Some(id) = delete_id {
                        self.config.saved_subnets.retain(|s| s.id != id);
                        self.persist_and_sync();
                    }
                }

                ui.add_space(12.0);
                hairline_h(ui, pal);
                ui.add_space(12.0);

                // 區塊 B：本機網卡（Windows 原生 GetAdaptersAddresses）
                ui.horizontal(|ui| {
                    let (ir, _) = ui.allocate_exact_size(vec2(12.0, 12.0), egui::Sense::hover());
                    paint_icon(ui.painter(), ir, UiIcon::Network, pal.ink_secondary);
                    ui.label(
                        RichText::new("本機網卡")
                            .size(11.5)
                            .strong()
                            .color(pal.ink_secondary),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_only_ghost(ui, pal, UiIcon::Refresh, false)
                            .on_hover_text("重新偵測本機網卡")
                            .clicked()
                        {
                            self.local_subnets = detect_local_subnets();
                        }
                    });
                });
                ui.add_space(4.0);

                egui::ScrollArea::vertical()
                    .id_salt("local_nics_scroll")
                    .max_height((ui.available_height() - 48.0).max(80.0))
                    .show(ui, |ui| {
                        if self.local_subnets.is_empty() {
                            ui.label(
                                RichText::new("未偵測到啟用中的 IPv4 網卡")
                                    .size(11.5)
                                    .color(pal.ink_muted),
                            );
                        } else {
                            let subnets = self.local_subnets.clone();
                            for s in subnets {
                                let pref_cidr = s.preferred_scan_cidr();
                                self.render_nic_item(
                                    ui,
                                    pal,
                                    &s.adapter_name,
                                    &pref_cidr,
                                    s.is_virtual,
                                    if s.prefix_len < 24 {
                                        Some("本機 /24")
                                    } else {
                                        None
                                    },
                                );
                                if s.prefix_len < 24
                                    && parse_subnet_spec(&s.cidr)
                                        .is_ok_and(|t| t.hosts.len() <= MAX_SCAN_HOSTS)
                                {
                                    self.render_nic_item(
                                        ui,
                                        pal,
                                        &s.adapter_name,
                                        &s.cidr,
                                        s.is_virtual,
                                        Some("完整網段"),
                                    );
                                }
                            }
                        }
                    });

                // 底部遙測摘要（固定於側欄最下方）
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.label(
                        RichText::new(format!(
                            "每 {}s 偵測 · 連失 {} 次判定關機",
                            self.config.settings.poll_interval_secs,
                            self.config.settings.offline_threshold
                        ))
                        .size(11.0)
                        .color(pal.ink_muted),
                    );
                    ui.add_space(4.0);
                    hairline_h(ui, pal);
                });
            });
    }

    fn render_nic_item(
        &mut self,
        ui: &mut egui::Ui,
        pal: &Palette,
        adapter_name: &str,
        cidr: &str,
        is_virtual: bool,
        badge: Option<&str>,
    ) {
        let selected = self.subnet_input.trim() == cidr;
        let desired = vec2(ui.available_width(), 42.0);
        let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click());
        let line1 = match (is_virtual, badge) {
            (true, Some(b)) => format!("[虛擬] {adapter_name} · {b}"),
            (true, None) => format!("[虛擬] {adapter_name}"),
            (false, Some(b)) => format!("{adapter_name} · {b}"),
            (false, None) => adapter_name.to_string(),
        };

        if ui.is_rect_visible(rect) {
            let bg = if selected {
                pal.surface
            } else if resp.hovered() {
                pal.surface_hover
            } else {
                Color32::TRANSPARENT
            };
            let border = if selected {
                Stroke::new(1.0, pal.hairline)
            } else {
                Stroke::NONE
            };
            ui.painter()
                .rect(rect, 5.0, bg, border, egui::StrokeKind::Inside);

            if selected {
                ui.painter().line_segment(
                    [
                        pos2(rect.left() + 2.0, rect.top() + 6.0),
                        pos2(rect.left() + 2.0, rect.bottom() - 6.0),
                    ],
                    Stroke::new(2.5, pal.accent),
                );
            }

            let max_text_w = (rect.width() - 18.0).max(40.0);
            let g1 = layout_single_line_ellipsis(
                ui.painter(),
                line1.clone(),
                egui::FontId::proportional(11.5),
                if selected {
                    pal.ink_primary
                } else {
                    pal.ink_secondary
                },
                max_text_w,
            );
            let g2 = layout_single_line_ellipsis(
                ui.painter(),
                cidr.to_string(),
                egui::FontId::monospace(11.5),
                if selected { pal.accent } else { pal.ink_muted },
                max_text_w,
            );
            ui.painter()
                .galley(pos2(rect.left() + 9.0, rect.top() + 6.0), g1, pal.ink_primary);
            ui.painter()
                .galley(pos2(rect.left() + 9.0, rect.top() + 23.0), g2, pal.ink_muted);
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp
            .on_hover_text(format!("{line1}\n{cidr}\n點此帶入查詢欄"))
            .clicked()
        {
            self.subnet_input = cidr.to_string();
            self.persist_and_sync();
        }
        ui.add_space(2.0);
    }

    /// 內嵌高醒目關機提醒橫幅（不阻擋主工作區操作，同時一眼可見並可一鍵喚醒）
    fn render_offline_alert_banner(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        if self.pending_alerts.is_empty() {
            return;
        }

        let alerts_snapshot = self.pending_alerts.clone();
        let mut wake_ids = Vec::new();
        let mut dismiss_ids = Vec::new();
        let mut unmonitor_ids = Vec::new();

        egui::Frame::NONE
            .fill(pal.offline_bg)
            .stroke(Stroke::new(1.0, pal.offline))
            .corner_radius(7.0)
            .inner_margin(Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (ir, _) = ui.allocate_exact_size(vec2(16.0, 16.0), egui::Sense::hover());
                    paint_icon(ui.painter(), ir, UiIcon::Warning, pal.offline);
                    ui.label(
                        RichText::new(format!(
                            "{} 台設備已關機，要現在開機嗎？",
                            alerts_snapshot.len()
                        ))
                        .size(13.0)
                        .strong()
                        .color(pal.offline),
                    );
                    if alerts_snapshot.len() > 1 {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if styled_button(
                                ui,
                                pal,
                                ButtonStyle::Ghost,
                                None,
                                "全部略過",
                                true,
                            )
                            .clicked()
                            {
                                dismiss_ids
                                    .extend(alerts_snapshot.iter().map(|a| a.device_id.clone()));
                            }
                            if styled_button(
                                ui,
                                pal,
                                ButtonStyle::Primary,
                                Some(UiIcon::Power),
                                "全部開機",
                                true,
                            )
                            .clicked()
                            {
                                wake_ids
                                    .extend(alerts_snapshot.iter().map(|a| a.device_id.clone()));
                            }
                        });
                    }
                });

                ui.add_space(6.0);
                for alert in &alerts_snapshot {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(&alert.device_name)
                                .size(12.5)
                                .strong()
                                .color(pal.ink_primary),
                        );
                        ui.label(
                            RichText::new(format!(
                                "{}  ·  {}  ·  {}",
                                alert.ip, alert.mac, alert.detected_at_text
                            ))
                            .size(11.5)
                            .monospace()
                            .color(pal.ink_secondary),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            if styled_button(
                                ui,
                                pal,
                                ButtonStyle::Ghost,
                                None,
                                "停用監控",
                                true,
                            )
                            .clicked()
                            {
                                unmonitor_ids.push(alert.device_id.clone());
                            }
                            if styled_button(
                                ui,
                                pal,
                                ButtonStyle::Secondary,
                                None,
                                "略過本次",
                                true,
                            )
                            .clicked()
                            {
                                dismiss_ids.push(alert.device_id.clone());
                            }
                            if styled_button(
                                ui,
                                pal,
                                ButtonStyle::Primary,
                                Some(UiIcon::Power),
                                "立即開機",
                                true,
                            )
                            .clicked()
                            {
                                wake_ids.push(alert.device_id.clone());
                            }
                        });
                    });
                }
            });

        ui.add_space(14.0);

        for id in wake_ids {
            self.wake_device_by_id(&id);
        }
        for id in dismiss_ids {
            self.pending_alerts.retain(|a| a.device_id != id);
        }
        if !unmonitor_ids.is_empty() {
            for id in &unmonitor_ids {
                if let Some(dev) = self.config.devices.iter_mut().find(|d| &d.id == id) {
                    dev.monitored = false;
                }
                self.pending_alerts.retain(|a| &a.device_id != id);
            }
            self.persist_and_sync();
        }
    }

    fn render_discovery_band(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, pal: &Palette) {
        let parsed_preview = parse_subnet_spec(&self.subnet_input);
        let is_specific_query = parsed_preview.as_ref().is_ok_and(|t| t.report_offline());

        ui.horizontal(|ui| {
            ui.label(
                RichText::new("掃描與查詢")
                    .size(13.5)
                    .strong()
                    .color(pal.ink_primary),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new("可輸入網段、區間或特定 IP")
                    .size(11.5)
                    .color(pal.ink_muted),
            );
        });
        ui.add_space(8.0);

        // 主命令輸入列
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

            let input_w = (ui.available_width() - 220.0).clamp(180.0, 340.0);
            let resp = ui.add_sized(
                vec2(input_w, 30.0),
                egui::TextEdit::singleline(&mut self.subnet_input)
                    .margin(Margin::symmetric(8, 6))
                    .font(egui::FontId::monospace(13.0))
                    .hint_text("192.168.2.50 或 192.168.2.0/24"),
            );
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.start_manual_scan(ctx);
            }

            if self.is_scanning {
                if styled_button(ui, pal, ButtonStyle::Danger, Some(UiIcon::Stop), "中止掃描", false)
                    .clicked()
                {
                    self.cancel_scan();
                }
            } else {
                let btn_text = if is_specific_query {
                    "查詢特定 IP"
                } else {
                    "開始掃描"
                };
                if styled_button(
                    ui,
                    pal,
                    ButtonStyle::Primary,
                    Some(UiIcon::Search),
                    btn_text,
                    false,
                )
                .clicked()
                {
                    self.start_manual_scan(ctx);
                }
            }

            if styled_button(
                ui,
                pal,
                ButtonStyle::Secondary,
                Some(UiIcon::Save),
                "儲存網段",
                false,
            )
            .clicked()
            {
                self.show_save_subnet_popover = !self.show_save_subnet_popover;
            }
        });

        // 展開式儲存網段別名列（漸進揭露，平時不佔用主列空間）
        if self.show_save_subnet_popover {
            ui.add_space(6.0);
            egui::Frame::NONE
                .fill(pal.surface)
                .stroke(Stroke::new(1.0, pal.hairline))
                .corner_radius(6.0)
                .inner_margin(Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("網段別名：").size(12.0).color(pal.ink_secondary));
                        let alias_resp = ui.add_sized(
                            vec2(170.0, 24.0),
                            egui::TextEdit::singleline(&mut self.subnet_label_input)
                                .hint_text("如：辦公室 / Home（選填）"),
                        );
                        if (alias_resp.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                            || styled_button(ui, pal, ButtonStyle::Primary, None, "確認儲存", true)
                                .clicked()
                        {
                            self.save_current_subnet();
                        }
                        if styled_button(ui, pal, ButtonStyle::Ghost, None, "取消", true).clicked() {
                            self.show_save_subnet_popover = false;
                        }
                    });
                });
        }

        // 即時解析預覽（純本地計算，不發送任何網路封包）
        ui.add_space(5.0);
        match &parsed_preview {
            Ok(target) => {
                let preview_text = match target.mode {
                    QueryMode::SingleIp => {
                        format!("查詢 {}（單一 IP）", target.normalized)
                    }
                    QueryMode::SpecificIps => format!(
                        "查詢 {} 個指定 IP：{}",
                        target.hosts.len(),
                        target.normalized
                    ),
                    QueryMode::Range => format!(
                        "掃描 {}（{} 個 IP）",
                        target.normalized,
                        target.hosts.len()
                    ),
                    QueryMode::Subnet => {
                        if let Some(orig) = target.auto_narrowed_from {
                            format!(
                                "自動縮減為 {}（{} 個 IP，原 /{orig}）",
                                target.normalized,
                                target.hosts.len()
                            )
                        } else {
                            format!(
                                "掃描 {}（{} 個 IP）",
                                target.normalized,
                                target.hosts.len()
                            )
                        }
                    }
                };
                let color = if target.auto_narrowed_from.is_some() {
                    pal.waking
                } else {
                    pal.ink_muted
                };
                ui.label(RichText::new(preview_text).size(11.5).color(color));
            }
            Err(err) if !self.subnet_input.trim().is_empty() => {
                ui.label(RichText::new(err).size(11.5).color(pal.offline));
            }
            _ => {}
        }

        // 精簡掃描進度條
        if self.is_scanning {
            let (done, total) = self.scan_progress;
            let progress = if total == 0 {
                0.0
            } else {
                done as f32 / total as f32
            };
            ui.add_space(6.0);
            let bar_w = ui.available_width();
            let (rect, _) = ui.allocate_exact_size(vec2(bar_w, 4.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 2.0, pal.hairline);
            let fill_rect = egui::Rect::from_min_size(rect.min, vec2(bar_w * progress, 4.0));
            ui.painter().rect_filled(fill_rect, 2.0, pal.accent);
            ui.label(
                RichText::new(format!("掃描進度：{done} / {total}"))
                    .size(11.0)
                    .color(pal.ink_secondary),
            );
        }

        // 掃描／查詢結果表（僅在有結果時顯示）
        if !self.scanned_hosts.is_empty() {
            ui.add_space(10.0);
            let online_found = self.scanned_hosts.iter().filter(|h| h.online).count();
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!(
                        "查詢結果 · {} 筆（{} 台在線）",
                        self.scanned_hosts.len(),
                        online_found
                    ))
                    .size(12.0)
                    .strong()
                    .color(pal.ink_secondary),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if styled_button(ui, pal, ButtonStyle::Ghost, None, "清除結果", true).clicked()
                    {
                        self.scanned_hosts.clear();
                    }
                });
            });
            ui.add_space(4.0);

            let saved_by_ip: HashMap<String, Device> = self
                .config
                .devices
                .iter()
                .map(|d| (d.ip.clone(), d.clone()))
                .collect();
            let saved_macs: HashSet<String> = self
                .config
                .devices
                .iter()
                .map(|d| d.mac.to_ascii_uppercase())
                .collect();

            let mut to_add: Option<ScannedHost> = None;
            let mut quick_wake: Option<ScannedHost> = None;
            let mut wake_saved_id: Option<String> = None;
            let mut manual_add_ip: Option<(Ipv4Addr, Ipv4Addr)> = None;

            egui::Frame::NONE
                .fill(pal.surface)
                .stroke(Stroke::new(1.0, pal.hairline))
                .corner_radius(6.0)
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    let table_w = ui.available_width();
                    let w_ip = 132.0;
                    let w_status = 112.0;
                    let w_ops = 166.0;
                    let w_mac = (table_w - w_ip - w_status - w_ops).max(140.0);

                    // 表頭列
                    let (hdr_rect, _) =
                        ui.allocate_exact_size(vec2(table_w, 22.0), egui::Sense::hover());
                    let cols_x = [
                        hdr_rect.left(),
                        hdr_rect.left() + w_ip,
                        hdr_rect.left() + w_ip + w_mac,
                        hdr_rect.left() + w_ip + w_mac + w_status,
                    ];
                    let hdrs = ["IP 位址", "MAC 位址", "狀態", "操作"];
                    for (idx, title) in hdrs.iter().enumerate() {
                        let g = ui.painter().layout_no_wrap(
                            (*title).to_string(),
                            egui::FontId::proportional(11.5),
                            pal.ink_muted,
                        );
                        ui.painter().galley(
                            pos2(cols_x[idx], hdr_rect.center().y - g.size().y * 0.5),
                            g,
                            pal.ink_muted,
                        );
                    }
                    ui.painter().line_segment(
                        [
                            pos2(hdr_rect.left(), hdr_rect.bottom()),
                            pos2(hdr_rect.right(), hdr_rect.bottom()),
                        ],
                        Stroke::new(1.0, pal.hairline),
                    );

                    egui::ScrollArea::vertical()
                        .id_salt("scan_results_scroll")
                        .max_height(148.0)
                        .show(ui, |ui| {
                            for host in &self.scanned_hosts {
                                let ip_str = host.ip.to_string();
                                let saved_dev = saved_by_ip.get(&ip_str);
                                let already = saved_dev.is_some()
                                    || host.mac.as_ref().is_some_and(|m| {
                                        saved_macs.contains(&m.to_ascii_uppercase())
                                    });

                                let mac_str = host
                                    .mac
                                    .clone()
                                    .or_else(|| saved_dev.map(|d| format!("{} (已存)", d.mac)))
                                    .unwrap_or_else(|| {
                                        if host.online {
                                            "非同 L2 區網".to_string()
                                        } else {
                                            "—".to_string()
                                        }
                                    });

                                let (row_rect, row_resp) = ui.allocate_exact_size(
                                    vec2(table_w, 34.0),
                                    egui::Sense::hover(),
                                );
                                if row_resp.hovered() {
                                    ui.painter().rect_filled(row_rect, 4.0, pal.surface_hover);
                                }
                                ui.painter().line_segment(
                                    [
                                        pos2(row_rect.left(), row_rect.bottom()),
                                        pos2(row_rect.right(), row_rect.bottom()),
                                    ],
                                    Stroke::new(1.0, pal.hairline),
                                );

                                // IP
                                let g_ip = layout_single_line_ellipsis(
                                    ui.painter(),
                                    ip_str,
                                    egui::FontId::monospace(12.5),
                                    pal.ink_primary,
                                    w_ip - 8.0,
                                );
                                ui.painter().galley(
                                    pos2(cols_x[0], row_rect.center().y - g_ip.size().y * 0.5),
                                    g_ip,
                                    pal.ink_primary,
                                );

                                // MAC
                                let g_mac = layout_single_line_ellipsis(
                                    ui.painter(),
                                    mac_str,
                                    egui::FontId::monospace(12.0),
                                    pal.ink_secondary,
                                    w_mac - 10.0,
                                );
                                ui.painter().galley(
                                    pos2(cols_x[1], row_rect.center().y - g_mac.size().y * 0.5),
                                    g_mac,
                                    pal.ink_secondary,
                                );

                                // 狀態膠囊
                                let status_rect = egui::Rect::from_min_max(
                                    pos2(cols_x[2], row_rect.top()),
                                    pos2(cols_x[3] - 6.0, row_rect.bottom()),
                                );
                                ui.scope_builder(
                                    egui::UiBuilder::new()
                                        .max_rect(status_rect)
                                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                                    |ui| {
                                        if host.online {
                                            let txt = host
                                                .rtt_ms
                                                .map(|ms| format!("在線 · {ms}ms"))
                                                .unwrap_or_else(|| "在線 · ARP".to_string());
                                            status_pill(ui, &txt, pal.online, pal.online_bg, true);
                                        } else {
                                            status_pill(
                                                ui,
                                                "無回應",
                                                pal.offline,
                                                pal.offline_bg,
                                                true,
                                            );
                                        }
                                    },
                                );

                                // 操作按鈕
                                let ops_rect = egui::Rect::from_min_max(
                                    pos2(cols_x[3], row_rect.top()),
                                    pos2(row_rect.right(), row_rect.bottom()),
                                );
                                ui.scope_builder(
                                    egui::UiBuilder::new()
                                        .max_rect(ops_rect)
                                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                                    |ui| {
                                        ui.spacing_mut().item_spacing.x = 6.0;
                                        if host.online {
                                            if already {
                                                ui.label(
                                                    RichText::new("已在清單")
                                                        .size(11.5)
                                                        .color(pal.ink_muted),
                                                );
                                            } else if styled_button(
                                                ui,
                                                pal,
                                                ButtonStyle::Secondary,
                                                Some(UiIcon::Plus),
                                                "加入監控",
                                                true,
                                            )
                                            .clicked()
                                            {
                                                to_add = Some(host.clone());
                                            }
                                            if host.mac.is_some()
                                                && styled_button(
                                                    ui,
                                                    pal,
                                                    ButtonStyle::Ghost,
                                                    Some(UiIcon::Power),
                                                    "喚醒",
                                                    true,
                                                )
                                                .clicked()
                                            {
                                                quick_wake = Some(host.clone());
                                            }
                                        } else if let Some(dev) = saved_dev {
                                            if styled_button(
                                                ui,
                                                pal,
                                                ButtonStyle::Primary,
                                                Some(UiIcon::Power),
                                                "立即開機",
                                                true,
                                            )
                                            .clicked()
                                            {
                                                wake_saved_id = Some(dev.id.clone());
                                            }
                                        } else if styled_button(
                                            ui,
                                            pal,
                                            ButtonStyle::Secondary,
                                            Some(UiIcon::Plus),
                                            "手動填 MAC",
                                            true,
                                        )
                                        .clicked()
                                        {
                                            manual_add_ip = Some((host.ip, host.broadcast_ip));
                                        }
                                    },
                                );
                            }
                        });
                });

            if let Some(host) = to_add {
                self.add_scanned_host_to_devices(&host);
            }
            if let Some(id) = wake_saved_id {
                self.wake_device_by_id(&id);
            }
            if let Some((ip, bcast)) = manual_add_ip {
                let mut form = DeviceForm::new_empty(&bcast.to_string());
                form.ip = ip.to_string();
                form.name = format!("主機 {ip}");
                self.device_form = Some(form);
            }
            if let Some(host) = quick_wake
                && let Some(mac_str) = host.mac
                && let Ok(mac) = parse_mac(&mac_str)
            {
                match send_wol(mac, Some(host.broadcast_ip), 9) {
                    Ok(t) => self.set_banner(
                        format!("已發送測試喚醒封包給 {} → {}", host.ip, t.join(", ")),
                        false,
                    ),
                    Err(e) => self.set_banner(format!("測試喚醒失敗：{e}"), true),
                }
            }
        }
    }

    fn render_devices_band(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, pal: &Palette) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("受監控設備")
                    .size(13.5)
                    .strong()
                    .color(pal.ink_primary),
            );
            ui.label(
                RichText::new(format!("· {} 台", self.config.devices.len()))
                    .size(12.5)
                    .color(pal.ink_muted),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;

                if styled_button(
                    ui,
                    pal,
                    ButtonStyle::Secondary,
                    Some(UiIcon::Plus),
                    "新增設備",
                    true,
                )
                .clicked()
                {
                    let bcast = self.default_broadcast_hint();
                    self.device_form = Some(DeviceForm::new_empty(&bcast));
                }

                let has_monitored = self.config.devices.iter().any(|d| d.monitored);
                if has_monitored {
                    let label = if self.is_probing {
                        "檢測中..."
                    } else {
                        "重新檢測"
                    };
                    let resp = ui
                        .add_enabled_ui(!self.is_probing, |ui| {
                            styled_button(
                                ui,
                                pal,
                                ButtonStyle::Ghost,
                                Some(UiIcon::Refresh),
                                label,
                                true,
                            )
                        })
                        .inner;
                    if resp.on_hover_text("立刻重新檢測一次").clicked() {
                        self.monitor.trigger_check_now();
                        self.set_banner("檢測中...", false);
                    }
                }

                let has_offline = self.config.devices.iter().any(|d| {
                    self.runtimes
                        .get(&d.id)
                        .is_some_and(|r| matches!(r.status, PowerStatus::Offline))
                });
                if has_offline
                    && styled_button(
                        ui,
                        pal,
                        ButtonStyle::Primary,
                        Some(UiIcon::Power),
                        "全部開機",
                        true,
                    )
                    .clicked()
                {
                    let ids: Vec<String> = self
                        .config
                        .devices
                        .iter()
                        .filter(|d| {
                            self.runtimes
                                .get(&d.id)
                                .is_some_and(|r| matches!(r.status, PowerStatus::Offline))
                        })
                        .map(|d| d.id.clone())
                        .collect();
                    for id in ids {
                        self.wake_device_by_id(&id);
                    }
                }
            });
        });

        ui.add_space(10.0);

        if self.config.devices.is_empty() {
            // 引導型空白狀態
            egui::Frame::NONE
                .fill(pal.surface)
                .stroke(Stroke::new(1.0, pal.hairline))
                .corner_radius(7.0)
                .inner_margin(Margin::symmetric(20, 22))
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("清單是空的")
                                .size(13.5)
                                .strong()
                                .color(pal.ink_primary),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(
                                "關機中的電腦不會回應掃描。請先掃描開機中的設備加入清單，或直接手動輸入 IP 與 MAC。",
                            )
                            .size(12.0)
                            .color(pal.ink_secondary),
                        );
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if styled_button(
                                ui,
                                pal,
                                ButtonStyle::Primary,
                                Some(UiIcon::Search),
                                "掃描目前網段",
                                true,
                            )
                            .clicked()
                            {
                                self.start_manual_scan(ctx);
                            }
                            if styled_button(
                                ui,
                                pal,
                                ButtonStyle::Secondary,
                                Some(UiIcon::Plus),
                                "手動新增設備",
                                true,
                            )
                            .clicked()
                            {
                                let bcast = self.default_broadcast_hint();
                                self.device_form = Some(DeviceForm::new_empty(&bcast));
                            }
                        });
                    });
                });
            return;
        }

        let mut wake_id: Option<String> = None;
        let mut edit_dev: Option<Device> = None;
        let mut delete_id: Option<String> = None;
        let mut monitor_toggled = false;
        let mut reset_alert_id: Option<String> = None;

        // 滿版精確欄寬表格（每列 48px、全欄垂直置中、強制單行不折行、1px 細底線分隔）
        let total_w = ui.available_width();
        let w_chk = 42.0;
        let w_status = 114.0;
        let w_mac = 146.0;
        let w_time = 104.0;
        let w_ops = 136.0;
        let w_name = (total_w - w_chk - w_status - w_mac - w_time - w_ops).max(136.0);

        let col_x = [
            0.0,
            w_chk,
            w_chk + w_status,
            w_chk + w_status + w_name,
            w_chk + w_status + w_name + w_mac,
            w_chk + w_status + w_name + w_mac + w_time,
        ];

        // 固定表頭列
        let (hdr_rect, _) = ui.allocate_exact_size(vec2(total_w, 26.0), egui::Sense::hover());
        let base_x = hdr_rect.left();
        let headers = ["監控", "電源狀態", "設備 / IP", "MAC 位址", "最後檢測", "操作"];
        for (i, title) in headers.iter().enumerate() {
            let g = ui.painter().layout_no_wrap(
                (*title).to_string(),
                egui::FontId::proportional(11.5),
                pal.ink_muted,
            );
            let tx = if i == 5 {
                hdr_rect.right() - g.size().x - 4.0
            } else {
                base_x + col_x[i] + 2.0
            };
            ui.painter()
                .galley(pos2(tx, hdr_rect.center().y - g.size().y * 0.5), g, pal.ink_muted);
        }
        ui.painter().line_segment(
            [
                pos2(hdr_rect.left(), hdr_rect.bottom()),
                pos2(hdr_rect.right(), hdr_rect.bottom()),
            ],
            Stroke::new(1.0, pal.hairline),
        );

        egui::ScrollArea::vertical()
            .id_salt("devices_scroll")
            .show(ui, |ui| {
                for dev in &mut self.config.devices {
                    let rt = self.runtimes.get(&dev.id);
                    let (row_rect, row_resp) =
                        ui.allocate_exact_size(vec2(total_w, 48.0), egui::Sense::hover());
                    let rx = row_rect.left();

                    if row_resp.hovered() {
                        ui.painter().rect_filled(row_rect, 5.0, pal.surface);
                    }
                    ui.painter().line_segment(
                        [
                            pos2(row_rect.left(), row_rect.bottom()),
                            pos2(row_rect.right(), row_rect.bottom()),
                        ],
                        Stroke::new(1.0, pal.hairline),
                    );

                    // Col 0: 監控 Checkbox（垂直置中）
                    let c0 = egui::Rect::from_min_max(
                        pos2(rx + col_x[0], row_rect.top()),
                        pos2(rx + col_x[1], row_rect.bottom()),
                    );
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(c0)
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        |ui| {
                            ui.add_space(4.0);
                            if ui.checkbox(&mut dev.monitored, "").changed() {
                                monitor_toggled = true;
                            }
                        },
                    );

                    // Col 1: 電源狀態膠囊（垂直置中）
                    let (pill_text, fg, bg, filled) = if !dev.monitored {
                        ("未監控".to_string(), pal.unknown, pal.unknown_bg, false)
                    } else {
                        match rt.map(|r| &r.status) {
                            Some(PowerStatus::Online { rtt_ms }) => (
                                match rtt_ms {
                                    Some(ms) => format!("在線 · {ms}ms"),
                                    None => "在線 · TCP".to_string(),
                                },
                                pal.online,
                                pal.online_bg,
                                true,
                            ),
                            Some(PowerStatus::Offline) => (
                                if rt.is_some_and(|r| r.wake_timed_out) {
                                    "已關機 · 逾時".to_string()
                                } else {
                                    "已關機".to_string()
                                },
                                pal.offline,
                                pal.offline_bg,
                                true,
                            ),
                            Some(PowerStatus::Waking { started_at }) => (
                                format!("喚醒中 · {}s", started_at.elapsed().as_secs()),
                                pal.waking,
                                pal.waking_bg,
                                true,
                            ),
                            _ => ("檢測中".to_string(), pal.unknown, pal.unknown_bg, false),
                        }
                    };
                    let c1 = egui::Rect::from_min_max(
                        pos2(rx + col_x[1], row_rect.top()),
                        pos2(rx + col_x[2] - 6.0, row_rect.bottom()),
                    );
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(c1)
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        |ui| {
                            status_pill(ui, &pill_text, fg, bg, filled);
                        },
                    );

                    // Col 2: 設備名稱 + IP（雙行單截斷，絕不折成多行）
                    let name_max_w = (w_name - 12.0).max(40.0);
                    let g_name = layout_single_line_ellipsis(
                        ui.painter(),
                        dev.name.clone(),
                        egui::FontId::proportional(13.0),
                        pal.ink_primary,
                        name_max_w,
                    );
                    let g_ip = layout_single_line_ellipsis(
                        ui.painter(),
                        dev.ip.clone(),
                        egui::FontId::monospace(11.5),
                        pal.ink_muted,
                        name_max_w,
                    );
                    let name_block_h = g_name.size().y + 2.0 + g_ip.size().y;
                    let name_top = row_rect.center().y - name_block_h * 0.5;
                    ui.painter().galley(
                        pos2(rx + col_x[2] + 2.0, name_top),
                        g_name.clone(),
                        pal.ink_primary,
                    );
                    ui.painter().galley(
                        pos2(rx + col_x[2] + 2.0, name_top + g_name.size().y + 2.0),
                        g_ip,
                        pal.ink_muted,
                    );

                    // Col 3: MAC 位址（單行垂直置中）
                    let g_mac = layout_single_line_ellipsis(
                        ui.painter(),
                        dev.mac.clone(),
                        egui::FontId::monospace(12.0),
                        pal.ink_secondary,
                        w_mac - 10.0,
                    );
                    ui.painter().galley(
                        pos2(rx + col_x[3] + 2.0, row_rect.center().y - g_mac.size().y * 0.5),
                        g_mac,
                        pal.ink_secondary,
                    );

                    // Col 4: 最後檢測 / 上線（雙行垂直置中，絕不折行）
                    let checked = rt
                        .and_then(|r| r.last_checked_text.as_deref())
                        .unwrap_or("—");
                    let seen = dev.last_seen_text.as_deref().unwrap_or("—");
                    let g_chk = layout_single_line_ellipsis(
                        ui.painter(),
                        checked.to_string(),
                        egui::FontId::monospace(11.5),
                        pal.ink_secondary,
                        w_time - 8.0,
                    );
                    let g_seen = layout_single_line_ellipsis(
                        ui.painter(),
                        format!("上線 {seen}"),
                        egui::FontId::proportional(10.5),
                        pal.ink_muted,
                        w_time - 8.0,
                    );
                    let time_block_h = g_chk.size().y + 2.0 + g_seen.size().y;
                    let time_top = row_rect.center().y - time_block_h * 0.5;
                    ui.painter().galley(
                        pos2(rx + col_x[4] + 2.0, time_top),
                        g_chk.clone(),
                        pal.ink_secondary,
                    );
                    ui.painter().galley(
                        pos2(rx + col_x[4] + 2.0, time_top + g_chk.size().y + 2.0),
                        g_seen,
                        pal.ink_muted,
                    );

                    // Col 5: 操作列（靠右垂直置中）
                    let c5 = egui::Rect::from_min_max(
                        pos2(rx + col_x[5], row_rect.top()),
                        pos2(row_rect.right(), row_rect.bottom()),
                    );
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(c5)
                            .layout(egui::Layout::right_to_left(egui::Align::Center)),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            if icon_only_ghost(ui, pal, UiIcon::Trash, true)
                                .on_hover_text("刪除設備")
                                .clicked()
                            {
                                delete_id = Some(dev.id.clone());
                            }
                            if rt.is_some_and(|r| r.notified_offline)
                                && icon_only_ghost(ui, pal, UiIcon::Refresh, false)
                                    .on_hover_text("清除已提醒紀錄，下次若仍關機再通知")
                                    .clicked()
                            {
                                reset_alert_id = Some(dev.id.clone());
                            }
                            if icon_only_ghost(ui, pal, UiIcon::Edit, false)
                                .on_hover_text("編輯")
                                .clicked()
                            {
                                edit_dev = Some(dev.clone());
                            }
                            let is_offline = rt
                                .is_some_and(|r| matches!(r.status, PowerStatus::Offline));
                            let wake_style = if is_offline {
                                ButtonStyle::Primary
                            } else {
                                ButtonStyle::Secondary
                            };
                            if styled_button(
                                ui,
                                pal,
                                wake_style,
                                Some(UiIcon::Power),
                                "開機",
                                true,
                            )
                            .on_hover_text("送出開機封包")
                            .clicked()
                            {
                                wake_id = Some(dev.id.clone());
                            }
                        },
                    );
                }
            });

        if monitor_toggled {
            self.persist_and_sync();
            self.monitor.trigger_check_now();
        }
        if let Some(id) = wake_id {
            self.wake_device_by_id(&id);
        }
        if let Some(dev) = edit_dev {
            self.device_form = Some(DeviceForm::from_device(&dev));
        }
        if let Some(id) = reset_alert_id {
            self.monitor.reset_offline_notification(&id);
            self.monitor.trigger_check_now();
        }
        if let Some(id) = delete_id {
            self.config.devices.retain(|d| d.id != id);
            self.runtimes.remove(&id);
            self.pending_alerts.retain(|a| a.device_id != id);
            self.persist_and_sync();
        }
    }

    fn render_device_form_modal(&mut self, ctx: &egui::Context, pal: &Palette) {
        let Some(mut form) = self.device_form.take() else {
            return;
        };

        let mut keep_open = true;
        let mut submitted = false;

        let title = if form.editing_id.is_some() {
            "編輯設備"
        } else {
            "手動新增設備"
        };

        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.add_space(4.0);
                egui::Grid::new("device_form_grid")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label(RichText::new("設備名稱").color(pal.ink_secondary));
                        ui.add_sized(
                            vec2(230.0, 26.0),
                            egui::TextEdit::singleline(&mut form.name)
                                .hint_text("例如：辦公室主機"),
                        );
                        ui.end_row();

                        ui.label(RichText::new("IP 位址").color(pal.ink_secondary));
                        ui.add_sized(
                            vec2(230.0, 26.0),
                            egui::TextEdit::singleline(&mut form.ip)
                                .font(egui::FontId::monospace(12.5))
                                .hint_text("192.168.2.50"),
                        );
                        ui.end_row();

                        ui.label(RichText::new("MAC 位址").color(pal.ink_secondary));
                        ui.add_sized(
                            vec2(230.0, 26.0),
                            egui::TextEdit::singleline(&mut form.mac)
                                .font(egui::FontId::monospace(12.5))
                                .hint_text("AA:BB:CC:DD:EE:FF"),
                        );
                        ui.end_row();

                        ui.label(RichText::new("廣播位址").color(pal.ink_secondary));
                        ui.add_sized(
                            vec2(230.0, 26.0),
                            egui::TextEdit::singleline(&mut form.broadcast_ip)
                                .font(egui::FontId::monospace(12.5))
                                .hint_text("留空自動依 IP/24 推算"),
                        );
                        ui.end_row();

                        ui.label(RichText::new("WoL UDP 埠").color(pal.ink_secondary));
                        ui.add_sized(
                            vec2(90.0, 26.0),
                            egui::TextEdit::singleline(&mut form.wol_port),
                        );
                        ui.end_row();

                        ui.label(RichText::new("TCP 輔助埠").color(pal.ink_secondary));
                        ui.add_sized(
                            vec2(140.0, 26.0),
                            egui::TextEdit::singleline(&mut form.tcp_probe_port)
                                .hint_text("選填，如 3389"),
                        );
                        ui.end_row();

                        ui.label(RichText::new("自動監控").color(pal.ink_secondary));
                        ui.checkbox(&mut form.monitored, "開啟自動監控，關機時提醒");
                        ui.end_row();
                    });

                if let Some(err) = &form.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(err).size(12.0).color(pal.offline));
                }

                ui.add_space(10.0);
                hairline_h(ui, pal);
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if styled_button(ui, pal, ButtonStyle::Primary, None, "儲存設備", false).clicked()
                    {
                        submitted = true;
                    }
                    if styled_button(ui, pal, ButtonStyle::Ghost, None, "取消", false).clicked() {
                        keep_open = false;
                    }
                });
            });

        if submitted {
            match self.validate_and_save_device(&form) {
                Ok(()) => keep_open = false,
                Err(e) => form.error = Some(e),
            }
        }

        if keep_open {
            self.device_form = Some(form);
        }
    }

    fn validate_and_save_device(&mut self, form: &DeviceForm) -> Result<(), String> {
        let ip: Ipv4Addr = form
            .ip
            .trim()
            .parse()
            .map_err(|_| "請輸入有效的 IPv4 位址".to_string())?;
        let mac_bytes = parse_mac(&form.mac)?;
        let mac_norm = format_mac(mac_bytes);
        let wol_port: u16 = form
            .wol_port
            .trim()
            .parse()
            .map_err(|_| "WoL 埠號必須為 1~65535 的數字".to_string())?;
        if wol_port == 0 {
            return Err("WoL 埠號不可為 0".to_string());
        }

        let tcp_probe_port = if form.tcp_probe_port.trim().is_empty() {
            None
        } else {
            let p: u16 = form
                .tcp_probe_port
                .trim()
                .parse()
                .map_err(|_| "TCP 輔助探測埠必須為有效埠號".to_string())?;
            if p == 0 {
                return Err("TCP 輔助探測埠不可為 0".to_string());
            }
            Some(p)
        };

        let broadcast_ip = if form.broadcast_ip.trim().is_empty() {
            ipv4_network_and_broadcast(ip, 24).1.to_string()
        } else {
            let b: Ipv4Addr = form
                .broadcast_ip
                .trim()
                .parse()
                .map_err(|_| "廣播位址格式無效".to_string())?;
            b.to_string()
        };

        let name = if form.name.trim().is_empty() {
            format!("主機 {ip}")
        } else {
            form.name.trim().to_string()
        };

        if let Some(edit_id) = &form.editing_id {
            if let Some(dev) = self.config.devices.iter_mut().find(|d| &d.id == edit_id) {
                dev.name = name;
                dev.ip = ip.to_string();
                dev.mac = mac_norm;
                dev.broadcast_ip = broadcast_ip;
                dev.wol_port = wol_port;
                dev.monitored = form.monitored;
                dev.tcp_probe_port = tcp_probe_port;
            }
            self.set_banner("已更新設備設定", false);
        } else {
            self.config.devices.push(Device {
                id: generate_id("dev"),
                name: name.clone(),
                ip: ip.to_string(),
                mac: mac_norm,
                broadcast_ip,
                wol_port,
                monitored: form.monitored,
                tcp_probe_port,
                last_seen_text: None,
            });
            self.set_banner(format!("已新增設備「{name}」"), false);
        }

        self.persist_and_sync();
        self.monitor.trigger_check_now();
        Ok(())
    }

    fn render_settings_window(&mut self, ctx: &egui::Context, pal: &Palette) {
        if !self.show_settings_window {
            return;
        }
        let mut open = self.show_settings_window;
        let mut changed = false;

        egui::Window::new("設定")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.label(
                    RichText::new("外觀主題")
                        .size(11.5)
                        .strong()
                        .color(pal.ink_secondary),
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if segment_choice(
                        ui,
                        pal,
                        self.config.settings.theme == ThemeMode::System,
                        UiIcon::Monitor,
                        "跟隨系統",
                    )
                    .clicked()
                    {
                        self.config.settings.theme = ThemeMode::System;
                        changed = true;
                    }
                    if segment_choice(
                        ui,
                        pal,
                        self.config.settings.theme == ThemeMode::Light,
                        UiIcon::Sun,
                        "淺色",
                    )
                    .clicked()
                    {
                        self.config.settings.theme = ThemeMode::Light;
                        changed = true;
                    }
                    if segment_choice(
                        ui,
                        pal,
                        self.config.settings.theme == ThemeMode::Dark,
                        UiIcon::Moon,
                        "暗黑",
                    )
                    .clicked()
                    {
                        self.config.settings.theme = ThemeMode::Dark;
                        changed = true;
                    }
                });

                ui.add_space(10.0);
                hairline_h(ui, pal);
                ui.add_space(10.0);

                ui.label(
                    RichText::new("監控頻率")
                        .size(11.5)
                        .strong()
                        .color(pal.ink_secondary),
                );
                ui.add_space(6.0);
                egui::Grid::new("settings_num_grid")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("檢測間隔（秒）");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.config.settings.poll_interval_secs)
                                    .range(5..=300),
                            )
                            .changed();
                        ui.end_row();

                        ui.label("連續無回應幾次判定關機");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.config.settings.offline_threshold)
                                    .range(1..=10),
                            )
                            .changed();
                        ui.end_row();
                    });

                ui.add_space(10.0);
                hairline_h(ui, pal);
                ui.add_space(10.0);

                changed |= ui
                    .checkbox(
                        &mut self.config.settings.minimize_to_tray_on_close,
                        "按關閉 (X) 時縮到系統匣，不結束程式",
                    )
                    .changed();
                ui.add_space(4.0);
                changed |= ui
                    .checkbox(
                        &mut self.config.settings.enable_desktop_notifications,
                        "設備關機時跳出系統通知",
                    )
                    .changed();

                ui.add_space(10.0);
                ui.label(
                    RichText::new(format!(
                        "設定檔：{}",
                        storage::default_config_path().display()
                    ))
                    .size(10.5)
                    .color(pal.ink_muted),
                );
            });

        self.show_settings_window = open;
        if changed {
            apply_theme_mode(ctx, self.config.settings.theme);
            self.titlebar_sync_passes = 3;
            self.persist_and_sync();
        }
    }

    fn render_footer_status(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        let Some((msg, is_err)) = &self.status_banner else {
            return;
        };
        let frame = egui::Frame::NONE
            .fill(pal.sidebar)
            .inner_margin(Margin::symmetric(16, 6))
            .stroke(Stroke::new(1.0, pal.hairline));
        egui::Panel::bottom("status_footer")
            .frame(frame)
            .show_separator_line(false)
            .show(ui, |ui| {
                let color = if *is_err { pal.offline } else { pal.ink_secondary };
                ui.label(RichText::new(msg).size(11.5).color(color));
            });
    }
}

impl eframe::App for WakeOnLanApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.drain_background_events();

        let is_dark = ctx.theme() == egui::Theme::Dark;
        if self.last_titlebar_dark != Some(is_dark) {
            self.last_titlebar_dark = Some(is_dark);
            self.titlebar_sync_passes = 3;
        }
        if self.titlebar_sync_passes > 0 {
            if let Ok(wh) = frame.window_handle()
                && let RawWindowHandle::Win32(h) = wh.as_raw()
            {
                apply_windows_titlebar_colors(h.hwnd.get(), is_dark);
            }
            self.titlebar_sync_passes -= 1;
            if self.titlebar_sync_passes > 0 {
                ctx.request_repaint();
            }
        }

        let pal = Palette::for_mode(is_dark);

        // 攔截關閉視窗事件：縮小至系統匣背景常駐
        if ctx.input(|i| i.viewport().close_requested())
            && self.config.settings.minimize_to_tray_on_close
            && !self.allow_exit.load(Ordering::SeqCst)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        self.render_top_command_bar(ui, &ctx, &pal);
        self.render_footer_status(ui, &pal);
        self.render_left_rail(ui, &pal);

        let central_frame = egui::Frame::NONE
            .fill(pal.canvas)
            .inner_margin(Margin {
                left: 22,
                right: 22,
                top: 16,
                bottom: 16,
            });
        egui::CentralPanel::default()
            .frame(central_frame)
            .show(ui, |ui| {
                self.render_offline_alert_banner(ui, &pal);
                self.render_discovery_band(ui, &ctx, &pal);
                ui.add_space(16.0);
                hairline_h(ui, &pal);
                ui.add_space(16.0);
                self.render_devices_band(ui, &ctx, &pal);
            });

        self.render_device_form_modal(&ctx, &pal);
        self.render_settings_window(&ctx, &pal);

        if self
            .runtimes
            .values()
            .any(|r| matches!(r.status, PowerStatus::Waking { .. }))
        {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
    }

    fn on_exit(&mut self) {
        self.cancel_scan();
        self.monitor.stop();
        self.tray.shutdown();
        let _ = storage::save_config(&self.config);
    }
}
