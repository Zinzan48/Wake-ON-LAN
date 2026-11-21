use crate::model::{LocalSubnet, ScannedHost};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

/// 單次網段掃描最大 IP 數量防呆上限
pub const MAX_SCAN_HOSTS: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryMode {
    SingleIp,
    SpecificIps,
    Range,
    Subnet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubnetTarget {
    pub normalized: String,
    pub hosts: Vec<Ipv4Addr>,
    pub broadcast_ip: Ipv4Addr,
    pub mode: QueryMode,
    /// 若使用者從較大遮罩（例如 /22、/16）改輸入特定 C 段（例如 192.168.2.0/22），記錄原本的遮罩長度
    pub auto_narrowed_from: Option<u8>,
}

impl SubnetTarget {
    pub fn report_offline(&self) -> bool {
        matches!(self.mode, QueryMode::SingleIp | QueryMode::SpecificIps)
    }
}

#[derive(Debug, Clone)]
pub enum ScanEvent {
    Progress { done: usize, total: usize },
    HostFound(ScannedHost),
    Finished { cancelled: bool },
}

/// 計算指定 IPv4 與遮罩長度的網路位址與廣播位址
pub fn ipv4_network_and_broadcast(ip: Ipv4Addr, prefix_len: u8) -> (Ipv4Addr, Ipv4Addr) {
    let ip_u32 = u32::from(ip);
    let mask = if prefix_len == 0 {
        0u32
    } else if prefix_len >= 32 {
        u32::MAX
    } else {
        !0u32 << (32 - prefix_len)
    };
    let net = ip_u32 & mask;
    let bcast = net | !mask;
    (Ipv4Addr::from(net), Ipv4Addr::from(bcast))
}

/// 解析使用者輸入的目標（同時支援網段掃描與特定 IP 查詢）：
/// - 單一特定 IP：`192.168.2.50` 或 `192.168.2.50/32`（僅查詢該單一 IP）
/// - 多筆特定 IP：`192.168.2.10, 192.168.2.50`（逗號或分號分隔）
/// - 網段位址：`192.168.2.0`（第四段為 0，自動視為 `192.168.2.0/24`）
/// - 前綴簡寫：`192.168.2.` 或 `192.168.2`（自動視為 `192.168.2.0/24`）
/// - CIDR：`192.168.2.0/24`、`192.168.0.0/22`
///   * 若遮罩 `< 24` 但輸入 IP 未對齊該遮罩（例如將 `192.168.0.0/22` 改為 `192.168.2.0/22`），
///     自動縮減為 `/24`（`192.168.2.0/24`），避免被強制拉回 `192.168.0.0`！
/// - IP 區間：`192.168.2.10-50` 或 `192.168.2.10-192.168.2.50`
pub fn parse_subnet_spec(raw: &str) -> Result<SubnetTarget, String> {
    let input = raw.trim();
    if input.is_empty() {
        return Err("請輸入目標網段或特定 IP，例如 192.168.2.50 或 192.168.2.0/24".to_string());
    }

    // 0. 多筆特定 IP（逗號或分號分隔）：192.168.2.10, 192.168.2.50
    if input.contains(',') || input.contains(';') {
        let mut hosts = Vec::new();
        for part in input.split([',', ';']) {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                continue;
            }
            let ip: Ipv4Addr = trimmed
                .parse()
                .map_err(|_| format!("無效的 IPv4 位址: {trimmed}"))?;
            if !hosts.contains(&ip) {
                hosts.push(ip);
            }
        }
        if hosts.is_empty() {
            return Err("請輸入至少一個有效的 IPv4 位址".to_string());
        }
        if hosts.len() > MAX_SCAN_HOSTS {
            return Err(format!("指定 IP 數量超過單次上限 ({MAX_SCAN_HOSTS})"));
        }
        let (_, bcast) = ipv4_network_and_broadcast(hosts[0], 24);
        let normalized = hosts
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let mode = if hosts.len() == 1 {
            QueryMode::SingleIp
        } else {
            QueryMode::SpecificIps
        };
        return Ok(SubnetTarget {
            normalized,
            hosts,
            broadcast_ip: bcast,
            mode,
            auto_narrowed_from: None,
        });
    }

    // 1. CIDR 格式：a.b.c.d/prefix
    if let Some((ip_part, prefix_part)) = input.split_once('/') {
        let ip: Ipv4Addr = ip_part
            .trim()
            .parse()
            .map_err(|_| format!("無效的 IPv4 位址: {ip_part}"))?;
        let raw_prefix: u8 = prefix_part
            .trim()
            .parse()
            .map_err(|_| format!("無效的遮罩長度: {prefix_part}"))?;
        if raw_prefix > 32 {
            return Err("子網路遮罩長度必須介於 0 到 32 之間".to_string());
        }

        if raw_prefix == 32 {
            let (_, bcast) = ipv4_network_and_broadcast(ip, 24);
            return Ok(SubnetTarget {
                normalized: ip.to_string(),
                hosts: vec![ip],
                broadcast_ip: bcast,
                mode: QueryMode::SingleIp,
                auto_narrowed_from: None,
            });
        }

        let (raw_net, _) = ipv4_network_and_broadcast(ip, raw_prefix);
        // 若使用者原本是較大遮罩（例如 192.168.0.0/22 或 /16），改成 192.168.2.0/22（未對齊於原遮罩）
        // 則自動縮減為 /24，避免遮罩把 192.168.2.0 拉回 192.168.0.0
        let (effective_prefix, auto_narrowed_from) = if raw_prefix < 24 && ip != raw_net {
            (24, Some(raw_prefix))
        } else {
            (raw_prefix, None)
        };

        let (net, bcast) = ipv4_network_and_broadcast(ip, effective_prefix);
        let net_u32 = u32::from(net);
        let bcast_u32 = u32::from(bcast);

        let (start, end) = if effective_prefix >= 31 {
            (net_u32, bcast_u32)
        } else {
            (net_u32 + 1, bcast_u32 - 1)
        };
        let count = (end - start) as usize + 1;
        if count > MAX_SCAN_HOSTS {
            return Err(format!(
                "範圍內有 {count} 個 IP，超過單次上限 ({MAX_SCAN_HOSTS})，建議縮小到 /22～/24"
            ));
        }

        let hosts = (start..=end).map(Ipv4Addr::from).collect();
        return Ok(SubnetTarget {
            normalized: format!("{net}/{effective_prefix}"),
            hosts,
            broadcast_ip: bcast,
            mode: QueryMode::Subnet,
            auto_narrowed_from,
        });
    }

    // 2. IP 範圍格式：a.b.c.1-254 或 a.b.c.1-a.b.c.100
    if let Some((left, right)) = input.split_once('-') {
        let start_ip: Ipv4Addr = left
            .trim()
            .parse()
            .map_err(|_| format!("無效的起始 IPv4: {left}"))?;
        let right_trim = right.trim();
        let end_ip: Ipv4Addr = if let Ok(last_octet) = right_trim.parse::<u8>() {
            let [a, b, c, _] = start_ip.octets();
            Ipv4Addr::new(a, b, c, last_octet)
        } else {
            right_trim
                .parse()
                .map_err(|_| format!("無效的結束 IPv4: {right_trim}"))?
        };

        let start_u32 = u32::from(start_ip);
        let end_u32 = u32::from(end_ip);
        if end_u32 < start_u32 {
            return Err("結束 IP 不可小於起始 IP".to_string());
        }
        let count = (end_u32 - start_u32) as usize + 1;
        if count > MAX_SCAN_HOSTS {
            return Err(format!(
                "IP 範圍包含 {count} 個主機，超過單次安全上限 ({MAX_SCAN_HOSTS})"
            ));
        }

        let (_, bcast) = ipv4_network_and_broadcast(start_ip, 24);
        let hosts = (start_u32..=end_u32).map(Ipv4Addr::from).collect();
        return Ok(SubnetTarget {
            normalized: format!("{start_ip}-{end_ip}"),
            hosts,
            broadcast_ip: bcast,
            mode: QueryMode::Range,
            auto_narrowed_from: None,
        });
    }

    // 3. 三段前綴簡寫：192.168.2. 或 192.168.2 -> 192.168.2.0/24
    let trimmed_dot = input.trim_end_matches('.');
    let octets: Vec<&str> = trimmed_dot.split('.').collect();
    if octets.len() == 3 {
        let a: u8 = octets[0]
            .parse()
            .map_err(|_| format!("無效的網段前綴: {input}"))?;
        let b: u8 = octets[1]
            .parse()
            .map_err(|_| format!("無效的網段前綴: {input}"))?;
        let c: u8 = octets[2]
            .parse()
            .map_err(|_| format!("無效的網段前綴: {input}"))?;
        let cidr = format!("{a}.{b}.{c}.0/24");
        return parse_subnet_spec(&cidr);
    }

    // 4. 完整四段 IPv4（無遮罩）：
    //    - 第四段為 0（如 192.168.2.0）-> 自動視為該 /24 網段（192.168.2.0/24）
    //    - 第四段非 0（如 192.168.2.50）-> 視為「特定單一 IP」查詢（只查這 1 台！）
    if let Ok(ip) = input.parse::<Ipv4Addr>() {
        let [a, b, c, d] = ip.octets();
        if d == 0 {
            let cidr = format!("{a}.{b}.{c}.0/24");
            return parse_subnet_spec(&cidr);
        }
        let (_, bcast) = ipv4_network_and_broadcast(ip, 24);
        return Ok(SubnetTarget {
            normalized: ip.to_string(),
            hosts: vec![ip],
            broadcast_ip: bcast,
            mode: QueryMode::SingleIp,
            auto_narrowed_from: None,
        });
    }

    Err(format!(
        "無法解析「{input}」，支援格式：特定 IP (192.168.2.50)、多筆 IP (192.168.2.10, 192.168.2.20)、網段 (192.168.2.0 或 192.168.2.0/24)、區間 (192.168.2.1-50)"
    ))
}

