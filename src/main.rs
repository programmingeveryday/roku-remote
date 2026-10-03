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
        // Configure styling for a sleek, rounded modern remote feel
        let mut visuals = egui::Visuals::dark();
        visuals.window_rounding = egui::Rounding::same(12.0);
        visuals.widgets.noninteractive.rounding = egui::Rounding::same(8.0);
        visuals.widgets.inactive.rounding = egui::Rounding::same(8.0);
        visuals.widgets.hovered.rounding = egui::Rounding::same(8.0);
        visuals.widgets.active.rounding = egui::Rounding::same(8.0);
        visuals.widgets.open.rounding = egui::Rounding::same(8.0);
        cc.egui_ctx.set_visuals(visuals);

        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(10.0, 8.0);
        cc.egui_ctx.set_style(style);

        let (tx, rx) = channel();
        let app = Self {
            devices: Vec::new(),
            selected_device_ip: "192.168.0.108".to_string(),
            active_app: "Loading...".to_string(),
            apps: default_popular_apps(),
            is_scanning: false,
            status_text: "Ready".to_string(),
            rx,
            tx,
        };

        app.start_scan();
        app.refresh_device_info();

        app
    }

    fn start_scan(&self) {
        let tx = self.tx.clone();
        thread::spawn(move || {
            // 1. Try SSDP M-SEARCH broadcast
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
                        if text.to_lowercase().contains("roku") {
                            let ip = addr.ip().to_string();
                            let _ = tx.send(BackgroundMessage::DeviceDiscovered(RokuDevice {
                                ip,
                                name: "Roku Device".to_string(),
                            }));
                        }
                    }
                }
            }

            // 2. Direct probe known address
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
                    let cleaned = clean_html_entities(&app_name);
                    let _ = tx.send(BackgroundMessage::ActiveAppUpdated(cleaned));
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

fn clean_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&apos;", "'")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
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
                    let name = clean_html_entities(content[..app_close].trim());
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
        AppItem { name: "Apple TV".into(), id: "551012".into() },
        AppItem { name: "Spotify".into(), id: "19977".into() },
        AppItem { name: "Max / HBO".into(), id: "61322".into() },
        AppItem { name: "Plex".into(), id: "13535".into() },
    ]
}

