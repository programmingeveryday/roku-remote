use eframe::egui;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use crate::models::{BackgroundMessage, MediaPlayerInfo, PairedTvInfo, RokuDevice, TvBrand};
use crate::roku::parser::{
    clean_html_entities, parse_active_app_xml, parse_apps_xml,
    parse_device_details_xml, parse_device_name_xml,
    parse_media_player_xml, parse_screensaver_xml,
};

fn get_local_subnet_base() -> Option<(u8, u8, u8)> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let ip = match socket.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(ipv4) => ipv4,
        _ => return None,
    };
    let oct = ip.octets();
    Some((oct[0], oct[1], oct[2]))
}

pub fn fetch_device_display_name(ip: &str) -> String {
    let url = format!("http://{}:8060/query/device-info", ip);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(1200))
        .build();
    if let Ok(c) = client {
        if let Ok(resp) = c.get(&url).send() {
            if resp.status().is_success() {
                if let Ok(text) = resp.text() {
                    let base_name = parse_device_name_xml(&text)
                        .map(|n| clean_html_entities(&n))
                        .unwrap_or_else(|| "Roku Device".to_string());
                    let details = parse_device_details_xml(&text);
                    if !details.user_location.is_empty() && !base_name.contains(&details.user_location) {
                        return format!("{} ({})", base_name, details.user_location);
                    } else {
                        return base_name;
                    }
                }
            }
        }
    }
    "Roku Device".to_string()
}

pub fn start_scan(tx: Sender<BackgroundMessage>, ctx: egui::Context) {
    thread::spawn(move || {
        let ssdp_msg = "M-SEARCH * HTTP/1.1\r\n\
HOST: 239.255.255.250:1900\r\n\
MAN: \"ssdp:discover\"\r\n\
MX: 2\r\n\
ST: roku:ecp\r\n\r\n";

        if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
            let _ = socket.set_read_timeout(Some(Duration::from_millis(1500)));
            let _ = socket.send_to(ssdp_msg.as_bytes(), "239.255.255.250:1900");

            let mut buf = [0u8; 1024];
            let start = Instant::now();
            while start.elapsed() < Duration::from_millis(1500) {
                if let Ok((len, addr)) = socket.recv_from(&mut buf) {
                    let text = String::from_utf8_lossy(&buf[..len]);
                    if text.to_lowercase().contains("roku") {
                        let ip = addr.ip().to_string();
                        let name = fetch_device_display_name(&ip);
                        let _ = tx.send(BackgroundMessage::DeviceDiscovered(RokuDevice {
                            ip,
                            name,
                        }));
                        ctx.request_repaint();
                    }
                }
            }
        }

        // Fast multi-threaded subnet scan to discover Rokus even when multicast/SSDP is dropped by the router
        if let Some((o1, o2, o3)) = get_local_subnet_base() {
            let chunks: Vec<Vec<u8>> = (1..=254)
                .collect::<Vec<u8>>()
                .chunks(8)
                .map(|c| c.to_vec())
                .collect();

            thread::scope(|s| {
                for chunk in chunks {
                    let tx_ref = &tx;
                    let ctx_ref = &ctx;
                    s.spawn(move || {
                        for i in chunk {
                            let ip = format!("{}.{}.{}.{}", o1, o2, o3, i);
                            let mut is_roku = false;
                            if let Ok(addr) = format!("{}:8060", ip).parse::<SocketAddr>() {
                                if TcpStream::connect_timeout(&addr, Duration::from_millis(220)).is_ok() {
                                    is_roku = true;
                                    let name = fetch_device_display_name(&ip);
                                    let _ = tx_ref.send(BackgroundMessage::DeviceDiscovered(RokuDevice {
                                        ip: ip.clone(),
                                        name,
                                    }));
                                    ctx_ref.request_repaint();
                                }
                            }
                            if !is_roku {
                                if let Some(tv_info) = probe_smart_tv_at_ip(&ip) {
                                    let _ = tx_ref.send(BackgroundMessage::SmartTvDiscovered(tv_info));
                                    ctx_ref.request_repaint();
                                }
                            }
                        }
                    });
                }
            });
        }

        let _ = tx.send(BackgroundMessage::ScanFinished);
        ctx.request_repaint();
    });
}

