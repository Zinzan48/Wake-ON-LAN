# Wake-on-LAN 區網喚醒工具

給 Windows 用的區域網路 Wake-on-LAN 桌面小工具，用 Rust 寫成，編譯出來是單一執行檔。放著就能跑，平常縮在系統匣幫你盯著區網裡的電腦，哪台關機了就跳通知問你要不要叫醒。

## 系統需求（直接執行 Release 版）

從 GitHub Releases 下載 `.zip` 解壓後就能執行，不需要裝 Rust 或其他執行期套件，也不需要系統管理員權限。

- 作業系統：Windows 10（1903 以上）或 Windows 11，64 位元（x86_64）
- 顯示環境：支援 DirectX 12 或 Vulkan（Windows 10/11 內建即可）
- 系統字型：使用 Windows 內建的微軟正黑體（`msjh.ttc`，找不到時會退回 `mingliu.ttc` 或 `msyh.ttc`）
- 網路條件：本機與目標主機需在同一區網（若要跨子網喚醒，路由器需允許 Directed Broadcast）
- 目標主機設定：BIOS/UEFI 與 Windows 網卡驅動裡必須先開啟 Wake on LAN（Magic Packet）
- 防火牆：本機不需額外設定規則；若目標主機封鎖 ICMP Ping，可在設備設定中補填一個開放的 TCP 埠（像遠端桌面 `3389` 或 SMB `445`）當作備用探測

設定檔與設備清單保存在 `%APPDATA%\WakeOnLanTool\config.json`。如果不想用了，關掉程式再刪掉這個資料夾就乾淨移除。

## 功能一覽

開啟程式時會先偵測本機啟用中的 IPv4 網卡，預帶上次用過、預設儲存或本機所在的 `/24` 網段。預帶只填進輸入欄，不會自動開始掃描，按了查詢或掃描才會發送封包。

輸入欄支援網段、區間與特定 IP，打字時下方會先顯示實際要查詢的目標：

| 輸入範例 | 實際行為 |
|---|---|
| `192.168.2.50` 或 `192.168.2.50/32` | 只查這一台，在線或無回應都會顯示結果 |
| `192.168.2.10, 192.168.2.50` | 只查指定的這幾台 |
| `192.168.2.0` 或 `192.168.2.` | 視為 `192.168.2.0/24`（254 個 IP） |
| `192.168.2.0/22` 或 `192.168.2.0/16` | 非對齊大遮罩會自動縮成 `192.168.2.0/24`，不會被拉回 `192.168.0.0` |
| `192.168.0.0/22` | 對齊的大遮罩維持原範圍（單次上限 1,024 個 IP） |
| `192.168.2.10-50` | 掃描指定起訖區間 |

常查的網段可以加上別名存進左側欄，也能指定哪一組為開啟時的預設網段。

已關機的電腦不會回應 ARP 或 Ping，所以需要先在開機時掃進「受監控設備」，或直接手動輸入 IP 與 MAC。加入後背景執行緒會定期檢測開關機狀態，按右上角關閉（`X`）時預設會縮到系統匣繼續跑。當設備連續幾次無回應被判定關機時，會跳出 Windows 系統通知並在視窗上方顯示提醒列，點通知就能叫回視窗，直接決定要開機、略過或先停用該台監控。同一次關機只會提醒一次，等設備重新上線後才會重置。

發送 WoL 封包時，程式會先找同網段的本機網卡介面送往真實子網廣播位址，再補送 `255.255.255.255`，避免電腦裝了 WSL、Docker 或 VMware 虛擬網卡時封包走錯路。重複開啟執行檔時會自動喚回原本那一個視窗，不會開出兩個。

## 為什麼只用 4 個直接依賴

這個工具只直接依賴 `eframe`、`windows-sys`、`serde`、`serde_json`，其餘都走 Windows 系統內建的 DLL 與 Rust 標準庫：

- 網卡偵測與區網探測走 `iphlpapi.dll`（`GetAdaptersAddresses`、`SendARP`、`IcmpSendEcho`），不用呼叫 `ping.exe`，也不需要管理員權限
- 系統匣、右鍵選單與桌面通知走 `shell32.dll`（`Shell_NotifyIconW`）與 `user32.dll`（`TrackPopupMenu`）
- 單一實例檢查走 `kernel32.dll`（`CreateMutexW`），標題列深淺色同步走 `dwmapi.dll`（`DwmSetWindowAttribute`）
- WoL 封包直接用 Rust 標準庫的 `UdpSocket`

## 下載與執行

1. 到 GitHub Releases 頁面下載最新的 `wake-on-lan-tool-vX.Y.Z-windows-x86_64.zip`。
2. 解壓縮後執行 `wake-on-lan-tool.exe`。

## 從原始碼建置（開發者需求）

需要 Rust 1.95 以上版本與 Windows MSVC toolchain。

```powershell
# 執行單元測試
cargo test

# 編譯 Release 單一執行檔
cargo build --release
```

編譯出來的檔案會在 `target\release\wake-on-lan-tool.exe`。

## 發佈新版

這個儲存庫已經設好 GitHub Actions（[ci.yml](.github/workflows/ci.yml) 與 [release.yml](.github/workflows/release.yml)）。要發版時：

1. 更新 `Cargo.toml` 的 `version`，並在 `CHANGELOG.md` 補上對應版本區塊（`## [X.Y.Z] - YYYY-MM-DD`）。
2. 提交後建立對應的 Tag 推上 GitHub：

```powershell
git add .
git commit -m "chore: release v0.1.0"
git tag v0.1.0
git push origin master --tags
```

推送 `v*` Tag 後，Workflow 會自動核對版本號、跑測試、編譯 Release、打包 `.zip` 與 `SHA256SUMS.txt`，並擷取 `CHANGELOG.md` 內容建立 GitHub Release。
