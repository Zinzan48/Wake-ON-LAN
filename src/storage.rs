use crate::model::AppConfig;
use std::fs;
use std::path::{Path, PathBuf};

const APP_DIR_NAME: &str = "WakeOnLanTool";
const CONFIG_FILE_NAME: &str = "config.json";

/// 取得設定檔預設路徑：%APPDATA%\WakeOnLanTool\config.json
pub fn default_config_path() -> PathBuf {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return PathBuf::from(appdata)
            .join(APP_DIR_NAME)
            .join(CONFIG_FILE_NAME);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        return parent.join(CONFIG_FILE_NAME);
    }
    PathBuf::from(CONFIG_FILE_NAME)
}

pub fn load_config() -> AppConfig {
    load_from(&default_config_path())
}

pub fn save_config(cfg: &AppConfig) -> Result<(), String> {
    save_to(&default_config_path(), cfg)
}

pub fn load_from(path: &Path) -> AppConfig {
    let Ok(raw) = fs::read_to_string(path) else {
        return AppConfig::default();
    };
    serde_json::from_str::<AppConfig>(&raw).unwrap_or_default()
}

pub fn save_to(path: &Path, cfg: &AppConfig) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|e| format!("建立設定目錄失敗: {e}"))?;
    }

    let json =
        serde_json::to_string_pretty(cfg).map_err(|e| format!("序列化設定失敗: {e}"))?;

    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, json.as_bytes()).map_err(|e| format!("寫入暫存設定檔失敗: {e}"))?;

    // Windows 上 rename 若目標存在有時會受檔案鎖影響，先嘗試直接 rename，失敗再覆蓋
    if let Err(first_err) = fs::rename(&tmp_path, path) {
        if path.exists() {
            let _ = fs::remove_file(path);
            fs::rename(&tmp_path, path)
                .map_err(|e| format!("更新設定檔失敗: {e} (原始錯誤: {first_err})"))?;
        } else {
            return Err(format!("更新設定檔失敗: {first_err}"));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Device, SavedSubnet};

    #[test]
    fn config_roundtrip_and_atomic_overwrite() {
        let dir = std::env::temp_dir().join(format!(
            "wol_tool_test_{}",
            crate::model::generate_id("cfg")
        ));
        let path = dir.join("config.json");

        let mut cfg = AppConfig::default();
        cfg.saved_subnets.push(SavedSubnet {
            id: "s1".to_string(),
            label: "辦公室區網".to_string(),
            cidr: "192.168.10.0/24".to_string(),
            is_default: true,
        });
        cfg.devices.push(Device {
            id: "d1".to_string(),
            name: "工作站".to_string(),
            ip: "192.168.10.25".to_string(),
            mac: "AA:BB:CC:DD:EE:FF".to_string(),
            broadcast_ip: "192.168.10.255".to_string(),
            wol_port: 9,
            monitored: true,
            tcp_probe_port: Some(3389),
            last_seen_text: Some("10:30:00".to_string()),
        });
        cfg.settings.poll_interval_secs = 20;
        cfg.settings.last_subnet_input = Some("192.168.10.0/24".to_string());

        save_to(&path, &cfg).expect("save first");
        let loaded = load_from(&path);
        assert_eq!(loaded, cfg);

        // 測試覆寫原子存檔
        cfg.settings.poll_interval_secs = 30;
        save_to(&path, &cfg).expect("save overwrite");
        let loaded2 = load_from(&path);
        assert_eq!(loaded2.settings.poll_interval_secs, 30);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_config_returns_default() {
        let path = std::env::temp_dir().join("wol_nonexistent_dir_xyz/config.json");
        let loaded = load_from(&path);
        assert_eq!(loaded, AppConfig::default());
    }
}
