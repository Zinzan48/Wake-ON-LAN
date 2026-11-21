# Changelog

記錄 Wake-on-LAN 區網喚醒工具各版本的變更內容。
格式參考 [Keep a Changelog](https://keepachangelog.com/zh-TW/1.1.0/)，版本號採用 [Semantic Versioning](https://semver.org/lang/zh-TW/)。

## [Unreleased]

## [0.1.0] - 2026-11-21

首個正式版本，提供 Windows 區網掃描、特定 IP 查詢、背景開關機監控與 Wake-on-LAN 喚醒功能。

### Added
- 啟動時自動偵測本機 IPv4 網卡並預帶網段（大遮罩預設帶入本機 `/24`），不會主動發起掃描
- 支援網段（CIDR、`.0` 結尾、前綴簡寫）、IP 區間、單一 IP 與逗號分隔多筆特定 IP 查詢；輸入未對齊大遮罩（如 `192.168.2.0/22`）時自動縮減為 `/24`
- 左側欄集中管理已儲存網段（別名、預設值、刪除）與本機網卡快選
- 區網探索採用 Windows 原生 `SendARP` 與 `IcmpSendEcho`，免管理員權限也不會跳出命令列黑窗
- 受監控設備清單支援從掃描結果加入或手動輸入 IP / MAC / 廣播位址 / 選用 TCP 輔助探測埠，設定保存於 `%APPDATA%\WakeOnLanTool\config.json`
- 背景定期監控開關機狀態，連續無回應達次數後發送 Windows 系統通知並在視窗上方顯示提醒，點擊通知可直接叫回視窗確認是否開機（同次關機不重複提醒）
- 支援縮小至 Windows 系統匣常駐，右鍵選單可直接喚醒全部關機設備、重新檢測或暫停／開始監控
- WoL 封包發送會自動比對同網段本機網卡送往真實子網廣播位址並補送 `255.255.255.255`
- 單一實例檢查，重複開啟執行檔時自動喚回原本視窗（包含已縮在系統匣的狀態）
- 設定視窗提供「跟隨系統 / 淺色 / 暗黑」主題切換，並同步更新 Windows 原生標題列顏色
- 內建純向量介面圖示與嵌入 `.exe` 的應用程式圖示
- 新增 GitHub Actions CI 驗證與推送 `v*` Tag 自動打包 `.zip` 並建立 GitHub Release 的流程
