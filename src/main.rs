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
        let mut visuals = egui::Visuals::dark();
        visuals.window_rounding = egui::Rounding::same(10.0);
        visuals.widgets.noninteractive.rounding = egui::Rounding::same(6.0);
        visuals.widgets.inactive.rounding = egui::Rounding::same(6.0);
        visuals.widgets.hovered.rounding = egui::Rounding::same(6.0);
        visuals.widgets.active.rounding = egui::Rounding::same(6.0);
        visuals.widgets.open.rounding = egui::Rounding::same(6.0);
        cc.egui_ctx.set_visuals(visuals);

        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(5.0, 5.0);
        style.spacing.button_padding = egui::vec2(6.0, 4.0);
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

    // Ultra-compact D-Pad controller: Takes only ~150px height total
    fn render_controls_section(&self, ui: &mut egui::Ui, width: f32) {
        ui.vertical_centered(|ui| {
            let btn_dir = egui::vec2(44.0, 28.0);
            let btn_ok = egui::vec2(50.0, 28.0);
            let btn_nav = egui::vec2(60.0, 26.0);

            // Row 1: Back & Home
            ui.horizontal(|ui| {
                let spacing = ((width - (btn_nav.x * 2.0)) / 3.0).max(8.0);
                ui.add_space(spacing);
                if ui.add_sized(btn_nav, egui::Button::new("Back")).clicked() {
                    self.send_key("Back");
                }
                ui.add_space(spacing);
                if ui.add_sized(btn_nav, egui::Button::new("Home")).clicked() {
                    self.send_key("Home");
                }
            });

            ui.add_space(2.0);

            // Row 2: UP
            if ui.add_sized(btn_dir, egui::Button::new("Up")).clicked() {
                self.send_key("Up");
            }

            ui.add_space(2.0);

            // Row 3: LEFT, OK, RIGHT
            ui.horizontal(|ui| {
                let row_w = btn_dir.x + 4.0 + btn_ok.x + 4.0 + btn_dir.x;
                let pad = ((width - row_w) / 2.0).max(0.0);
                ui.add_space(pad);

                if ui.add_sized(btn_dir, egui::Button::new("Left")).clicked() {
                    self.send_key("Left");
                }
                if ui.add_sized(btn_ok, egui::Button::new(egui::RichText::new("OK").strong())).clicked() {
                    self.send_key("Select");
                }
                if ui.add_sized(btn_dir, egui::Button::new("Right")).clicked() {
                    self.send_key("Right");
                }
            });

            ui.add_space(2.0);

            // Row 4: DOWN
            if ui.add_sized(btn_dir, egui::Button::new("Down")).clicked() {
                self.send_key("Down");
            }

            ui.add_space(2.0);

            // Row 5: Replay & Info
            ui.horizontal(|ui| {
                let spacing = ((width - (btn_nav.x * 2.0)) / 3.0).max(8.0);
                ui.add_space(spacing);
                if ui.add_sized(btn_nav, egui::Button::new("Replay")).clicked() {
                    self.send_key("InstantReplay");
                }
                ui.add_space(spacing);
                if ui.add_sized(btn_nav, egui::Button::new("Info (*)")).clicked() {
                    self.send_key("Info");
                }
            });

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(4.0);

            // Media & Volume Toolbar (Single compact row)
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                let media_btn = egui::vec2(34.0, 26.0);
                let play_btn = egui::vec2(76.0, 26.0);
                let vol_btn = egui::vec2(46.0, 26.0);

                if ui.add_sized(media_btn, egui::Button::new("<<")).clicked() {
                    self.send_key("Rev");
                }
                if ui.add_sized(play_btn, egui::Button::new("Play / Pause")).clicked() {
                    self.send_key("Play");
                }
                if ui.add_sized(media_btn, egui::Button::new(">>")).clicked() {
                    self.send_key("Fwd");
                }

                ui.separator();

                if ui.add_sized(vol_btn, egui::Button::new("Vol -")).clicked() {
                    self.send_key("VolumeDown");
                }
                if ui.add_sized(vol_btn, egui::Button::new("Vol +")).clicked() {
                    self.send_key("VolumeUp");
                }
                if ui.add_sized(vol_btn, egui::Button::new("Mute")).clicked() {
                    self.send_key("VolumeMute");
                }
            });
        });
    }

    // Applications Section: strict uniform cells with proper centered multi-line wrapping
    fn render_apps_section(&self, ui: &mut egui::Ui, is_wide_layout: bool) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Quick Launch Apps").strong().size(14.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(format!("{} apps", self.apps.len())).weak().size(11.0));
            });
        });
        ui.add_space(4.0);

        let mut app_to_launch = None;
        let avail_h = ui.available_height().max(180.0);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(avail_h)
            .show(ui, |ui| {
                let avail_w = ui.available_width() - 8.0;

                let min_card_w = if is_wide_layout { 110.0 } else { 96.0 };
                let cols = ((avail_w / min_card_w).floor() as usize).max(2);
                let spacing = 6.0;
                let btn_w = ((avail_w - (spacing * (cols as f32 - 1.0))) / (cols as f32)).max(75.0);
                let btn_h = 42.0;

                egui::Grid::new("apps_grid")
                    .spacing([spacing, spacing])
                    .min_col_width(btn_w)
                    .max_col_width(btn_w)
                    .show(ui, |ui| {
                        for (i, app) in self.apps.iter().enumerate() {
                            let label = egui::Label::new(
                                egui::RichText::new(&app.name).size(12.0)
                            )
                            .wrap_mode(egui::TextWrapMode::Wrap)
                            .selectable(false);

                            // Render custom button with centered multi-line label
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(btn_w, btn_h),
                                egui::Sense::click(),
                            );

                            if response.clicked() {
                                app_to_launch = Some(app.id.clone());
                            }

                            let visuals = ui.style().interact(&response);
                            ui.painter().rect(
                                rect,
                                visuals.rounding,
                                visuals.bg_fill,
                                visuals.bg_stroke,
                            );

                            let text_rect = rect.shrink2(egui::vec2(4.0, 2.0));
                            let mut child_ui = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(text_rect)
                                    .layout(egui::Layout::centered_and_justified(egui::Direction::TopDown)),
                            );
                            child_ui.add(label);

                            if (i + 1) % cols == 0 {
                                ui.end_row();
                            }
                        }
                    });
            });

        if let Some(id) = app_to_launch {
            self.launch_app(id);
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
            let total_width = ui.available_width();
            let is_wide = total_width >= 680.0;

            // Global Header
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("📺 Roku Remote").strong().size(17.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new("Scan")).clicked() {
                        self.is_scanning = true;
                        self.status_text = "Scanning network...".into();
                        self.start_scan();
                    }
                    if ui.add(egui::Button::new("Power")).clicked() {
                        self.send_key("Power");
                    }
                });
            });

            ui.add_space(2.0);

            ui.horizontal(|ui| {
                ui.label("Device IP:");
                let text_edit = ui.add(
                    egui::TextEdit::singleline(&mut self.selected_device_ip)
                        .desired_width(120.0)
                );
                if text_edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.refresh_device_info();
                }
                if ui.button("Connect").clicked() {
                    self.refresh_device_info();
                }

                ui.add_space(10.0);
                ui.label(egui::RichText::new("Current:").weak());
                ui.label(
                    egui::RichText::new(&self.active_app)
                        .strong()
                        .color(egui::Color32::from_rgb(130, 210, 255)),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(&self.status_text).weak().size(11.0));
                });
            });

            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);

            if is_wide {
                // WIDE SCREEN: Controls Left (300px), Apps Grid Right
                let controls_width = 300.0f32;
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(controls_width);
                        self.render_controls_section(ui, controls_width);
                    });

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    ui.vertical(|ui| {
                        self.render_apps_section(ui, true);
                    });
                });
            } else {
                // NARROW SCREEN: Slim controls at the top, leaving majority of screen for apps
                let content_width = 360.0f32.min(total_width - 8.0).max(260.0);
                let horizontal_margin = ((total_width - content_width) / 2.0).max(0.0);

                ui.horizontal(|ui| {
                    ui.add_space(horizontal_margin);
                    ui.vertical(|ui| {
                        ui.set_width(content_width);
                        ui.set_min_height(ui.available_height());

                        // Compact controls section
                        self.render_controls_section(ui, content_width);

                        ui.add_space(6.0);
                        ui.separator();
                        ui.add_space(4.0);

                        // Maximize apps viewport to full remaining window height
                        self.render_apps_section(ui, false);
                    });
                });
            }
        });

        ctx.request_repaint_after(Duration::from_millis(300));
    }
}

fn main() -> Result<(), eframe::Error> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([380.0, 640.0])
            .with_min_inner_size([320.0, 480.0])
            .with_title("Roku Remote"),
        ..Default::default()
    };

    eframe::run_native(
        "Roku Remote",
        native_options,
        Box::new(|cc| Ok(Box::new(RokuRemoteApp::new(cc)))),
    )
}
