use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// 產生簡單的唯一識別碼（無須額外 UUID 依賴）
pub fn generate_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{ts}_{seq}")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SavedSubnet {
    pub id: String,
    pub label: String,
    pub cidr: String,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub ip: String,
    pub mac: String,
    #[serde(default)]
    pub broadcast_ip: String,
    #[serde(default = "default_wol_port")]
    pub wol_port: u16,
    #[serde(default = "default_true")]
    pub monitored: bool,
    #[serde(default)]
    pub tcp_probe_port: Option<u16>,
    #[serde(default)]
    pub last_seen_text: Option<String>,
}

fn default_wol_port() -> u16 {
    9
}

fn default_true() -> bool {
    true
}

fn default_poll_interval() -> u64 {
    15
}

fn default_offline_threshold() -> u32 {
    2
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppSettings {
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
    #[serde(default = "default_offline_threshold")]
    pub offline_threshold: u32,
    #[serde(default = "default_true")]
    pub minimize_to_tray_on_close: bool,
    #[serde(default = "default_true")]
    pub enable_desktop_notifications: bool,
    #[serde(default)]
    pub monitor_paused: bool,
    #[serde(default)]
    pub last_subnet_input: Option<String>,
    #[serde(default)]
    pub theme: ThemeMode,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            poll_interval_secs: default_poll_interval(),
            offline_threshold: default_offline_threshold(),
            minimize_to_tray_on_close: true,
            enable_desktop_notifications: true,
            monitor_paused: false,
            last_subnet_input: None,
            theme: ThemeMode::System,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppConfig {
    #[serde(default)]
    pub saved_subnets: Vec<SavedSubnet>,
    #[serde(default)]
    pub devices: Vec<Device>,
    #[serde(default)]
    pub settings: AppSettings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PowerStatus {
    Unknown,
    Online { rtt_ms: Option<u32> },
    Offline,
    Waking { started_at: Instant },
}

#[derive(Debug, Clone)]
pub struct DeviceRuntime {
    pub status: PowerStatus,
    pub consecutive_failures: u32,
    pub notified_offline: bool,
    pub last_checked_text: Option<String>,
    pub last_seen_text: Option<String>,
    pub wake_timed_out: bool,
}

impl Default for DeviceRuntime {
    fn default() -> Self {
        Self {
            status: PowerStatus::Unknown,
            consecutive_failures: 0,
            notified_offline: false,
            last_checked_text: None,
            last_seen_text: None,
            wake_timed_out: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PendingWakeAlert {
    pub device_id: String,
    pub device_name: String,
    pub ip: String,
    pub mac: String,
    pub detected_at_text: String,
}

#[derive(Debug, Clone)]
pub struct LocalSubnet {
    pub adapter_name: String,
    pub local_ip: Ipv4Addr,
    pub prefix_len: u8,
    pub network_ip: Ipv4Addr,
    pub broadcast_ip: Ipv4Addr,
    pub cidr: String,
    pub is_virtual: bool,
}

impl LocalSubnet {
    pub fn contains(&self, ip: Ipv4Addr) -> bool {
        let mask = if self.prefix_len == 0 {
            0u32
        } else if self.prefix_len >= 32 {
            u32::MAX
        } else {
            !0u32 << (32 - self.prefix_len)
        };
        (u32::from(ip) & mask) == u32::from(self.network_ip)
    }

    /// 若本機網卡遮罩大於 /24（如 /22、/16），預設預帶本機所在的 /24 子網以提升掃描精準度
    pub fn preferred_scan_cidr(&self) -> String {
        if self.prefix_len < 24 {
            let [a, b, c, _] = self.local_ip.octets();
            format!("{a}.{b}.{c}.0/24")
        } else {
            self.cidr.clone()
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScannedHost {
    pub ip: Ipv4Addr,
    pub mac: Option<String>,
    pub rtt_ms: Option<u32>,
    pub broadcast_ip: Ipv4Addr,
    pub online: bool,
}
