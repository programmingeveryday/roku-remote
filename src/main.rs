use eframe::egui;
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct RokuDevice {
    pub ip: String,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct AppItem {
    pub name: String,
    pub id: String,
}

enum BackgroundMessage {
    DeviceDiscovered(RokuDevice),
    ActiveAppUpdated(String),
    AppsListUpdated(Vec<AppItem>),
    ScanFinished,
}

struct RokuRemoteApp {
    devices: Vec<RokuDevice>,
    selected_device_ip: String,
    active_app: String,
    apps: Vec<AppItem>,
    is_scanning: bool,
    status_text: String,
    rx: Receiver<BackgroundMessage>,
    tx: Sender<BackgroundMessage>,
}

impl RokuRemoteApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Set custom styling
        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 8.0);
        cc.egui_ctx.set_style(style);

        let (tx, rx) = channel();
        let app = Self {
            devices: Vec::new(),
            selected_device_ip: "192.168.0.108".to_string(),
            active_app: "Unknown".to_string(),
            apps: default_popular_apps(),
            is_scanning: false,
            status_text: "Ready".to_string(),
            rx,
            tx,
        };

        // Initial scan and load
        app.start_scan();
        app.refresh_device_info();

        app
    }

    fn start_scan(&self) {
        let tx = self.tx.clone();
        thread::spawn(move || {
            // 1. Try SSDP M-SEARCH broadcast (Standard Roku UPnP discovery)
            let ssdp_msg = "M-SEARCH * HTTP/1.1\r\n\
                HOST: 239.255.255.250:1900\r\n\
                MAN: \"ssdp:discover\"\r\n\
                MX: 2\r\n\
                ST: roku:ecp\r\n\r\n";

            if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
                let _ = socket.set_read_timeout(Some(Duration::from_millis(1500)));
                let _ = socket.send_to(ssdp_msg.as_bytes(), "239.255.255.250:1900");

                let mut buf = [0u8; 1024];
                let start = std::time::Instant::now();
                while start.elapsed() < Duration::from_millis(1500) {
                    if let Ok((len, addr)) = socket.recv_from(&mut buf) {
                        let text = String::from_utf8_lossy(&buf[..len]);
                        if text.contains("roku") || text.contains("Roku") {
                            let ip = addr.ip().to_string();
                            let _ = tx.send(BackgroundMessage::DeviceDiscovered(RokuDevice {
                                ip,
                                name: "Roku Device (SSDP)".to_string(),
                            }));
                        }
                    }
                }
            }

            // 2. Direct probe on known subnet prefix / current default IP
            let probe_ips = vec!["192.168.0.108".to_string()];
            for ip in probe_ips {
                let target: Result<SocketAddr, _> = format!("{}:8060", ip).parse();
                if let Ok(addr) = target {
                    if TcpStream::connect_timeout(&addr, Duration::from_millis(400)).is_ok() {
                        let _ = tx.send(BackgroundMessage::DeviceDiscovered(RokuDevice {
                            ip,
                            name: "Roku Streaming Stick Plus".to_string(),
                        }));
                    }
                }
            }

            let _ = tx.send(BackgroundMessage::ScanFinished);
        });
    }

    fn send_key(&self, key: &'static str) {
        let ip = self.selected_device_ip.clone();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let url = format!("http://{}:8060/keypress/{}", ip, key);
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_millis(1200))
                .build();
            if let Ok(c) = client {
                let _ = c.post(&url).send();
            }
            // Fetch active app update
            thread::sleep(Duration::from_millis(800));
            update_active_app_worker(&ip, &tx);
        });
    }

    fn launch_app(&self, app_id: String) {
        let ip = self.selected_device_ip.clone();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let url = format!("http://{}:8060/launch/{}", ip, app_id);
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_millis(1500))
                .build();
            if let Ok(c) = client {
                let _ = c.post(&url).send();
            }
            thread::sleep(Duration::from_millis(1000));
            update_active_app_worker(&ip, &tx);
        });
    }

    fn refresh_device_info(&self) {
        let ip = self.selected_device_ip.clone();
        let tx = self.tx.clone();
        thread::spawn(move || {
            update_active_app_worker(&ip, &tx);
            update_apps_worker(&ip, &tx);
        });
    }

    fn handle_incoming_messages(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                BackgroundMessage::DeviceDiscovered(dev) => {
                    if !self.devices.iter().any(|d| d.ip == dev.ip) {
                        self.devices.push(dev);
                    }
                    if self.selected_device_ip.is_empty() && !self.devices.is_empty() {
                        self.selected_device_ip = self.devices[0].ip.clone();
                        self.refresh_device_info();
                    }
                }
                BackgroundMessage::ActiveAppUpdated(app) => {
                    self.active_app = app;
                }
                BackgroundMessage::AppsListUpdated(apps) => {
                    if !apps.is_empty() {
                        self.apps = apps;
                    }
                }
                BackgroundMessage::ScanFinished => {
                    self.is_scanning = false;
                    self.status_text = format!("Found {} device(s)", self.devices.len());
                }
            }
        }
    }
}