pub fn update_device_name_worker(ip: &str, tx: &Sender<BackgroundMessage>, ctx: &egui::Context) {
    let url = format!("http://{}:8060/query/device-info", ip);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(1500))
        .build();
    let mut success = false;
    if let Ok(c) = client {
        if let Ok(resp) = c.get(&url).send() {
            if resp.status().is_success() {
                if let Ok(text) = resp.text() {
                    success = true;
                    if let Some(name) = parse_device_name_xml(&text) {
                        let cleaned = clean_html_entities(&name);
                        let _ = tx.send(BackgroundMessage::DeviceNameUpdated(cleaned));
                    }
                    let details = parse_device_details_xml(&text);
                    let _ = tx.send(BackgroundMessage::DeviceDetailsUpdated(details));
                    let _ = tx.send(BackgroundMessage::PowerStateUpdated(true));
                    ctx.request_repaint();
                }
            }
        }
    }
    if !success {
        let _ = tx.send(BackgroundMessage::PowerStateUpdated(false));
        ctx.request_repaint();
    }
}

pub fn update_media_player_worker(ip: &str, tx: &Sender<BackgroundMessage>, ctx: &egui::Context) {
    let url = format!("http://{}:8060/query/media-player", ip);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(1500))
        .build();
    if let Ok(c) = client {
        if let Ok(resp) = c.get(&url).send() {
            if let Ok(text) = resp.text() {
                let info = parse_media_player_xml(&text);
                let _ = tx.send(BackgroundMessage::MediaPlayerUpdated(info));
                ctx.request_repaint();
            }
        }
    }
}

pub fn get_icon_cache_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let cache_dir = std::path::PathBuf::from(home).join(".cache/roku-remote-rs/icons");
    let _ = std::fs::create_dir_all(&cache_dir);
    cache_dir
}

pub fn load_app_icon_worker(ip: &str, app_id: &str, tx: &Sender<BackgroundMessage>, ctx: &egui::Context) {
    let cache_file = get_icon_cache_dir().join(format!("{}.img", app_id));

    // 1. Try loading cached icon from disk first
    if let Ok(bytes) = std::fs::read(&cache_file) {
        if let Ok(img) = image::load_from_memory(&bytes) {
            let rgba = img.to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            let pixels = rgba.into_raw();
            let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
            let _ = tx.send(BackgroundMessage::AppIconLoaded {
                id: app_id.to_string(),
                image: color_image,
            });
            ctx.request_repaint();
            return;
        }
    }

    // 2. If not cached or if reading failed, fetch from device
    if ip.is_empty() {
        return;
    }
    let url = format!("http://{}:8060/query/icon/{}", ip, app_id);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(1500))
        .build();
    if let Ok(c) = client {
        if let Ok(resp) = c.get(&url).send() {
            if resp.status().is_success() {
                if let Ok(bytes) = resp.bytes() {
                    // Persist to disk cache
                    let _ = std::fs::write(&cache_file, &bytes);

                    if let Ok(img) = image::load_from_memory(&bytes) {
                        let rgba = img.to_rgba8();
                        let size = [rgba.width() as usize, rgba.height() as usize];
                        let pixels = rgba.into_raw();
                        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
                        let _ = tx.send(BackgroundMessage::AppIconLoaded {
                            id: app_id.to_string(),
                            image: color_image,
                        });
                        ctx.request_repaint();
                    }
                }
            }
        }
    }
}

pub fn update_active_app_worker(ip: &str, tx: &Sender<BackgroundMessage>, ctx: &egui::Context) {
    let url = format!("http://{}:8060/query/active-app", ip);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(1500))
        .build();
    if let Ok(c) = client {
        if let Ok(resp) = c.get(&url).send() {
            if let Ok(text) = resp.text() {
                if let Some(app_name) = parse_active_app_xml(&text) {
                    let cleaned = clean_html_entities(&app_name);
                    let _ = tx.send(BackgroundMessage::ActiveAppUpdated(cleaned));
                }
                let ssvr = parse_screensaver_xml(&text);
                let _ = tx.send(BackgroundMessage::ScreensaverUpdated(ssvr));
                ctx.request_repaint();
            }
        }
    }
}

