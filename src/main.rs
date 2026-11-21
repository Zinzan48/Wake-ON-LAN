#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod icon;
mod model;
mod monitor;
mod storage;
mod ui_icons;
mod win_net;
mod win_tray;

use app::WakeOnLanApp;
use eframe::egui;

fn main() -> eframe::Result<()> {
    // 單一實例檢查：若已有實例在執行中（包含縮在系統匣），直接喚回原本視窗並結束本次重複啟動
    let _single_instance = match win_tray::acquire_single_instance_or_activate_existing() {
        Some(guard) => guard,
        None => return Ok(()),
    };

    let config = storage::load_config();
    let local_subnets = win_net::detect_local_subnets();
    let icon_rgba = icon::generate_app_icon_rgba(64);

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([980.0, 680.0])
            .with_min_inner_size([760.0, 520.0])
            .with_icon(egui::IconData {
                rgba: icon_rgba,
                width: 64,
                height: 64,
            }),
        ..Default::default()
    };

    eframe::run_native(
        "Wake-on-LAN 區網喚醒工具",
        native_options,
        Box::new(move |cc| Ok(Box::new(WakeOnLanApp::new(cc, config, local_subnets)))),
    )
}