/// 解析並驗證 MAC 位址，支援 AA:BB:CC:DD:EE:FF、AA-BB-CC-DD-EE-FF、AABBCCDDEEFF
pub fn parse_mac(input: &str) -> Result<[u8; 6], String> {
    let hex_only: String = input
        .trim()
        .chars()
        .filter(|c| !matches!(c, ':' | '-' | '.' | ' '))
        .collect();
    if hex_only.len() != 12 {
        return Err(format!("無效的 MAC 位址格式: {input}"));
    }
    let mut mac = [0u8; 6];
    for (i, slot) in mac.iter_mut().enumerate() {
        let part = &hex_only[i * 2..i * 2 + 2];
        *slot = u8::from_str_radix(part, 16)
            .map_err(|_| format!("MAC 位址包含非十六進位字元: {input}"))?;
    }
    if mac == [0u8; 6] {
        return Err("MAC 位址不可為全零".to_string());
    }
    Ok(mac)
}

pub fn format_mac(mac: [u8; 6]) -> String {
    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}

/// 建構標準 102-byte Wake-on-LAN Magic Packet
pub fn build_magic_packet(mac: [u8; 6]) -> [u8; 102] {
    let mut packet = [0u8; 102];
    packet[0..6].fill(0xFF);
    for i in 0..16 {
        let offset = 6 + i * 6;
        packet[offset..offset + 6].copy_from_slice(&mac);
    }
    packet
}