pub fn save_cached_apps(apps: &[crate::models::AppItem]) {
    let cache_file = get_icon_cache_dir().join("apps_cache.json");
    let mut json = String::from("[\n");
    for (i, app) in apps.iter().enumerate() {
        let name_escaped = app.name.replace('\\', "\\\\").replace('"', "\\\"");
        let id_escaped = app.id.replace('\\', "\\\\").replace('"', "\\\"");
        json.push_str(&format!(
            "  {{\"id\": \"{}\", \"name\": \"{}\"}}{}",
            id_escaped,
            name_escaped,
            if i + 1 < apps.len() { ",\n" } else { "\n" }
        ));
    }
    json.push(']');
    let _ = std::fs::write(cache_file, json);
}

pub fn load_cached_apps() -> Option<Vec<crate::models::AppItem>> {
    let cache_file = get_icon_cache_dir().join("apps_cache.json");
    let content = std::fs::read_to_string(cache_file).ok()?;
    let mut apps = Vec::new();

    // Simple robust JSON parser for app array
    for line in content.lines() {
        if let (Some(id_start), Some(name_start)) = (line.find("\"id\": \""), line.find("\"name\": \"")) {
            let id_val = &line[id_start + 7..];
            if let Some(id_end) = id_val.find('"') {
                let id = id_val[..id_end].to_string();
                let name_val = &line[name_start + 9..];
                if let Some(name_end) = name_val.find('"') {
                    let name = name_val[..name_end].to_string();
                    apps.push(crate::models::AppItem { id, name });
                }
            }
        }
    }

    if apps.is_empty() {
        None
    } else {
        Some(apps)
    }
}

pub fn refresh_apps_worker(ip: &str, tx: &Sender<BackgroundMessage>, ctx: &egui::Context) {
    if ip.is_empty() {
        let _ = tx.send(BackgroundMessage::AppsRefreshFailed);
        return;
    }
    let url = format!("http://{}:8060/query/apps", ip);
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(2500))
        .build()
    {
        Ok(c) => c,
        Err(_) => {
            let _ = tx.send(BackgroundMessage::AppsRefreshFailed);
            return;
        }
    };

    if let Ok(resp) = client.get(&url).send() {
        if resp.status().is_success() {
            if let Ok(text) = resp.text() {
                let apps = parse_apps_xml(&text);
                if !apps.is_empty() {
                    save_cached_apps(&apps);
                    let _ = tx.send(BackgroundMessage::AppsListRefreshed(apps));
                    ctx.request_repaint();
                    return;
                }
            }
        }
    }
    let _ = tx.send(BackgroundMessage::AppsRefreshFailed);
    ctx.request_repaint();
}

pub fn load_cached_last_ip() -> Option<String> {
    let cache_file = get_icon_cache_dir().join("last_ip.txt");
    std::fs::read_to_string(cache_file)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn save_cached_last_ip(ip: &str) {
    if !ip.is_empty() {
        let cache_file = get_icon_cache_dir().join("last_ip.txt");
        let _ = std::fs::write(cache_file, ip);
    }
}

pub fn load_cached_paired_tv() -> PairedTvInfo {
    let paired_file = get_icon_cache_dir().join("paired_tv.txt");
    if let Ok(raw) = std::fs::read_to_string(&paired_file) {
        let parts: Vec<&str> = raw.trim().splitn(3, '|').collect();
        if parts.len() == 3 {
            if let Some(brand) = TvBrand::from_str(parts[0]) {
                let ip = parts[1].trim().to_string();
                let name = parts[2].trim().to_string();
                if !ip.is_empty() {
                    return PairedTvInfo { brand, ip, name };
                }
            }
        }
    }

    // Fallback to legacy samsung_tv_ip.txt or default Livingroom Samsung TV
    let legacy_file = get_icon_cache_dir().join("samsung_tv_ip.txt");
    let ip = std::fs::read_to_string(legacy_file)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "192.168.0.212".to_string());

    PairedTvInfo {
        brand: TvBrand::Samsung,
        ip,
        name: "[TV] Livingroom TV".to_string(),
    }
}