fn update_active_app_worker(ip: &str, tx: &Sender<BackgroundMessage>) {
    let url = format!("http://{}:8060/query/active-app", ip);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(1500))
        .build();
    if let Ok(c) = client {
        if let Ok(resp) = c.get(&url).send() {
            if let Ok(text) = resp.text() {
                if let Some(app_name) = parse_active_app_xml(&text) {
                    let _ = tx.send(BackgroundMessage::ActiveAppUpdated(app_name));
                }
            }
        }
    }
}

fn update_apps_worker(ip: &str, tx: &Sender<BackgroundMessage>) {
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
                }
            }
        }
    }
}

fn parse_active_app_xml(xml: &str) -> Option<String> {
    if let Some(start) = xml.find("<app") {
        if let Some(tag_end) = xml[start..].find('>') {
            let content_start = start + tag_end + 1;
            if let Some(close) = xml[content_start..].find("</app>") {
                return Some(xml[content_start..content_start + close].trim().to_string());
            }
        }
    }
    None
}

fn parse_apps_xml(xml: &str) -> Vec<AppItem> {
    let mut items = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<app id=\"") {
        let after_id = &rest[start + 9..];
        if let Some(quote) = after_id.find('"') {
            let id = after_id[..quote].to_string();
            if let Some(tag_close) = after_id[quote..].find('>') {
                let content = &after_id[quote + tag_close + 1..];
                if let Some(app_close) = content.find("</app>") {
                    let name = content[..app_close].trim().to_string();
                    items.push(AppItem { name, id });
                    rest = &content[app_close + 6..];
                    continue;
                }
            }
        }
        break;
    }
    items
}

fn default_popular_apps() -> Vec<AppItem> {
    vec![
        AppItem { name: "YouTube".into(), id: "837".into() },
        AppItem { name: "Netflix".into(), id: "12".into() },
        AppItem { name: "Disney+".into(), id: "291097".into() },
        AppItem { name: "Prime Video".into(), id: "13".into() },
        AppItem { name: "Hulu".into(), id: "2285".into() },
        AppItem { name: "Max / HBO".into(), id: "61322".into() },
        AppItem { name: "Spotify".into(), id: "19977".into() },
        AppItem { name: "Apple TV".into(), id: "551012".into() },
        AppItem { name: "Plex".into(), id: "13535".into() },
    ]
}

