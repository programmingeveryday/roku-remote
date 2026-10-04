use eframe::egui;
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use crate::models::{BackgroundMessage, RokuDevice};
use crate::roku::parser::{
    clean_html_entities, parse_active_app_xml, parse_apps_xml, parse_device_details_xml,
    parse_device_name_xml, parse_media_player_xml,
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
                            if let Ok(addr) = format!("{}:8060", ip).parse::<SocketAddr>() {
                                if TcpStream::connect_timeout(&addr, Duration::from_millis(220)).is_ok() {
                                    let name = fetch_device_display_name(&ip);
                                    let _ = tx_ref.send(BackgroundMessage::DeviceDiscovered(RokuDevice {
                                        ip,
                                        name,
                                    }));
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
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
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
                    ctx.request_repaint();
                }
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
    let mut items = Vec::new();
    let mut rest = content.as_str();
    while let Some(start) = rest.find("{\"id\": \"") {
        let after_id = &rest[start + 8..];
        if let Some(id_end) = after_id.find('"') {
            let id = after_id[..id_end].to_string();
            if let Some(name_start) = after_id[id_end..].find("\"name\": \"") {
                let after_name = &after_id[id_end + name_start + 9..];
                if let Some(name_end) = after_name.find('"') {
                    let name = after_name[..name_end].to_string();
                    items.push(crate::models::AppItem { id, name });
                    rest = &after_name[name_end + 1..];
                    continue;
                }
            }
        }
        break;
    }
    if !items.is_empty() {
        Some(items)
    } else {
        None
    }
}

pub fn update_apps_worker(ip: &str, tx: &Sender<BackgroundMessage>, ctx: &egui::Context) {
    let url = format!("http://{}:8060/query/apps", ip);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(1500))
        .build();
    let mut sent = false;
    if let Ok(c) = client {
        if let Ok(resp) = c.get(&url).send() {
            if resp.status().is_success() {
                if let Ok(text) = resp.text() {
                    let parsed = parse_apps_xml(&text);
                    if !parsed.is_empty() {
                        let _ = tx.send(BackgroundMessage::AppsListUpdated(parsed));
                        ctx.request_repaint();
                        sent = true;
                    }
                }
            }
        }
    }
    if !sent {
        let _ = tx.send(BackgroundMessage::AppsRefreshFailed);
        ctx.request_repaint();
    }
}

pub fn refresh_apps_worker(ip: &str, tx: &Sender<BackgroundMessage>, ctx: &egui::Context) {
    let url = format!("http://{}:8060/query/apps", ip);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(2000))
        .build();
    let mut sent = false;
    if let Ok(c) = client {
        if let Ok(resp) = c.get(&url).send() {
            if resp.status().is_success() {
                if let Ok(text) = resp.text() {
                    let parsed = parse_apps_xml(&text);
                    if !parsed.is_empty() {
                        let _ = tx.send(BackgroundMessage::AppsListRefreshed(parsed));
                        ctx.request_repaint();
                        sent = true;
                    }
                }
            }
        }
    }
    if !sent {
        let _ = tx.send(BackgroundMessage::AppsRefreshFailed);
        ctx.request_repaint();
    }
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