/// 發送 Wake-on-LAN Magic Packet（支援多網卡綁定對應區網介面）
pub fn send_wol(
    mac: [u8; 6],
    broadcast_ip: Option<Ipv4Addr>,
    port: u16,
) -> Result<Vec<String>, String> {
    let packet = build_magic_packet(mac);
    let port = if port == 0 { 9 } else { port };
    let local_subnets = detect_local_subnets();
    let mut sent_targets = Vec::new();
    let mut last_err: Option<String> = None;

    // 1. 若指定廣播位址，優先尋找涵蓋該網段的本機網卡 IP 綁定發送
    if let Some(bcast) = broadcast_ip {
        let matching_subnet = local_subnets
            .iter()
            .find(|s| s.broadcast_ip == bcast || s.contains(bcast));

        let bind_ip = matching_subnet
            .map(|s| s.local_ip)
            .unwrap_or(Ipv4Addr::UNSPECIFIED);
        match send_udp_broadcast(bind_ip, bcast, port, &packet) {
            Ok(()) => sent_targets.push(format!("{bcast}:{port} (介面 {bind_ip})")),
            Err(e) => last_err = Some(e),
        }

        // 若本機實體網卡屬於較大遮罩（例如 /22，真實廣播為 192.168.3.255 而非 192.168.2.255），補送真實網卡廣播位址
        if let Some(sub) = matching_subnet
            && sub.broadcast_ip != bcast
            && send_udp_broadcast(sub.local_ip, sub.broadcast_ip, port, &packet).is_ok()
        {
            sent_targets.push(format!("{}:{} ({})", sub.broadcast_ip, port, sub.adapter_name));
        }
    } else {
        // 未指定廣播位址時，對所有非虛擬優先網段發送
        for subnet in local_subnets.iter().filter(|s| !s.is_virtual) {
            match send_udp_broadcast(subnet.local_ip, subnet.broadcast_ip, port, &packet) {
                Ok(()) => sent_targets.push(format!(
                    "{}:{} ({})",
                    subnet.broadcast_ip, port, subnet.adapter_name
                )),
                Err(e) => last_err = Some(e),
            }
        }
    }

    // 2. 補送受限廣播 255.255.255.255（部分路由器/網卡環境需要）
    if broadcast_ip != Some(Ipv4Addr::BROADCAST)
        && send_udp_broadcast(Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST, port, &packet).is_ok()
    {
        sent_targets.push(format!("255.255.255.255:{port}"));
    }

    if sent_targets.is_empty() {
        Err(last_err.unwrap_or_else(|| "無法發送 WoL 封包".to_string()))
    } else {
        Ok(sent_targets)
    }
}

