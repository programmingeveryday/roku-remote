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
                        let _ = tx.send(BackgroundMessage::DeviceDiscovered(RokuDevice {
                            ip,
                            name: "Roku Device".to_string(),
                        }));
                        ctx.request_repaint();
                    }
                }
            }
        }

        let probe_ips = vec!["192.168.0.108".to_string()];
        for ip in probe_ips {
            let target: Result<SocketAddr, _> = format!("{}:8060", ip).parse();
            if let Ok(addr) = target {
                if TcpStream::connect_timeout(&addr, Duration::from_millis(400)).is_ok() {
                    let _ = tx.send(BackgroundMessage::DeviceDiscovered(RokuDevice {
                        ip,
                        name: "Roku Streaming Stick Plus".to_string(),
                    }));
                    ctx.request_repaint();
                }
            }
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

pub fn load_app_icon_worker(ip: &str, app_id: &str, tx: &Sender<BackgroundMessage>, ctx: &egui::Context) {
    let url = format!("http://{}:8060/query/icon/{}", ip, app_id);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(1500))
        .build();
    if let Ok(c) = client {
        if let Ok(resp) = c.get(&url).send() {
            if resp.status().is_success() {
                if let Ok(bytes) = resp.bytes() {
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

pub fn update_apps_worker(ip: &str, tx: &Sender<BackgroundMessage>, ctx: &egui::Context) {
    let url = format!("http://{}:8060/query/apps", ip);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(1500))
        .build();
    if let Ok(c) = client {
        if let Ok(resp) = c.get(&url).send() {
            if let Ok(text) = resp.text() {
                let parsed = parse_apps_xml(&text);
                if !parsed.is_empty() {
                    let _ = tx.send(BackgroundMessage::AppsListUpdated(parsed));
                    ctx.request_repaint();
                }
            }
        }
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
    if !sent {
        let _ = tx.send(BackgroundMessage::AppsRefreshFailed);
        ctx.request_repaint();
    }
}