impl eframe::App for RokuRemoteApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_incoming_messages();

        egui::CentralPanel::default().show(ctx, |ui| {
            // Header: Device & Scan
            ui.horizontal(|ui| {
                ui.heading("📺 Roku Remote");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("⟳ Scan").clicked() {
                        self.is_scanning = true;
                        self.status_text = "Scanning network...".into();
                        self.start_scan();
                    }
                    if ui.button("🔌 Power").clicked() {
                        self.send_key("Power");
                    }
                });
            });

            // Target selector
            ui.horizontal(|ui| {
                ui.label("Device IP:");
                let text_edit = ui.text_edit_singleline(&mut self.selected_device_ip);
                if text_edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.refresh_device_info();
                }
                if ui.button("Connect").clicked() {
                    self.refresh_device_info();
                }
            });

            ui.horizontal(|ui| {
                ui.label(format!("Active App: {}", self.active_app));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(&self.status_text);
                });
            });

            ui.separator();

            // Remote Navigation Controls (D-PAD)
            ui.vertical_centered(|ui| {
                // Row 1: Back, [spacer], Home
                ui.horizontal(|ui| {
                    ui.add_space(ui.available_width() / 4.0);
                    if ui.add_sized([75.0, 36.0], egui::Button::new("⮌ Back")).clicked() {
                        self.send_key("Back");
                    }
                    ui.add_space(30.0);
                    if ui.add_sized([75.0, 36.0], egui::Button::new("⌂ Home")).clicked() {
                        self.send_key("Home");
                    }
                });

                ui.add_space(6.0);

                // Row 2: UP
                if ui.add_sized([65.0, 42.0], egui::Button::new("▲")).clicked() {
                    self.send_key("Up");
                }

                // Row 3: LEFT, OK, RIGHT
                ui.horizontal(|ui| {
                    ui.add_space(ui.available_width() / 4.0);
                    if ui.add_sized([60.0, 42.0], egui::Button::new("◀")).clicked() {
                        self.send_key("Left");
                    }
                    if ui.add_sized([70.0, 42.0], egui::Button::new("OK")).clicked() {
                        self.send_key("Select");
                    }
                    if ui.add_sized([60.0, 42.0], egui::Button::new("▶")).clicked() {
                        self.send_key("Right");
                    }
                });

                // Row 4: DOWN
                if ui.add_sized([65.0, 42.0], egui::Button::new("▼")).clicked() {
                    self.send_key("Down");
                }

                ui.add_space(6.0);

                // Row 5: Replay & Info
                ui.horizontal(|ui| {
                    ui.add_space(ui.available_width() / 4.0);
                    if ui.add_sized([75.0, 34.0], egui::Button::new("↺ Replay")).clicked() {
                        self.send_key("InstantReplay");
                    }
                    ui.add_space(30.0);
                    if ui.add_sized([75.0, 34.0], egui::Button::new("✱ Info")).clicked() {
                        self.send_key("Info");
                    }
                });
            });

            ui.add_space(10.0);
            ui.separator();

            // Media & Volume Playback Bar
            ui.horizontal_wrapped(|ui| {
                if ui.button("⏪").clicked() {
                    self.send_key("Rev");
                }
                if ui.button("⏯ Play/Pause").clicked() {
                    self.send_key("Play");
                }
                if ui.button("⏩").clicked() {
                    self.send_key("Fwd");
                }
                ui.separator();
                if ui.button("🔉 Vol -").clicked() {
                    self.send_key("VolumeDown");
                }
                if ui.button("🔊 Vol +").clicked() {
                    self.send_key("VolumeUp");
                }
                if ui.button("🔇 Mute").clicked() {
                    self.send_key("VolumeMute");
                }
            });

            ui.add_space(10.0);
            ui.separator();

            // App Launcher Section
            ui.heading("🚀 Quick Launch Apps");
            egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                let mut app_to_launch = None;
                ui.horizontal_wrapped(|ui| {
                    for app in &self.apps {
                        if ui.add_sized([100.0, 32.0], egui::Button::new(&app.name)).clicked() {
                            app_to_launch = Some(app.id.clone());
                        }
                    }
                });
                if let Some(id) = app_to_launch {
                    self.launch_app(id);
                }
            });
        });

        // Request periodic repaints for background async UI updates
        ctx.request_repaint_after(Duration::from_millis(300));
    }
}

fn main() -> Result<(), eframe::Error> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([380.0, 600.0])
            .with_min_inner_size([340.0, 500.0])
            .with_title("Roku Remote"),
        ..Default::default()
    };

    eframe::run_native(
        "Roku Remote",
        native_options,
        Box::new(|cc| Ok(Box::new(RokuRemoteApp::new(cc)))),
    )
}