fn send_udp_broadcast(
    bind_ip: Ipv4Addr,
    target_ip: Ipv4Addr,
    port: u16,
    packet: &[u8; 102],
) -> Result<(), String> {
    let socket = UdpSocket::bind(SocketAddrV4::new(bind_ip, 0))
        .map_err(|e| format!("綁定 UDP 介面 {bind_ip} 失敗: {e}"))?;
    socket
        .set_broadcast(true)
        .map_err(|e| format!("啟用 UDP Broadcast 失敗: {e}"))?;
    socket
        .send_to(packet, SocketAddrV4::new(target_ip, port))
        .map_err(|e| format!("傳送至 {target_ip}:{port} 失敗: {e}"))?;
    Ok(())
}

/// TCP 連接埠探測（可選，供封鎖 ICMP Ping 的主機使用）
pub fn tcp_probe(ip: Ipv4Addr, port: u16, timeout_ms: u64) -> bool {
    let addr = SocketAddr::V4(SocketAddrV4::new(ip, port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(timeout_ms)).is_ok()
}

/// 監控用存活探測：優先使用 Windows 原生 ICMP Ping，無回應且有設定 TCP 埠時以 TCP 補測
pub fn probe_device_online(ip: Ipv4Addr, tcp_port: Option<u16>) -> Option<Option<u32>> {
    if let Some(rtt) = icmp_ping(ip, 900) {
        return Some(Some(rtt));
    }
    if let Some(port) = tcp_port
        && tcp_probe(ip, port, 900)
    {
        return Some(None);
    }
    None
}

/// 啟動手動區網掃描或特定 IP 查詢（僅於使用者明確點擊按鈕後呼叫）
pub fn start_subnet_scan(
    target: SubnetTarget,
    cancel: Arc<AtomicBool>,
    tx: mpsc::Sender<ScanEvent>,
    on_update: Arc<dyn Fn() + Send + Sync>,
) {
    thread::spawn(move || {
        let total = target.hosts.len();
        let report_offline = target.report_offline();
        let hosts = Arc::new(target.hosts);
        let fallback_broadcast = target.broadcast_ip;
        let local_subnets = Arc::new(detect_local_subnets());
        let next_idx = Arc::new(AtomicUsize::new(0));
        let done_count = Arc::new(AtomicUsize::new(0));

        let worker_count = total.clamp(1, 48);
        let mut handles = Vec::with_capacity(worker_count);

        for _ in 0..worker_count {
            let hosts = Arc::clone(&hosts);
            let local_subnets = Arc::clone(&local_subnets);
            let next_idx = Arc::clone(&next_idx);
            let done_count = Arc::clone(&done_count);
            let cancel = Arc::clone(&cancel);
            let tx = tx.clone();
            let on_update = Arc::clone(&on_update);

            handles.push(thread::spawn(move || {
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let idx = next_idx.fetch_add(1, Ordering::Relaxed);
                    if idx >= hosts.len() {
                        break;
                    }
                    let ip = hosts[idx];
                    let real_broadcast = local_subnets
                        .iter()
                        .find(|s| s.contains(ip))
                        .map(|s| s.broadcast_ip)
                        .unwrap_or(fallback_broadcast);

                    // 同區網優先使用 Windows 原生 SendARP（可抓到 MAC，且不受 ICMP 防火牆限制）
                    let mac_bytes = send_arp(ip);
                    let rtt_ms = if mac_bytes.is_some() {
                        icmp_ping(ip, 450)
                    } else {
                        // 跨子網或非 L2 同網段時仍以 ICMP Ping 探測
                        icmp_ping(ip, if report_offline { 900 } else { 450 })
                    };

                    let online = mac_bytes.is_some() || rtt_ms.is_some();
                    if online || report_offline {
                        let _ = tx.send(ScanEvent::HostFound(ScannedHost {
                            ip,
                            mac: mac_bytes.map(format_mac),
                            rtt_ms,
                            broadcast_ip: real_broadcast,
                            online,
                        }));
                    }

                    let done = done_count.fetch_add(1, Ordering::Relaxed) + 1;
                    if done == total || done.is_multiple_of(4) || online || report_offline {
                        let _ = tx.send(ScanEvent::Progress { done, total });
                        on_update();
                    }
                }
            }));
        }

        for h in handles {
            let _ = h.join();
        }

        let cancelled = cancel.load(Ordering::Relaxed);
        let _ = tx.send(ScanEvent::Finished { cancelled });
        on_update();
    });
}

