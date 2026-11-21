use crate::model::{AppSettings, Device, DeviceRuntime, PendingWakeAlert, PowerStatus};
use crate::win_net::{parse_mac, probe_device_online, send_wol};
use crate::win_tray::{TrayHandle, local_time_hhmmss};
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

pub const WAKE_TIMEOUT_SECS: u64 = 90;

#[derive(Debug, Clone)]
pub enum MonitorEvent {
    Probing(bool),
    StateUpdated {
        device_id: String,
        runtime: DeviceRuntime,
    },
    OfflineAlert(PendingWakeAlert),
}

#[derive(Debug, Clone)]
struct SharedInput {
    devices: Vec<Device>,
    settings: AppSettings,
    force_check: bool,
    one_shot_check: bool,
    stop: bool,
    waking_requests: Vec<String>,
    reset_notify_ids: Vec<String>,
}

/// 監控執行緒控制器（主介面與系統匣共用，即使視窗隱藏於系統匣也能直接操作）
#[derive(Clone)]
pub struct MonitorController {
    shared: Arc<(Mutex<SharedInput>, Condvar)>,
    runtimes: Arc<Mutex<HashMap<String, DeviceRuntime>>>,
}

impl MonitorController {
    pub fn start(
        initial_devices: Vec<Device>,
        initial_settings: AppSettings,
        tray: TrayHandle,
        on_ui_repaint: Arc<dyn Fn() + Send + Sync>,
    ) -> (Self, mpsc::Receiver<MonitorEvent>) {
        let shared = Arc::new((
            Mutex::new(SharedInput {
                devices: initial_devices,
                settings: initial_settings,
                force_check: true,
                one_shot_check: false,
                stop: false,
                waking_requests: Vec::new(),
                reset_notify_ids: Vec::new(),
            }),
            Condvar::new(),
        ));
        let runtimes = Arc::new(Mutex::new(HashMap::new()));
        let (event_tx, event_rx) = mpsc::channel();

        let shared_bg = Arc::clone(&shared);
        let runtimes_bg = Arc::clone(&runtimes);

        thread::spawn(move || {
            monitor_loop(shared_bg, runtimes_bg, event_tx, tray, on_ui_repaint);
        });

        (Self { shared, runtimes }, event_rx)
    }

    pub fn sync_config(&self, devices: Vec<Device>, settings: AppSettings) {
        let (lock, cvar) = &*self.shared;
        if let Ok(mut guard) = lock.lock() {
            guard.devices = devices;
            guard.settings = settings;
            cvar.notify_all();
        }
    }

    pub fn trigger_check_now(&self) {
        let (lock, cvar) = &*self.shared;
        if let Ok(mut guard) = lock.lock() {
            guard.force_check = true;
            guard.one_shot_check = true;
            cvar.notify_all();
        }
    }

    pub fn mark_device_waking(&self, device_id: &str) {
        let (lock, cvar) = &*self.shared;
        if let Ok(mut guard) = lock.lock() {
            guard.waking_requests.push(device_id.to_string());
            guard.force_check = true;
            cvar.notify_all();
        }
    }

    pub fn reset_offline_notification(&self, device_id: &str) {
        let (lock, cvar) = &*self.shared;
        if let Ok(mut guard) = lock.lock() {
            guard.reset_notify_ids.push(device_id.to_string());
            cvar.notify_all();
        }
    }

    pub fn toggle_pause(&self) -> bool {
        let (lock, cvar) = &*self.shared;
        if let Ok(mut guard) = lock.lock() {
            guard.settings.monitor_paused = !guard.settings.monitor_paused;
            let paused = guard.settings.monitor_paused;
            cvar.notify_all();
            paused
        } else {
            false
        }
    }

    /// 即使視窗縮小在系統匣中，也能直接從系統匣右鍵選單喚醒所有已關機設備
    pub fn wake_all_offline_now(&self) -> (usize, usize) {
        let devices = {
            let (lock, _) = &*self.shared;
            lock.lock().map(|g| g.devices.clone()).unwrap_or_default()
        };
        let rt_snapshot = self
            .runtimes
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();

        let mut ok_count = 0usize;
        let mut fail_count = 0usize;

        for dev in devices {
            let is_offline = rt_snapshot
                .get(&dev.id)
                .is_some_and(|r| matches!(r.status, PowerStatus::Offline));
            if !is_offline {
                continue;
            }
            let bcast = dev.broadcast_ip.parse::<Ipv4Addr>().ok();
            match parse_mac(&dev.mac).and_then(|mac| send_wol(mac, bcast, dev.wol_port)) {
                Ok(_) => {
                    ok_count += 1;
                    self.mark_device_waking(&dev.id);
                }
                Err(_) => {
                    fail_count += 1;
                }
            }
        }
        (ok_count, fail_count)
    }