pub fn save_cached_paired_tv(tv: &PairedTvInfo) {
    if !tv.ip.is_empty() {
        let paired_file = get_icon_cache_dir().join("paired_tv.txt");
        let content = format!("{}|{}|{}", tv.brand.as_str(), tv.ip, tv.name);
        let _ = std::fs::write(paired_file, content);
        if tv.brand == TvBrand::Samsung {
            let legacy_file = get_icon_cache_dir().join("samsung_tv_ip.txt");
            let _ = std::fs::write(legacy_file, &tv.ip);
        }
    }
}

fn extract_json_string_field(json: &str, key: &str) -> Option<String> {
    for pattern in [format!("\"{}\":\"", key), format!("\"{}\": \"", key)] {
        if let Some(start) = json.find(&pattern) {
            let sub = &json[start + pattern.len()..];
            if let Some(end) = sub.find('"') {
                let val = sub[..end].trim();
                if !val.is_empty() {
                    return Some(val.to_string());
                }
            }
        }
    }
    None
}

pub fn parse_samsung_power_state(json: &str) -> Option<bool> {
    if let Some(state) = extract_json_string_field(json, "PowerState") {
        return Some(state.eq_ignore_ascii_case("on"));
    }
    if json.contains("\"id\":") && json.contains("\"device\":") {
        // Older Samsung Tizen models that omit PowerState only respond on port 8001 while awake
        return Some(true);
    }
    None
}

pub fn parse_samsung_tv_name(json: &str) -> Option<String> {
    extract_json_string_field(json, "name")
        .or_else(|| extract_json_string_field(json, "modelName"))
}

pub fn parse_sony_power_status(json: &str) -> Option<bool> {
    if let Some(status) = extract_json_string_field(json, "status") {
        if status.eq_ignore_ascii_case("active") {
            return Some(true);
        }
        if status.eq_ignore_ascii_case("standby") || status.eq_ignore_ascii_case("off") {
            return Some(false);
        }
    }
    None
}

pub fn parse_sony_model_name(json: &str) -> Option<String> {
    if !json.contains("BRAVIA") && !json.contains("sony") && !json.contains("InterfaceVersion") {
        return None;
    }
    if let Some(model) = extract_json_string_field(json, "modelName") {
        return Some(format!("Sony BRAVIA ({})", model));
    }
    Some("Sony BRAVIA TV".to_string())
}

pub fn parse_lg_power_state(payload: &str) -> Option<bool> {
    let lower = payload.to_lowercase();
    if lower.contains("\"screen off\"")
        || lower.contains("\"active standby\"")
        || lower.contains("\"standby\"")
        || lower.contains("\"power off\"")
        || lower.contains("\"suspend\"")
    {
        return Some(false);
    }
    if lower.contains("\"active\"") || lower.contains("101 switching protocols") {
        return Some(true);
    }
    None
}

pub fn parse_vizio_power_mode(json: &str) -> Option<bool> {
    let compact: String = json.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.contains("\"VALUE\":1") {
        return Some(true);
    }
    if compact.contains("\"VALUE\":0") {
        return Some(false);
    }
    None
}

fn check_samsung_tv_power(tv_ip: &str, tx: &Sender<BackgroundMessage>) -> bool {
    let url = format!("http://{}:8001/api/v2/", tv_ip);
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    if let Ok(resp) = client.get(&url).send() {
        if resp.status().is_success() {
            if let Ok(text) = resp.text() {
                if let Some(name) = parse_samsung_tv_name(&text) {
                    let _ = tx.send(BackgroundMessage::SmartTvDiscovered(PairedTvInfo {
                        brand: TvBrand::Samsung,
                        ip: tv_ip.to_string(),
                        name,
                    }));
                }
                return parse_samsung_power_state(&text).unwrap_or(false);
            }
        }
    }
    false
}

fn check_sony_tv_power(tv_ip: &str) -> bool {
    let url = format!("http://{}/sony/system", tv_ip);
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    let body = r#"{"method":"getPowerStatus","id":1,"params":[],"version":"1.0"}"#;
    if let Ok(resp) = client
        .post(&url)
        .header("Content-Type", "application/json")
        .body(body)
        .send()
    {
        if resp.status().is_success() {
            if let Ok(text) = resp.text() {
                return parse_sony_power_status(&text).unwrap_or(false);
            }
        }
    }
    false
}