fn is_virtual_adapter_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    const KEYWORDS: [&str; 10] = [
        "vethernet",
        "wsl",
        "hyper-v",
        "docker",
        "vmware",
        "virtualbox",
        "tailscale",
        "zerotier",
        "loopback",
        "bluetooth",
    ];
    KEYWORDS.iter().any(|k| lower.contains(k))
}

fn subnet_priority(s: &LocalSubnet) -> u8 {
    let base: u8 = if s.is_virtual { 10 } else { 0 };
    let [a, b, _, _] = s.local_ip.octets();
    let range_score = if a == 192 && b == 168 {
        0
    } else if a == 10 {
        1
    } else if a == 172 && (16..=31).contains(&b) {
        2
    } else {
        3
    };
    base + range_score
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::ffi::c_void;
    use std::ptr;
    use windows_sys::Win32::Foundation::{
        ERROR_BUFFER_OVERFLOW, INVALID_HANDLE_VALUE, NO_ERROR,
    };
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST,
        GetAdaptersAddresses, ICMP_ECHO_REPLY, IP_ADAPTER_ADDRESSES_LH, IcmpCloseHandle,
        IcmpCreateFile, IcmpSendEcho, SendARP,
    };
    use windows_sys::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};

    const IF_OPER_STATUS_UP: i32 = 1;
    const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
    const IP_SUCCESS: u32 = 0;

    unsafe fn pwstr_to_string(ptr: *const u16) -> String {
        if ptr.is_null() {
            return String::new();
        }
        let mut len = 0usize;
        unsafe {
            while *ptr.add(len) != 0 {
                len += 1;
            }
            let slice = std::slice::from_raw_parts(ptr, len);
            String::from_utf16_lossy(slice)
        }
    }

    /// 透過 Windows 原生 iphlpapi.dll (GetAdaptersAddresses) 偵測本機有效 IPv4 網段
    pub fn detect_local_subnets() -> Vec<LocalSubnet> {
        let flags =
            GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
        let mut buf_len: u32 = 16 * 1024;
        let mut buffer = vec![0u8; buf_len as usize];

        let mut ret = unsafe {
            GetAdaptersAddresses(
                AF_INET as u32,
                flags,
                ptr::null_mut(),
                buffer.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH,
                &mut buf_len,
            )
        };

        if ret == ERROR_BUFFER_OVERFLOW {
            buffer.resize(buf_len as usize, 0);
            ret = unsafe {
                GetAdaptersAddresses(
                    AF_INET as u32,
                    flags,
                    ptr::null_mut(),
                    buffer.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH,
                    &mut buf_len,
                )
            };
        }

        if ret != NO_ERROR {
            return Vec::new();
        }

        let mut results = Vec::new();
        let mut adapter_ptr = buffer.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;

        while !adapter_ptr.is_null() {
            let adapter = unsafe { &*adapter_ptr };
            if adapter.OperStatus == IF_OPER_STATUS_UP
                && adapter.IfType != IF_TYPE_SOFTWARE_LOOPBACK
            {
                let friendly_name = unsafe { pwstr_to_string(adapter.FriendlyName) };
                let description = unsafe { pwstr_to_string(adapter.Description) };
                let is_virtual = is_virtual_adapter_name(&friendly_name)
                    || is_virtual_adapter_name(&description);

                let mut unicast_ptr = adapter.FirstUnicastAddress;
                while !unicast_ptr.is_null() {
                    let unicast = unsafe { &*unicast_ptr };
                    let sockaddr = unicast.Address.lpSockaddr;
                    if !sockaddr.is_null() && unsafe { (*sockaddr).sa_family } == AF_INET {
                        let sin = unsafe { &*(sockaddr as *const SOCKADDR_IN) };
                        let s_addr = unsafe { sin.sin_addr.S_un.S_addr };
                        let local_ip = Ipv4Addr::from(s_addr.to_ne_bytes());

                        // 排除 Loopback 與 APIPA (169.254.x.x)
                        if !local_ip.is_loopback() && !local_ip.is_link_local() {
                            let prefix_len = unicast.OnLinkPrefixLength.clamp(1, 32);
                            let (network_ip, broadcast_ip) =
                                ipv4_network_and_broadcast(local_ip, prefix_len);
                            let cidr = format!("{network_ip}/{prefix_len}");
                            results.push(LocalSubnet {
                                adapter_name: if friendly_name.is_empty() {
                                    "區網介面".to_string()
                                } else {
                                    friendly_name.clone()
                                },
                                local_ip,
                                prefix_len,
                                network_ip,
                                broadcast_ip,
                                cidr,
                                is_virtual,
                            });
                        }
                    }
                    unicast_ptr = unicast.Next;
                }
            }
            adapter_ptr = adapter.Next;
        }

        results.sort_by_key(subnet_priority);
        results
    }

    /// 透過 Windows 原生 SendARP 取得同區網主機 MAC（免系統管理員權限）
    pub fn send_arp(ip: Ipv4Addr) -> Option<[u8; 6]> {
        let dest_ip = u32::from_ne_bytes(ip.octets());
        let mut mac_buf = [0u8; 8];
        let mut mac_len: u32 = 6;
        let status = unsafe {
            SendARP(
                dest_ip,
                0,
                mac_buf.as_mut_ptr() as *mut c_void,
                &mut mac_len,
            )
        };
        if status == NO_ERROR && mac_len == 6 {
            let mac = [
                mac_buf[0], mac_buf[1], mac_buf[2], mac_buf[3], mac_buf[4], mac_buf[5],
            ];
            if mac != [0u8; 6] {
                return Some(mac);
            }
        }
        None
    }

    /// 透過 Windows 原生 IcmpSendEcho 進行 ICMP Ping（免系統管理員權限、不開黑窗）
    pub fn icmp_ping(ip: Ipv4Addr, timeout_ms: u32) -> Option<u32> {
        let handle = unsafe { IcmpCreateFile() };
        if handle == INVALID_HANDLE_VALUE || handle.is_null() {
            return None;
        }

        let payload = b"WOL_PING";
        let reply_size = std::mem::size_of::<ICMP_ECHO_REPLY>() + payload.len() + 32;
        let mut reply_buf = vec![0u8; reply_size];
        let dest_ip = u32::from_ne_bytes(ip.octets());

        let count = unsafe {
            IcmpSendEcho(
                handle,
                dest_ip,
                payload.as_ptr() as *const c_void,
                payload.len() as u16,
                ptr::null(),
                reply_buf.as_mut_ptr() as *mut c_void,
                reply_size as u32,
                timeout_ms,
            )
        };

        unsafe {
            IcmpCloseHandle(handle);
        }

        if count > 0 {
            let reply = unsafe { &*(reply_buf.as_ptr() as *const ICMP_ECHO_REPLY) };
            if reply.Status == IP_SUCCESS {
                return Some(reply.RoundTripTime);
            }
        }
        None
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    pub fn detect_local_subnets() -> Vec<LocalSubnet> {
        Vec::new()
    }
    pub fn send_arp(_ip: Ipv4Addr) -> Option<[u8; 6]> {
        None
    }
    pub fn icmp_ping(_ip: Ipv4Addr, _timeout_ms: u32) -> Option<u32> {
        None
    }
}