    pub fn stop(&self) {
        let (lock, cvar) = &*self.shared;
        if let Ok(mut guard) = lock.lock() {
            guard.stop = true;
            cvar.notify_all();
        }
    }
}

/// 純函式狀態機：評估單次探測結果，回傳是否應觸發新的「已關機提醒」
pub fn evaluate_probe_result(
    rt: &mut DeviceRuntime,
    probe_online: Option<Option<u32>>,
    offline_threshold: u32,
    now: Instant,
    now_text: &str,
) -> bool {
    let threshold = offline_threshold.max(1);
    rt.last_checked_text = Some(now_text.to_string());

    if let Some(rtt_ms) = probe_online {
        rt.status = PowerStatus::Online { rtt_ms };
        rt.consecutive_failures = 0;
        rt.notified_offline = false;
        rt.wake_timed_out = false;
        rt.last_seen_text = Some(now_text.to_string());
        return false;
    }

    // 若處於喚醒中，給設備開機緩衝時間（預設 90 秒）
    if let PowerStatus::Waking { started_at } = rt.status {
        if now.duration_since(started_at) < Duration::from_secs(WAKE_TIMEOUT_SECS) {
            return false;
        }
        rt.status = PowerStatus::Offline;
        rt.wake_timed_out = true;
        rt.consecutive_failures = threshold;
        // 喚醒逾時不再次跳出重複關機彈窗，改於畫面顯示逾時提示
        rt.notified_offline = true;
        return false;
    }

    rt.consecutive_failures = rt.consecutive_failures.saturating_add(1);
    if rt.consecutive_failures >= threshold {
        rt.status = PowerStatus::Offline;
        if !rt.notified_offline {
            rt.notified_offline = true;
            return true;
        }
    }
    false
}