/// Builds a masked RFC 6455 WebSocket text frame for sending small JSON payloads over raw TCP.
fn build_masked_ws_frame(payload: &[u8]) -> Vec<u8> {
    let mask_key: [u8; 4] = [0x37, 0xFA, 0x21, 0x3D];
    let mut frame = Vec::with_capacity(10 + payload.len());
    frame.push(0x81); // FIN + Text opcode
    if payload.len() < 126 {
        frame.push(0x80 | (payload.len() as u8));
    } else {
        frame.push(0x80 | 126);
        let len_bytes = (payload.len() as u16).to_be_bytes();
        frame.extend_from_slice(&len_bytes);
    }
    frame.extend_from_slice(&mask_key);
    for (i, &b) in payload.iter().enumerate() {
        frame.push(b ^ mask_key[i % 4]);
    }
    frame
}

fn check_lg_tv_power(tv_ip: &str) -> bool {
    // LG webOS listens on TCP port 3000 (WS) and 3001 (WSS) while the TV is On,
    // and closes the port when entering Standby / Power Off.
    if let Ok(addr) = format!("{}:3000", tv_ip).parse::<SocketAddr>() {
        if let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(650)) {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(450)));
            let _ = stream.set_write_timeout(Some(Duration::from_millis(450)));
            let upgrade = format!(
                "GET / HTTP/1.1\r\n\
                 Host: {}:3000\r\n\
                 Upgrade: websocket\r\n\
                 Connection: Upgrade\r\n\
                 Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
                 Sec-WebSocket-Version: 13\r\n\r\n",
                tv_ip
            );
            if stream.write_all(upgrade.as_bytes()).is_ok() {
                let mut buf = [0u8; 2048];
                if let Ok(n) = stream.read(&mut buf) {
                    let resp = String::from_utf8_lossy(&buf[..n]);
                    if resp.contains("101") {
                        // Query SSAP power state in case OLED "Screen Off" / "Active Standby" is active
                        let ssap_req = br#"{"type":"request","id":"pwr_1","uri":"ssap://com.webos.service.tvpower/power/getPowerState"}"#;
                        let frame = build_masked_ws_frame(ssap_req);
                        if stream.write_all(&frame).is_ok() {
                            if let Ok(n2) = stream.read(&mut buf) {
                                let ws_text = String::from_utf8_lossy(&buf[..n2]);
                                if let Some(state) = parse_lg_power_state(&ws_text) {
                                    return state;
                                }
                            }
                        }
                        return true;
                    }
                }
            }
            return true;
        }
    }
    // Fallback to TLS port 3001 reachability for newer webOS firmware that disables plain port 3000
    if let Ok(addr) = format!("{}:3001", tv_ip).parse::<SocketAddr>() {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(550)).is_ok() {
            return true;
        }
    }
    false
}

fn check_vizio_tv_power(tv_ip: &str) -> bool {
    let client = match reqwest::blocking::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_millis(800))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    for port in [7345u16, 9000u16] {
        let url = format!("https://{}:{}/state/device/power_mode", tv_ip, port);
        if let Ok(resp) = client.get(&url).send() {
            if let Ok(text) = resp.text() {
                if let Some(is_on) = parse_vizio_power_mode(&text) {
                    return is_on;
                }
                // If Vizio SmartCast responds with PAIRING_DENIED / REQUIRES_PAIRING,
                // the SmartCast daemon is awake and running (in Eco Mode, Vizio shuts down port 7345 when off)
                if text.contains("PAIRING_DENIED") || text.contains("STATUS") {
                    return true;
                }
            }
        }
    }
    false
}

pub fn check_smart_tv_power_worker(
    tv: &PairedTvInfo,
    tx: &Sender<BackgroundMessage>,
    ctx: &egui::Context,
) {
    if tv.ip.is_empty() {
        return;
    }
    let is_on = match tv.brand {
        TvBrand::Samsung => check_samsung_tv_power(&tv.ip, tx),
        TvBrand::Sony => check_sony_tv_power(&tv.ip),
        TvBrand::Lg => check_lg_tv_power(&tv.ip),
        TvBrand::Vizio => check_vizio_tv_power(&tv.ip),
    };
    let _ = tx.send(BackgroundMessage::TvPowerStateUpdated(is_on));
    ctx.request_repaint();
}