pub use platform::{detect_local_subnets, icmp_ping, send_arp};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cidr_24() {
        let t = parse_subnet_spec("192.168.1.55/24").expect("valid /24");
        assert_eq!(t.normalized, "192.168.1.0/24");
        assert_eq!(t.mode, QueryMode::Subnet);
        assert_eq!(t.auto_narrowed_from, None);
        assert_eq!(t.hosts.len(), 254);
        assert_eq!(t.hosts.first(), Some(&Ipv4Addr::new(192, 168, 1, 1)));
        assert_eq!(t.hosts.last(), Some(&Ipv4Addr::new(192, 168, 1, 254)));
        assert_eq!(t.broadcast_ip, Ipv4Addr::new(192, 168, 1, 255));
    }

    #[test]
    fn parse_single_and_multiple_specific_ips() {
        let single = parse_subnet_spec("192.168.2.50").expect("single IP");
        assert_eq!(single.mode, QueryMode::SingleIp);
        assert_eq!(single.normalized, "192.168.2.50");
        assert_eq!(single.hosts, vec![Ipv4Addr::new(192, 168, 2, 50)]);
        assert!(single.report_offline());

        let single32 = parse_subnet_spec("192.168.2.50/32").expect("/32 single IP");
        assert_eq!(single32.mode, QueryMode::SingleIp);
        assert_eq!(single32.hosts, vec![Ipv4Addr::new(192, 168, 2, 50)]);

        let multi = parse_subnet_spec("192.168.2.10, 192.168.2.50").expect("multi IP");
        assert_eq!(multi.mode, QueryMode::SpecificIps);
        assert_eq!(
            multi.hosts,
            vec![Ipv4Addr::new(192, 168, 2, 10), Ipv4Addr::new(192, 168, 2, 50)]
        );
        assert!(multi.report_offline());
    }

    #[test]
    fn auto_narrow_unaligned_wide_cidr_and_dot_zero_subnet() {
        // 使用者輸入 192.168.2.0 -> 視為 192.168.2.0/24
        let dot_zero = parse_subnet_spec("192.168.2.0").expect("dot zero");
        assert_eq!(dot_zero.normalized, "192.168.2.0/24");
        assert_eq!(dot_zero.mode, QueryMode::Subnet);
        assert_eq!(dot_zero.hosts.len(), 254);

        // 使用者從 192.168.0.0/22 改輸入 192.168.2.0/22 -> 自動縮減為 192.168.2.0/24，不拉回 192.168.0.0/22
        let narrowed = parse_subnet_spec("192.168.2.0/22").expect("auto narrow /22");
        assert_eq!(narrowed.normalized, "192.168.2.0/24");
        assert_eq!(narrowed.auto_narrowed_from, Some(22));
        assert_eq!(narrowed.hosts.len(), 254);
        assert_eq!(narrowed.hosts.first(), Some(&Ipv4Addr::new(192, 168, 2, 1)));

        // 若明確輸入對齊的 192.168.0.0/22 -> 保持 /22
        let aligned = parse_subnet_spec("192.168.0.0/22").expect("aligned /22");
        assert_eq!(aligned.normalized, "192.168.0.0/22");
        assert_eq!(aligned.auto_narrowed_from, None);
        assert_eq!(aligned.hosts.len(), 1022);
    }

    #[test]
    fn parse_short_and_full_ranges() {
        let t1 = parse_subnet_spec("10.0.0.10-20").expect("short range");
        assert_eq!(t1.hosts.len(), 11);
        assert_eq!(t1.hosts[0], Ipv4Addr::new(10, 0, 0, 10));
        assert_eq!(t1.hosts[10], Ipv4Addr::new(10, 0, 0, 20));

        let t2 = parse_subnet_spec("192.168.5.").expect("prefix shorthand");
        assert_eq!(t2.normalized, "192.168.5.0/24");
        assert_eq!(t2.hosts.len(), 254);
    }

    #[test]
    fn reject_oversized_subnet() {
        let err = parse_subnet_spec("10.0.0.0/16").expect_err("must reject /16");
        assert!(err.contains("上限"));
    }

    #[test]
    fn parse_and_format_mac_variants() {
        let m1 = parse_mac("aa:bb:cc:dd:ee:ff").expect("colon");
        let m2 = parse_mac("AA-BB-CC-DD-EE-FF").expect("dash");
        let m3 = parse_mac("aabbccddeeff").expect("plain");
        assert_eq!(m1, [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        assert_eq!(m1, m2);
        assert_eq!(m2, m3);
        assert_eq!(format_mac(m1), "AA:BB:CC:DD:EE:FF");
        assert!(parse_mac("00:00:00:00:00:00").is_err());
        assert!(parse_mac("ZZ:BB:CC:DD:EE:FF").is_err());
    }

    #[test]
    fn magic_packet_layout_is_exact() {
        let mac = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC];
        let pkt = build_magic_packet(mac);
        assert_eq!(pkt.len(), 102);
        assert_eq!(&pkt[0..6], &[0xFF; 6]);
        for i in 0..16 {
            let start = 6 + i * 6;
            assert_eq!(&pkt[start..start + 6], &mac);
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_native_icmp_and_adapter_detection() {
        // 驗證 Windows 原生 IcmpSendEcho 可免管理員權限 Ping 本機
        let rtt = icmp_ping(Ipv4Addr::LOCALHOST, 1000);
        assert!(rtt.is_some(), "IcmpSendEcho(127.0.0.1) 應成功回應");

        // 驗證 Windows 原生 GetAdaptersAddresses 正常執行且預帶網段格式合法
        let subnets = detect_local_subnets();
        for s in &subnets {
            let pref = s.preferred_scan_cidr();
            assert!(
                parse_subnet_spec(&pref).is_ok(),
                "預帶 CIDR 應可解析: {pref}"
            );
        }
    }
}