fn monitor_loop(
    shared: Arc<(Mutex<SharedInput>, Condvar)>,
    runtimes: Arc<Mutex<HashMap<String, DeviceRuntime>>>,
    event_tx: mpsc::Sender<MonitorEvent>,
    tray: TrayHandle,
    on_ui_repaint: Arc<dyn Fn() + Send + Sync>,
) {
    let mut last_poll: Option<Instant> = None;

    loop {
        let (devices, settings, should_poll) = {
            let (lock, cvar) = &*shared;
            let mut guard = match lock.lock() {
                Ok(g) => g,
                Err(_) => break,
            };

            if guard.stop {
                break;
            }

            // 套用喚醒標記與重設提醒要求
            if (!guard.waking_requests.is_empty() || !guard.reset_notify_ids.is_empty())
                && let Ok(mut rt_map) = runtimes.lock()
            {
                for id in guard.waking_requests.drain(..) {
                    let entry = rt_map.entry(id.clone()).or_default();
                    entry.status = PowerStatus::Waking {
                        started_at: Instant::now(),
                    };
                    entry.wake_timed_out = false;
                    entry.notified_offline = true;
                    let _ = event_tx.send(MonitorEvent::StateUpdated {
                        device_id: id,
                        runtime: entry.clone(),
                    });
                }
                for id in guard.reset_notify_ids.drain(..) {
                    if let Some(entry) = rt_map.get_mut(&id) {
                        entry.notified_offline = false;
                    }
                }
                on_ui_repaint();
            }

            let interval = Duration::from_secs(guard.settings.poll_interval_secs.clamp(5, 300));
            let one_shot = guard.one_shot_check;
            let due = one_shot
                || (!guard.settings.monitor_paused
                    && (guard.force_check || last_poll.is_none_or(|t| t.elapsed() >= interval)));

            if !due {
                let wait_dur = if guard.settings.monitor_paused {
                    Duration::from_secs(2)
                } else if let Some(t) = last_poll {
                    interval.saturating_sub(t.elapsed()).min(Duration::from_secs(2))
                } else {
                    Duration::from_millis(200)
                };
                let (new_guard, _) = match cvar.wait_timeout(guard, wait_dur) {
                    Ok(v) => v,
                    Err(_) => break,
                };
                if new_guard.stop {
                    break;
                }
                continue;
            }

            guard.force_check = false;
            guard.one_shot_check = false;
            (guard.devices.clone(), guard.settings.clone(), true)
        };

        if !should_poll {
            continue;
        }
        last_poll = Some(Instant::now());

        let monitored_devices: Vec<Device> =
            devices.into_iter().filter(|d| d.monitored).collect();

        if monitored_devices.is_empty() {
            tray.update_tooltip("Wake-on-LAN - 尚無監控設備");
            continue;
        }

        let _ = event_tx.send(MonitorEvent::Probing(true));
        on_ui_repaint();

        // 並行探測受監控設備
        let mut handles = Vec::with_capacity(monitored_devices.len());
        for dev in monitored_devices {
            handles.push(thread::spawn(move || {
                let probe_res = dev
                    .ip
                    .parse::<Ipv4Addr>()
                    .ok()
                    .and_then(|ip| probe_device_online(ip, dev.tcp_probe_port));
                (dev, probe_res)
            }));
        }

        let mut online_count = 0usize;
        let mut offline_count = 0usize;
        let mut waking_count = 0usize;
        let now = Instant::now();
        let now_text = local_time_hhmmss();

        for handle in handles {
            let Ok((dev, probe_res)) = handle.join() else {
                continue;
            };

            let (updated_rt, trigger_alert) = {
                let mut rt_map = match runtimes.lock() {
                    Ok(g) => g,
                    Err(_) => continue,
                };
                let entry = rt_map.entry(dev.id.clone()).or_default();
                let should_alert = evaluate_probe_result(
                    entry,
                    probe_res,
                    settings.offline_threshold,
                    now,
                    &now_text,
                );
                (entry.clone(), should_alert)
            };

            match updated_rt.status {
                PowerStatus::Online { .. } => online_count += 1,
                PowerStatus::Offline => offline_count += 1,
                PowerStatus::Waking { .. } => waking_count += 1,
                PowerStatus::Unknown => {}
            }

            let _ = event_tx.send(MonitorEvent::StateUpdated {
                device_id: dev.id.clone(),
                runtime: updated_rt,
            });

            if trigger_alert {
                let alert = PendingWakeAlert {
                    device_id: dev.id.clone(),
                    device_name: dev.name.clone(),
                    ip: dev.ip.clone(),
                    mac: dev.mac.clone(),
                    detected_at_text: now_text.clone(),
                };
                let _ = event_tx.send(MonitorEvent::OfflineAlert(alert));

                if settings.enable_desktop_notifications {
                    tray.show_offline_toast(
                        format!("{} 已關機", dev.name),
                        format!(
                            "{} ({}) 目前無回應，點此通知開啟視窗並確認是否開機。",
                            dev.name, dev.ip
                        ),
                    );
                }
            }
        }

        let summary = if waking_count > 0 {
            format!(
                "Wake-on-LAN - {online_count} 在線 / {offline_count} 關機 / {waking_count} 喚醒中"
            )
        } else {
            format!("Wake-on-LAN - {online_count} 在線 / {offline_count} 關機")
        };
        tray.update_tooltip(summary);
        let _ = event_tx.send(MonitorEvent::Probing(false));
        on_ui_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_threshold_and_deduplication() {
        let mut rt = DeviceRuntime::default();
        let t0 = Instant::now();

        // 第 1 次失敗尚未達閾值(2)，不觸發提醒
        assert!(!evaluate_probe_result(&mut rt, None, 2, t0, "10:00:00"));
        assert_eq!(rt.status, PowerStatus::Unknown);
        assert_eq!(rt.consecutive_failures, 1);

        // 第 2 次失敗達閾值，轉為 Offline 並觸發一次提醒
        assert!(evaluate_probe_result(&mut rt, None, 2, t0, "10:00:15"));
        assert_eq!(rt.status, PowerStatus::Offline);
        assert!(rt.notified_offline);

        // 同次關機第 3 次失敗，不重複打擾
        assert!(!evaluate_probe_result(&mut rt, None, 2, t0, "10:00:30"));
        assert_eq!(rt.status, PowerStatus::Offline);

        // 設備開機後重置提醒旗標
        assert!(!evaluate_probe_result(
            &mut rt,
            Some(Some(3)),
            2,
            t0,
            "10:01:00"
        ));
        assert_eq!(rt.status, PowerStatus::Online { rtt_ms: Some(3) });
        assert!(!rt.notified_offline);
        assert_eq!(rt.consecutive_failures, 0);

        // 再次關機時應能重新觸發提醒
        assert!(!evaluate_probe_result(&mut rt, None, 2, t0, "10:02:00"));
        assert!(evaluate_probe_result(&mut rt, None, 2, t0, "10:02:15"));
    }

    #[test]
    fn waking_grace_period_and_timeout() {
        let mut rt = DeviceRuntime::default();
        let start = Instant::now();
        rt.status = PowerStatus::Waking { started_at: start };
        rt.notified_offline = true;

        // 喚醒緩衝時間內不判定逾時
        assert!(!evaluate_probe_result(
            &mut rt,
            None,
            2,
            start + Duration::from_secs(30),
            "10:00:30"
        ));
        assert!(matches!(rt.status, PowerStatus::Waking { .. }));
        assert!(!rt.wake_timed_out);

        // 超過 90 秒判定喚醒逾時並回到 Offline
        assert!(!evaluate_probe_result(
            &mut rt,
            None,
            2,
            start + Duration::from_secs(WAKE_TIMEOUT_SECS + 1),
            "10:01:31"
        ));
        assert_eq!(rt.status, PowerStatus::Offline);
        assert!(rt.wake_timed_out);
    }
}