/// Probes a single IP address for a supported Smart TV (Samsung, LG, Vizio, or Sony).
pub fn probe_smart_tv_at_ip(ip: &str) -> Option<PairedTvInfo> {
    let timeout = Duration::from_millis(160);

    // 1. Samsung Smart TV (Tizen REST API on port 8001)
    if let Ok(addr) = format!("{}:8001", ip).parse::<SocketAddr>() {
        if TcpStream::connect_timeout(&addr, timeout).is_ok() {
            if let Ok(client) = reqwest::blocking::Client::builder()
                .timeout(Duration::from_millis(700))
                .build()
            {
                let url = format!("http://{}:8001/api/v2/", ip);
                if let Ok(resp) = client.get(&url).send() {
                    if let Ok(text) = resp.text() {
                        if text.contains("PowerState") || text.contains("modelName") {
                            let name = parse_samsung_tv_name(&text)
                                .unwrap_or_else(|| "Samsung Smart TV".to_string());
                            return Some(PairedTvInfo {
                                brand: TvBrand::Samsung,
                                ip: ip.to_string(),
                                name,
                            });
                        }
                    }
                }
            }
        }
    }

    // 2. LG webOS TV (SSAP WebSocket server on port 3000)
    if let Ok(addr) = format!("{}:3000", ip).parse::<SocketAddr>() {
        if let Ok(mut stream) = TcpStream::connect_timeout(&addr, timeout) {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(450)));
            let _ = stream.set_write_timeout(Some(Duration::from_millis(450)));
            let upgrade = format!(
                "GET / HTTP/1.1\r\n\
                 Host: {}:3000\r\n\
                 Upgrade: websocket\r\n\
                 Connection: Upgrade\r\n\
                 Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
                 Sec-WebSocket-Version: 13\r\n\r\n",
                ip
            );
            if stream.write_all(upgrade.as_bytes()).is_ok() {
                let mut buf = [0u8; 1024];
                if let Ok(n) = stream.read(&mut buf) {
                    let resp = String::from_utf8_lossy(&buf[..n]);
                    if resp.contains("101") && resp.to_lowercase().contains("websocket") {
                        return Some(PairedTvInfo {
                            brand: TvBrand::Lg,
                            ip: ip.to_string(),
                            name: "LG webOS TV".to_string(),
                        });
                    }
                }
            }
        }
    }

    // 3. Vizio SmartCast TV (HTTPS API on port 7345)
    if let Ok(addr) = format!("{}:7345", ip).parse::<SocketAddr>() {
        if TcpStream::connect_timeout(&addr, timeout).is_ok() {
            if let Ok(client) = reqwest::blocking::Client::builder()
                .danger_accept_invalid_certs(true)
                .timeout(Duration::from_millis(700))
                .build()
            {
                let mut friendly_name = "Vizio SmartCast TV".to_string();
                let desc_url = format!("http://{}:8008/ssdp/device-desc.xml", ip);
                if let Ok(resp) = client.get(&desc_url).send() {
                    if let Ok(xml) = resp.text() {
                        if let (Some(s), Some(e)) = (xml.find("<friendlyName>"), xml.find("</friendlyName>")) {
                            if e > s + 14 {
                                friendly_name = xml[s + 14..e].trim().to_string();
                            }
                        }
                    }
                }
                return Some(PairedTvInfo {
                    brand: TvBrand::Vizio,
                    ip: ip.to_string(),
                    name: friendly_name,
                });
            }
        }
    }

    // 4. Sony Bravia TV (JSON-RPC on port 80 /sony/system)
    if let Ok(addr) = format!("{}:80", ip).parse::<SocketAddr>() {
        if TcpStream::connect_timeout(&addr, timeout).is_ok() {
            if let Ok(client) = reqwest::blocking::Client::builder()
                .timeout(Duration::from_millis(700))
                .build()
            {
                let url = format!("http://{}/sony/system", ip);
                let body = r#"{"method":"getInterfaceInformation","id":1,"params":[],"version":"1.0"}"#;
                if let Ok(resp) = client
                    .post(&url)
                    .header("Content-Type", "application/json")
                    .body(body)
                    .send()
                {
                    if resp.status().is_success() {
                        if let Ok(text) = resp.text() {
                            if let Some(name) = parse_sony_model_name(&text) {
                                return Some(PairedTvInfo {
                                    brand: TvBrand::Sony,
                                    ip: ip.to_string(),
                                    name,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

pub fn fetch_device_stats_worker(
    ip: &str,
    tx: &Sender<BackgroundMessage>,
    ctx: &egui::Context,
) {
    if ip.is_empty() {
        return;
    }
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(1500))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };

    let get_xml = |endpoint: &str| -> String {
        let url = format!("http://{}:8060/query/{}", ip, endpoint);
        client.get(&url).send().ok().and_then(|r| r.text().ok()).unwrap_or_default()
    };

    let dev_xml = get_xml("device-info");
    let device_details = if !dev_xml.is_empty() {
        Some(parse_device_details_xml(&dev_xml))
    } else {
        None
    };

    // Multi-sample polling algorithm:
    // When playback transitions between segments or streams start, Roku may temporarily
    // report unmetered segments (missing resolution / bitrate). Query media-player up to 3 times
    // spaced ~160ms apart, merging the best metrics and breaking as soon as complete telemetry is read.
    let mut best_media_player: Option<MediaPlayerInfo> = None;
    for attempt in 0..3 {
        let media_xml = get_xml("media-player");
        if !media_xml.is_empty() {
            let parsed = parse_media_player_xml(&media_xml);
            let has_res = !parsed.video_res.is_empty() && parsed.video_res != "0" && parsed.video_res != "0x0";
            let has_bitrate = parsed.video_bitrate_bps.map_or(false, |b| b > 0);
            let is_playing = parsed.state == "play";

            if let Some(ref mut best) = best_media_player {
                if has_res {
                    best.video_res = parsed.video_res.clone();
                }
                if has_bitrate {
                    best.video_bitrate_bps = parsed.video_bitrate_bps;
                }
                if parsed.bandwidth_bps.map_or(false, |b| b > 0) {
                    best.bandwidth_bps = parsed.bandwidth_bps;
                }
                if !parsed.video_codec.is_empty() {
                    best.video_codec = parsed.video_codec.clone();
                }
                if !parsed.audio_codec.is_empty() {
                    best.audio_codec = parsed.audio_codec.clone();
                }
                if !parsed.container.is_empty() {
                    best.container = parsed.container.clone();
                }
                if !parsed.captions.is_empty() {
                    best.captions = parsed.captions.clone();
                }
                if !parsed.drm.is_empty() {
                    best.drm = parsed.drm.clone();
                }
                if parsed.buffer_current.is_some() {
                    best.buffer_current = parsed.buffer_current;
                }
                if parsed.buffer_max.is_some() {
                    best.buffer_max = parsed.buffer_max;
                }
                if parsed.position_ms.is_some() {
                    best.position_ms = parsed.position_ms;
                }
                if parsed.duration_ms.is_some() {
                    best.duration_ms = parsed.duration_ms;
                }
                best.state = parsed.state.clone();
                best.is_live = parsed.is_live;
            } else {
                best_media_player = Some(parsed);
            }

            // If we have full stream diagnostics (resolution + bitrate) while playing, we have high fidelity
            if is_playing && has_res && has_bitrate {
                break;
            }
        }
        if attempt < 2 {
            thread::sleep(Duration::from_millis(160));
        }
    }

    let _ = tx.send(BackgroundMessage::DeviceStatsUpdated {
        media_player: best_media_player,
        device_details,
    });
    ctx.request_repaint();
}

/// Encodes a character for the Roku ECP `/keypress/Lit_<char>` command.
/// Alphanumeric ASCII and unreserved characters are passed as-is.
/// Spaces, punctuation, and UTF-8 multi-byte characters are URL percent-encoded.
pub fn encode_char_for_lit(c: char) -> String {
    if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
        format!("Lit_{}", c)
    } else {
        let mut buf = [0u8; 4];
        let s = c.encode_utf8(&mut buf);
        let mut encoded = String::new();
        for b in s.as_bytes() {
            encoded.push_str(&format!("%{:02X}", b));
        }
        format!("Lit_{}", encoded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_char_for_lit_alphanumeric() {
        assert_eq!(encode_char_for_lit('a'), "Lit_a");
        assert_eq!(encode_char_for_lit('Z'), "Lit_Z");
        assert_eq!(encode_char_for_lit('5'), "Lit_5");
    }

    #[test]
    fn test_encode_char_for_lit_special() {
        assert_eq!(encode_char_for_lit(' '), "Lit_%20");
        assert_eq!(encode_char_for_lit('@'), "Lit_%40");
        assert_eq!(encode_char_for_lit('/'), "Lit_%2F");
        assert_eq!(encode_char_for_lit('?'), "Lit_%3F");
        assert_eq!(encode_char_for_lit('#'), "Lit_%23");
        assert_eq!(encode_char_for_lit('&'), "Lit_%26");
        assert_eq!(encode_char_for_lit('-'), "Lit_-");
        assert_eq!(encode_char_for_lit('_'), "Lit__");
    }

    #[test]
    fn test_encode_char_for_lit_unicode() {
        assert_eq!(encode_char_for_lit('€'), "Lit_%E2%82%AC");
        assert_eq!(encode_char_for_lit('ñ'), "Lit_%C3%B1");
    }

    #[test]
    fn test_parse_samsung_tv_power_state() {
        let on_payload = r#"{"device":{"PowerState":"on","modelName":"UN75TU700DFXZA","name":"[TV] Livingroom TV"}}"#;
        assert_eq!(parse_samsung_power_state(on_payload), Some(true));
        assert_eq!(
            parse_samsung_tv_name(on_payload),
            Some("[TV] Livingroom TV".to_string())
        );

        let standby_payload = r#"{"device":{"PowerState":"standby","modelName":"UN75TU700DFXZA"}}"#;
        assert_eq!(parse_samsung_power_state(standby_payload), Some(false));
        assert_eq!(
            parse_samsung_tv_name(standby_payload),
            Some("UN75TU700DFXZA".to_string())
        );
    }

    #[test]
    fn test_parse_sony_bravia_power_status() {
        let active_json = r#"{"result":[{"status":"active"}],"id":1}"#;
        assert_eq!(parse_sony_power_status(active_json), Some(true));

        let standby_json = r#"{"result":[{"status":"standby"}],"id":1}"#;
        assert_eq!(parse_sony_power_status(standby_json), Some(false));

        let info_json = r#"{"result":[{"productCategory":"tv","productName":"BRAVIA","modelName":"XR-65A80K","InterfaceVersion":"7.0.0"}],"id":1}"#;
        assert_eq!(
            parse_sony_model_name(info_json),
            Some("Sony BRAVIA (XR-65A80K)".to_string())
        );
    }

    #[test]
    fn test_parse_lg_webos_power_state() {
        let active_ssap = r#"{"type":"response","id":"pwr_1","payload":{"state":"Active","returnValue":true}}"#;
        assert_eq!(parse_lg_power_state(active_ssap), Some(true));

        let screen_off_ssap = r#"{"type":"response","id":"pwr_1","payload":{"state":"Screen Off","returnValue":true}}"#;
        assert_eq!(parse_lg_power_state(screen_off_ssap), Some(false));

        let standby_ssap = r#"{"type":"response","id":"pwr_1","payload":{"state":"Active Standby","returnValue":true}}"#;
        assert_eq!(parse_lg_power_state(standby_ssap), Some(false));
    }

    #[test]
    fn test_parse_vizio_smartcast_power_mode() {
        let on_json = r#"{"STATUS":{"RESULT":"SUCCESS","DETAIL":"Success"},"ITEMS":[{"CNAME":"power_mode","TYPE":"T_VALUE_V1","NAME":"Power Mode","VALUE":1}]}"#;
        assert_eq!(parse_vizio_power_mode(on_json), Some(true));

        let off_json = r#"{"STATUS":{"RESULT":"SUCCESS","DETAIL":"Success"},"ITEMS":[{"CNAME":"power_mode","TYPE":"T_VALUE_V1","NAME":"Power Mode","VALUE":0}]}"#;
        assert_eq!(parse_vizio_power_mode(off_json), Some(false));
    }
}