impl eframe::App for RokuRemoteApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_incoming_messages();

        egui::CentralPanel::default().show(ctx, |ui| {
            // Header Bar
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Roku Remote").strong().size(18.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Scan").clicked() {
                        self.is_scanning = true;
                        self.status_text = "Scanning network...".into();
                        self.start_scan();
                    }
                    if ui.button("Power").clicked() {
                        self.send_key("Power");
                    }
                });
            });

            ui.add_space(4.0);

            // Device Connection & Info Row
            ui.horizontal(|ui| {
                ui.label("Device IP:");
                let text_edit = ui.text_edit_singleline(&mut self.selected_device_ip);
                if text_edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.refresh_device_info();
                }
                if ui.button("Connect").clicked() {
                    self.refresh_device_info();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(&self.status_text).weak().size(11.0));
                });
            });

            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Active:").weak());
                ui.label(egui::RichText::new(&self.active_app).strong().color(egui::Color32::from_rgb(130, 210, 255)));
            });

            ui.add_space(4.0);
            ui.separator();
            ui.add_space(6.0);

            // REMOTE CONTROL PAD (Contained in a sleek centered card)
            let pad_btn_size = egui::vec2(52.0, 44.0);
            let action_btn_size = egui::vec2(72.0, 36.0);

            ui.vertical_centered(|ui| {
                egui::Frame::group(ui.style())
                    .fill(egui::Color32::from_rgba_premultiplied(35, 35, 42, 220))
                    .rounding(egui::Rounding::same(16.0))
                    .inner_margin(egui::Margin::symmetric(24.0, 14.0))
                    .show(ui, |ui| {
                        // Top Row: Back & Home
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(24.0, 0.0);
                            if ui.add_sized(action_btn_size, egui::Button::new("Back")).clicked() {
                                self.send_key("Back");
                            }
                            if ui.add_sized(action_btn_size, egui::Button::new("Home")).clicked() {
                                self.send_key("Home");
                            }
                        });

                        ui.add_space(10.0);

                        // Directional Cluster: Up
                        if ui.add_sized(pad_btn_size, egui::Button::new("Up")).clicked() {
                            self.send_key("Up");
                        }

                        ui.add_space(4.0);

                        // Directional Cluster: Left, OK, Right
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);
                            if ui.add_sized(pad_btn_size, egui::Button::new("Left")).clicked() {
                                self.send_key("Left");
                            }
                            if ui.add_sized(egui::vec2(60.0, 44.0), egui::Button::new(egui::RichText::new("OK").strong())).clicked() {
                                self.send_key("Select");
                            }
                            if ui.add_sized(pad_btn_size, egui::Button::new("Right")).clicked() {
                                self.send_key("Right");
                            }
                        });

                        ui.add_space(4.0);

                        // Directional Cluster: Down
                        if ui.add_sized(pad_btn_size, egui::Button::new("Down")).clicked() {
                            self.send_key("Down");
                        }

                        ui.add_space(10.0);

                        // Bottom Row: Replay & Info
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(24.0, 0.0);
                            if ui.add_sized(action_btn_size, egui::Button::new("Replay")).clicked() {
                                self.send_key("InstantReplay");
                            }
                            if ui.add_sized(action_btn_size, egui::Button::new("Info (*)")).clicked() {
                                self.send_key("Info");
                            }
                        });
                    });
            });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(4.0);

            // Playback & Volume Control Toolbar (Evenly spaced & clean)
            ui.vertical_centered(|ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                    
                    let media_size = egui::vec2(40.0, 32.0);
                    let play_size = egui::vec2(88.0, 32.0);
                    let vol_size = egui::vec2(60.0, 32.0);

                    if ui.add_sized(media_size, egui::Button::new("<<")).clicked() {
                        self.send_key("Rev");
                    }
                    if ui.add_sized(play_size, egui::Button::new("Play / Pause")).clicked() {
                        self.send_key("Play");
                    }
                    if ui.add_sized(media_size, egui::Button::new(">>")).clicked() {
                        self.send_key("Fwd");
                    }

                    ui.separator();

                    if ui.add_sized(vol_size, egui::Button::new("Vol -")).clicked() {
                        self.send_key("VolumeDown");
                    }
                    if ui.add_sized(vol_size, egui::Button::new("Vol +")).clicked() {
                        self.send_key("VolumeUp");
                    }
                    if ui.add_sized(vol_size, egui::Button::new("Mute")).clicked() {
                        self.send_key("VolumeMute");
                    }
                });
            });

            ui.add_space(6.0);
            ui.separator();

            // Quick Launch Apps Section
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Quick Launch Apps").strong().size(14.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(format!("{} apps", self.apps.len())).weak().size(11.0));
                });
            });
            ui.add_space(4.0);

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(160.0)
                .show(ui, |ui| {
                    let mut app_to_launch = None;
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                        for app in &self.apps {
                            let btn = egui::Button::new(&app.name);
                            if ui.add_sized([106.0, 32.0], btn).clicked() {
                                app_to_launch = Some(app.id.clone());
                            }
                        }
                    });
                    if let Some(id) = app_to_launch {
                        self.launch_app(id);
                    }
                });
        });

        ctx.request_repaint_after(Duration::from_millis(300));
    }
}

fn main() -> Result<(), eframe::Error> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([410.0, 640.0])
            .with_min_inner_size([380.0, 560.0])
            .with_title("Roku Remote"),
        ..Default::default()
    };

    eframe::run_native(
        "Roku Remote",
        native_options,
        Box::new(|cc| Ok(Box::new(RokuRemoteApp::new(cc)))),
    )
}
