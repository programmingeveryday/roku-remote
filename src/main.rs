use eframe::egui;
use std::collections::HashMap;
use std::fs;
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

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

#[derive(Clone, Debug, Default)]
pub struct MediaPlayerInfo {
    pub state: String, // "play", "pause", "buffer", "none"
    pub app_name: String,
    pub position_ms: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct DeviceDetails {
    pub model_name: String,
    pub model_number: String,
    pub software_version: String,
    pub network_name: String,
    pub power_mode: String,
    pub ui_resolution: String,
}

enum BackgroundMessage {
    DeviceDiscovered(RokuDevice),
    DeviceNameUpdated(String),
    ActiveAppUpdated(String),
    AppsListUpdated(Vec<AppItem>),
    ScanFinished,
    ThemeUpdated(ThemeColors),
    MediaPlayerUpdated(MediaPlayerInfo),
    DeviceDetailsUpdated(DeviceDetails),
    AppIconLoaded { id: String, image: egui::ColorImage },
}

#[derive(Clone, Debug)]
struct ThemeColors {
    mode: String,
    background: egui::Color32,
    dark_background: egui::Color32,
    lighter_background: egui::Color32,
    foreground: egui::Color32,
    dark_foreground: egui::Color32,
    accent: egui::Color32,
    roku_purple: egui::Color32,
}

impl Default for ThemeColors {
    fn default() -> Self {
        Self {
            mode: "dark".to_string(),
            background: egui::Color32::from_rgb(26, 26, 32),
            dark_background: egui::Color32::from_rgb(34, 34, 42),
            lighter_background: egui::Color32::from_rgb(45, 45, 56),
            foreground: egui::Color32::from_rgb(240, 240, 245),
            dark_foreground: egui::Color32::from_rgb(160, 160, 175),
            accent: egui::Color32::from_rgb(102, 45, 145),
            roku_purple: egui::Color32::from_rgb(102, 45, 145),
        }
    }
}

fn parse_hex_color(hex: &str) -> Option<egui::Color32> {
    let clean = hex.trim().trim_matches('"').trim_start_matches('#');
    if clean.len() == 6 {
        let r = u8::from_str_radix(&clean[0..2], 16).ok()?;
        let g = u8::from_str_radix(&clean[2..4], 16).ok()?;
        let b = u8::from_str_radix(&clean[4..6], 16).ok()?;
        return Some(egui::Color32::from_rgb(r, g, b));
    } else if clean.len() == 8 {
        let r = u8::from_str_radix(&clean[0..2], 16).ok()?;
        let g = u8::from_str_radix(&clean[2..4], 16).ok()?;
        let b = u8::from_str_radix(&clean[4..6], 16).ok()?;
        let a = u8::from_str_radix(&clean[6..8], 16).ok()?;
        return Some(egui::Color32::from_rgba_unmultiplied(r, g, b, a));
    }
    None
}

fn load_omarchy_theme() -> ThemeColors {
    let mut theme = ThemeColors::default();
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let candidates = vec![
        PathBuf::from(&home).join(".local/state/omarchy/current/theme/colors.toml"),
        PathBuf::from(&home).join(".config/omarchy/themes/current/colors.toml"),
    ];

    for path in candidates {
        if let Ok(content) = fs::read_to_string(&path) {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with('#') || !trimmed.contains('=') {
                    continue;
                }
                let mut parts = trimmed.splitn(2, '=');
                let key = parts.next().unwrap_or("").trim();
                let val = parts.next().unwrap_or("").trim().trim_matches('"');

                match key {
                    "mode" => theme.mode = val.to_string(),
                    "background" => {
                        if let Some(c) = parse_hex_color(val) { theme.background = c; }
                    }
                    "dark_background" => {
                        if let Some(c) = parse_hex_color(val) { theme.dark_background = c; }
                    }
                    "lighter_background" => {
                        if let Some(c) = parse_hex_color(val) { theme.lighter_background = c; }
                    }
                    "foreground" => {
                        if let Some(c) = parse_hex_color(val) { theme.foreground = c; }
                    }
                    "dark_foreground" => {
                        if let Some(c) = parse_hex_color(val) { theme.dark_foreground = c; }
                    }
                    "accent" => {
                        if let Some(c) = parse_hex_color(val) { theme.accent = c; }
                    }
                    _ => {}
                }
            }
            break;
        }
    }

    theme
}

struct RokuRemoteApp {
    devices: Vec<RokuDevice>,
    selected_device_ip: String,
    device_name: String,
    active_app: String,
    media_player: MediaPlayerInfo,
    device_details: DeviceDetails,
    show_device_info: bool,
    apps: Vec<AppItem>,
    app_textures: HashMap<String, egui::TextureHandle>,
    pending_icons: Vec<(String, egui::ColorImage)>,
    is_scanning: bool,
    status_text: String,
    show_shortcuts: bool,
    theme: ThemeColors,
    ctx: egui::Context,
    rx: Receiver<BackgroundMessage>,
    tx: Sender<BackgroundMessage>,
}

impl RokuRemoteApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let theme = load_omarchy_theme();
        Self::apply_theme(&cc.egui_ctx, &theme);

        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(6.0, 6.0);
        style.spacing.button_padding = egui::vec2(8.0, 6.0);
        cc.egui_ctx.set_style(style);

        let (tx, rx) = channel();
        Self::start_theme_watcher(tx.clone(), cc.egui_ctx.clone());

        let app = Self {
            devices: Vec::new(),
            selected_device_ip: "192.168.0.108".to_string(),
            device_name: "Roku Streaming Stick Plus".to_string(),
            active_app: "Loading...".to_string(),
            media_player: MediaPlayerInfo::default(),
            device_details: DeviceDetails::default(),
            show_device_info: false,
            apps: default_popular_apps(),
            app_textures: HashMap::new(),
            pending_icons: Vec::new(),
            is_scanning: false,
            status_text: "Ready".to_string(),
            show_shortcuts: false,
            theme,
            ctx: cc.egui_ctx.clone(),
            rx,
            tx,
        };

        app.start_scan();
        app.refresh_device_info();

        app
    }

    fn apply_theme(ctx: &egui::Context, theme: &ThemeColors) {
        let is_dark = theme.mode == "dark";
        let mut visuals = if is_dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };

        visuals.panel_fill = theme.background;
        visuals.window_fill = theme.background;

        visuals.window_rounding = egui::Rounding::same(12.0);
        visuals.widgets.noninteractive.rounding = egui::Rounding::same(8.0);
        visuals.widgets.inactive.rounding = egui::Rounding::same(8.0);
        visuals.widgets.hovered.rounding = egui::Rounding::same(8.0);
        visuals.widgets.active.rounding = egui::Rounding::same(8.0);
        visuals.widgets.open.rounding = egui::Rounding::same(8.0);

        visuals.widgets.inactive.bg_fill = theme.dark_background;
        visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0f32, theme.foreground);
        visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0f32, theme.lighter_background);

        visuals.widgets.hovered.bg_fill = theme.lighter_background;
        visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0f32, theme.foreground);
        visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.5f32, theme.accent);

        visuals.widgets.active.bg_fill = theme.accent;
        visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0f32, egui::Color32::WHITE);

        visuals.selection.bg_fill = theme.accent;
        visuals.selection.stroke = egui::Stroke::new(1.0f32, egui::Color32::WHITE);

        ctx.set_visuals(visuals);
    }

    fn start_theme_watcher(tx: Sender<BackgroundMessage>, ctx: egui::Context) {
        thread::spawn(move || {
            let mut current = load_omarchy_theme();
            loop {
                thread::sleep(Duration::from_secs(3));
                let next = load_omarchy_theme();
                if next.background != current.background || next.accent != current.accent {
                    current = next.clone();
                    let _ = tx.send(BackgroundMessage::ThemeUpdated(next));
                    ctx.request_repaint();
                }
            }
        });
    }

    fn start_scan(&self) {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
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

    fn send_key(&self, key: &'static str) {
        let ip = self.selected_device_ip.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        thread::spawn(move || {
            let url = format!("http://{}:8060/keypress/{}", ip, key);
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_millis(1200))
                .build();
            if let Ok(c) = client {
                let _ = c.post(&url).send();
            }
            thread::sleep(Duration::from_millis(800));
            update_active_app_worker(&ip, &tx, &ctx);
            update_media_player_worker(&ip, &tx, &ctx);
        });
    }

    fn launch_app(&self, app_id: String) {
        let ip = self.selected_device_ip.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        thread::spawn(move || {
            let url = format!("http://{}:8060/launch/{}", ip, app_id);
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_millis(1500))
                .build();
            if let Ok(c) = client {
                let _ = c.post(&url).send();
            }
            thread::sleep(Duration::from_millis(1000));
            update_active_app_worker(&ip, &tx, &ctx);
            update_media_player_worker(&ip, &tx, &ctx);
        });
    }

    fn refresh_device_info(&self) {
        let ip = self.selected_device_ip.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        thread::spawn(move || {
            update_device_name_worker(&ip, &tx, &ctx);
            update_active_app_worker(&ip, &tx, &ctx);
            update_media_player_worker(&ip, &tx, &ctx);
            update_apps_worker(&ip, &tx, &ctx);
        });
    }

    fn fetch_app_icons(&self, apps: &[AppItem]) {
        let ip = self.selected_device_ip.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let apps_clone = apps.to_vec();
        thread::spawn(move || {
            for app in apps_clone {
                load_app_icon_worker(&ip, &app.id, &tx, &ctx);
            }
        });
    }

    fn handle_incoming_messages(&mut self, ctx: &egui::Context) {
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
                BackgroundMessage::DeviceNameUpdated(name) => {
                    self.device_name = name;
                }
                BackgroundMessage::ActiveAppUpdated(app) => {
                    self.active_app = app;
                }
                BackgroundMessage::AppsListUpdated(apps) => {
                    if !apps.is_empty() {
                        self.fetch_app_icons(&apps);
                        self.apps = apps;
                    }
                }
                BackgroundMessage::ScanFinished => {
                    self.is_scanning = false;
                    self.status_text = format!("Found {} device(s)", self.devices.len());
                }
                BackgroundMessage::ThemeUpdated(theme) => {
                    Self::apply_theme(ctx, &theme);
                    self.theme = theme;
                }
                BackgroundMessage::MediaPlayerUpdated(info) => {
                    self.media_player = info;
                }
                BackgroundMessage::DeviceDetailsUpdated(details) => {
                    self.device_details = details;
                }
                BackgroundMessage::AppIconLoaded { id, image } => {
                    self.pending_icons.push((id, image));
                }
            }
        }

        // Convert pending ColorImages to egui Textures on UI thread
        if !self.pending_icons.is_empty() {
            let pending = std::mem::take(&mut self.pending_icons);
            for (id, img) in pending {
                let texture = ctx.load_texture(
                    format!("app_icon_{}", id),
                    img,
                    egui::TextureOptions::LINEAR,
                );
                self.app_textures.insert(id, texture);
            }
        }
    }

    fn render_controls_section(&self, ui: &mut egui::Ui, width: f32) {
        ui.vertical_centered(|ui| {
            let btn_dir = egui::vec2(58.0, 40.0);
            let btn_ok = egui::vec2(66.0, 42.0);
            let btn_nav = egui::vec2(76.0, 36.0);

            // Row 1: Back & Home
            ui.horizontal(|ui| {
                let spacing = ((width - (btn_nav.x * 2.0)) / 3.0).max(12.0);
                ui.add_space(spacing);
                if ui.add_sized(btn_nav, egui::Button::new("Back")).clicked() {
                    self.send_key("Back");
                }
                ui.add_space(spacing);
                if ui.add_sized(btn_nav, egui::Button::new("Home")).clicked() {
                    self.send_key("Home");
                }
            });

            ui.add_space(6.0);

            // Row 2: UP
            if ui.add_sized(btn_dir, egui::Button::new("Up")).clicked() {
                self.send_key("Up");
            }

            ui.add_space(6.0);

            // Row 3: LEFT, OK, RIGHT
            ui.horizontal(|ui| {
                let row_w = btn_dir.x + 8.0 + btn_ok.x + 8.0 + btn_dir.x;
                let pad = ((width - row_w) / 2.0).max(0.0);
                ui.add_space(pad);

                if ui.add_sized(btn_dir, egui::Button::new("Left")).clicked() {
                    self.send_key("Left");
                }
                
                let ok_btn = egui::Button::new(
                    egui::RichText::new("OK")
                        .strong()
                        .color(egui::Color32::WHITE)
                )
                .fill(self.theme.roku_purple);

                if ui.add_sized(btn_ok, ok_btn).clicked() {
                    self.send_key("Select");
                }

                if ui.add_sized(btn_dir, egui::Button::new("Right")).clicked() {
                    self.send_key("Right");
                }
            });

            ui.add_space(6.0);

            // Row 4: DOWN
            if ui.add_sized(btn_dir, egui::Button::new("Down")).clicked() {
                self.send_key("Down");
            }

            ui.add_space(6.0);

            // Row 5: Replay & Info
            ui.horizontal(|ui| {
                let spacing = ((width - (btn_nav.x * 2.0)) / 3.0).max(12.0);
                ui.add_space(spacing);
                if ui.add_sized(btn_nav, egui::Button::new("Replay")).clicked() {
                    self.send_key("InstantReplay");
                }
                ui.add_space(spacing);
                if ui.add_sized(btn_nav, egui::Button::new("Info (*)")).clicked() {
                    self.send_key("Info");
                }
            });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(6.0);

            // Dedicated Row 1: Media Playback (<<, Play/Pause, >>)
            ui.horizontal(|ui| {
                let media_btn = egui::vec2(48.0, 32.0);
                let play_btn = egui::vec2(110.0, 32.0);
                let row_w = media_btn.x + 8.0 + play_btn.x + 8.0 + media_btn.x;
                let pad = ((width - row_w) / 2.0).max(0.0);
                ui.add_space(pad);

                if ui.add_sized(media_btn, egui::Button::new("<<")).clicked() {
                    self.send_key("Rev");
                }
                if ui.add_sized(play_btn, egui::Button::new("Play / Pause")).clicked() {
                    self.send_key("Play");
                }
                if ui.add_sized(media_btn, egui::Button::new(">>")).clicked() {
                    self.send_key("Fwd");
                }
            });

            ui.add_space(6.0);

            // Dedicated Row 2: Volume & Sound (Vol -, Vol +, Mute)
            ui.horizontal(|ui| {
                let vol_btn = egui::vec2(66.0, 32.0);
                let row_w = vol_btn.x + 8.0 + vol_btn.x + 8.0 + vol_btn.x;
                let pad = ((width - row_w) / 2.0).max(0.0);
                ui.add_space(pad);

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

    fn render_apps_section(&self, ui: &mut egui::Ui, is_wide_layout: bool) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Quick Launch Apps").strong().size(14.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(format!("{} apps", self.apps.len())).weak().size(11.0));
            });
        });
        ui.add_space(6.0);

        let mut app_to_launch = None;

        let render_grid = |ui: &mut egui::Ui, app_to_launch: &mut Option<String>| {
            let avail_w = ui.available_width() - 8.0;
            // Near-square aspect ratio: width ~104px, height ~96px
            let min_card_w = if is_wide_layout { 108.0 } else { 96.0 };
            let cols = ((avail_w / min_card_w).floor() as usize).max(2);
            let spacing = 8.0;
            let btn_w = ((avail_w - (spacing * (cols as f32 - 1.0))) / (cols as f32)).max(85.0);
            let btn_h = (btn_w * 0.92).clamp(88.0, 102.0);

            egui::Grid::new("apps_grid")
                .spacing([spacing, spacing])
                .min_col_width(btn_w)
                .max_col_width(btn_w)
                .show(ui, |ui| {
                    for (i, app) in self.apps.iter().enumerate() {
                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(btn_w, btn_h),
                            egui::Sense::click(),
                        );

                        if response.clicked() {
                            *app_to_launch = Some(app.id.clone());
                        }

                        let visuals = ui.style().interact(&response);
                        ui.painter().rect(
                            rect,
                            visuals.rounding,
                            visuals.bg_fill,
                            visuals.bg_stroke,
                        );

                        let inner_rect = rect.shrink2(egui::vec2(4.0, 4.0));

                        let mut child_ui = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(inner_rect)
                                .layout(egui::Layout::top_down(egui::Align::Center)),
                        );

                        if let Some(texture) = self.app_textures.get(&app.id) {
                            child_ui.add_space(2.0);
                            child_ui.image((texture.id(), egui::vec2(42.0, 42.0)));
                            child_ui.add_space(3.0);
                        } else {
                            child_ui.add_space(8.0);
                            child_ui.label(
                                egui::RichText::new("📺")
                                    .size(24.0)
                            );
                            child_ui.add_space(4.0);
                        }

                        let label = egui::Label::new(
                            egui::RichText::new(&app.name)
                                .size(11.0)
                                .strong()
                                .color(self.theme.foreground)
                        )
                        .wrap_mode(egui::TextWrapMode::Wrap)
                        .selectable(false);

                        child_ui.allocate_ui_with_layout(
                            egui::vec2(inner_rect.width(), inner_rect.height() - 48.0),
                            egui::Layout::centered_and_justified(egui::Direction::TopDown),
                            |ui| {
                                ui.add(label);
                            },
                        );

                        if (i + 1) % cols == 0 {
                            ui.end_row();
                        }
                    }
                });
        };

        if is_wide_layout {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    render_grid(ui, &mut app_to_launch);
                });
        } else {
            render_grid(ui, &mut app_to_launch);
        }

        if let Some(id) = app_to_launch {
            self.launch_app(id);
        }
    }
}

fn update_device_name_worker(ip: &str, tx: &Sender<BackgroundMessage>, ctx: &egui::Context) {
    let url = format!("http://{}:8060/query/device-info", ip);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(1500))
        .build();
    if let Ok(c) = client {
        if let Ok(resp) = c.get(&url).send() {
            if let Ok(text) = resp.text() {
                if let Some(name) = parse_device_name_xml(&text) {
                    let cleaned = clean_html_entities(&name);
                    let _ = tx.send(BackgroundMessage::DeviceNameUpdated(cleaned));
                }
                let details = parse_device_details_xml(&text);
                let _ = tx.send(BackgroundMessage::DeviceDetailsUpdated(details));
                ctx.request_repaint();
            }
        }
    }
}

fn parse_device_details_xml(xml: &str) -> DeviceDetails {
    let extract = |tag: &str| -> String {
        let open = format!("<{}>", tag);
        let close = format!("</{}>", tag);
        if let Some(start) = xml.find(&open) {
            let s = start + open.len();
            if let Some(end) = xml[s..].find(&close) {
                return clean_html_entities(xml[s..s + end].trim());
            }
        }
        String::new()
    };

    DeviceDetails {
        model_name: extract("model-name"),
        model_number: extract("model-number"),
        software_version: extract("software-version"),
        network_name: extract("network-name"),
        power_mode: extract("power-mode"),
        ui_resolution: extract("ui-resolution"),
    }
}

fn update_media_player_worker(ip: &str, tx: &Sender<BackgroundMessage>, ctx: &egui::Context) {
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

fn parse_media_player_xml(xml: &str) -> MediaPlayerInfo {
    let mut info = MediaPlayerInfo::default();
    
    // Extract player state: <player state="play" ...>
    if let Some(state_idx) = xml.find("state=\"") {
        let after = &xml[state_idx + 7..];
        if let Some(quote) = after.find('"') {
            info.state = after[..quote].to_string();
        }
    }

    // Extract plugin / app name: <plugin id="..." name="YouTube" />
    if let Some(name_idx) = xml.find("name=\"") {
        let after = &xml[name_idx + 6..];
        if let Some(quote) = after.find('"') {
            info.app_name = clean_html_entities(&after[..quote]);
        }
    }

    // Extract position: <position>22961735 ms</position>
    if let Some(pos_idx) = xml.find("<position>") {
        let after = &xml[pos_idx + 10..];
        if let Some(close) = after.find("</position>") {
            let pos_str = after[..close].trim().trim_end_matches(" ms").trim();
            if let Ok(ms) = pos_str.parse::<u64>() {
                info.position_ms = Some(ms);
            }
        }
    }

    info
}

fn load_app_icon_worker(ip: &str, app_id: &str, tx: &Sender<BackgroundMessage>, ctx: &egui::Context) {
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

fn parse_device_name_xml(xml: &str) -> Option<String> {
    // Priority: user-device-name -> friendly-device-name -> model-name
    for tag in &["user-device-name", "friendly-device-name", "model-name"] {
        let open_tag = format!("<{}>", tag);
        let close_tag = format!("</{}>", tag);
        if let Some(start) = xml.find(&open_tag) {
            let content_start = start + open_tag.len();
            if let Some(end) = xml[content_start..].find(&close_tag) {
                let name = xml[content_start..content_start + end].trim();
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

fn update_active_app_worker(ip: &str, tx: &Sender<BackgroundMessage>, ctx: &egui::Context) {
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

fn update_apps_worker(ip: &str, tx: &Sender<BackgroundMessage>, ctx: &egui::Context) {
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

impl RokuRemoteApp {
    fn handle_keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        let (ctrl, key_up, key_down, key_left, key_right, key_enter, key_space, key_backspace, key_escape, key_h, key_r, key_i, key_m, key_p, key_comma) = ctx.input(|i| {
            (
                i.modifiers.ctrl,
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Space),
                i.key_pressed(egui::Key::Backspace),
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::H),
                i.key_pressed(egui::Key::R),
                i.key_pressed(egui::Key::I),
                i.key_pressed(egui::Key::M),
                i.key_pressed(egui::Key::P),
                i.key_pressed(egui::Key::Comma),
            )
        });

        // Ctrl + , -> Toggle keyboard shortcuts help modal
        if ctrl && key_comma {
            self.show_shortcuts = !self.show_shortcuts;
            return;
        }

        // Escape closes shortcuts dialog if open
        if key_escape && self.show_shortcuts {
            self.show_shortcuts = false;
            return;
        }

        // Navigation & Media / Volume
        if ctrl {
            if key_right {
                self.send_key("Fwd"); // Fast Forward
            } else if key_left {
                self.send_key("Rev"); // Rewind
            } else if key_up {
                self.send_key("VolumeUp"); // Volume Up
            } else if key_down {
                self.send_key("VolumeDown"); // Volume Down
            } else if key_m {
                self.send_key("VolumeMute"); // Mute
            } else if key_p {
                self.send_key("Play"); // Play/Pause
            }
        } else {
            // Standard Navigation
            if key_up {
                self.send_key("Up");
            } else if key_down {
                self.send_key("Down");
            } else if key_left {
                self.send_key("Left");
            } else if key_right {
                self.send_key("Right");
            } else if key_enter || key_space {
                self.send_key("Select"); // OK Button
            } else if key_backspace || key_escape {
                self.send_key("Back"); // Back Button
            } else if key_h {
                self.send_key("Home"); // Home Button
            } else if key_r {
                self.send_key("InstantReplay"); // Replay Button
            } else if key_i {
                self.send_key("Info"); // Info / Options Button
            } else if key_p {
                self.send_key("Play"); // Play / Pause
            }
        }
    }
}

impl eframe::App for RokuRemoteApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_incoming_messages(ctx);
        self.handle_keyboard_shortcuts(ctx);

        // Keyboard Shortcuts Modal Window
        if self.show_shortcuts {
            egui::Window::new("⌨ Keyboard Shortcuts")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                    ui.label(
                        egui::RichText::new("Control Roku directly with your keyboard:")
                            .strong()
                            .color(self.theme.foreground)
                    );
                    ui.separator();

                    egui::Grid::new("shortcuts_grid")
                        .spacing([14.0, 8.0])
                        .show(ui, |ui| {
                            // Column 1: Icon, Column 2: Key combo, Column 3: Description
                            ui.label("🎯");
                            ui.label(egui::RichText::new("Arrow Keys").strong().color(self.theme.accent));
                            ui.label(egui::RichText::new("Navigate Up / Down / Left / Right").color(self.theme.foreground));
                            ui.end_row();

                            ui.label("🔘");
                            ui.label(egui::RichText::new("Enter / Space").strong().color(self.theme.accent));
                            ui.label(egui::RichText::new("OK / Select").color(self.theme.foreground));
                            ui.end_row();

                            ui.label("🔊");
                            ui.label(egui::RichText::new("Ctrl + Up / Down").strong().color(self.theme.accent));
                            ui.label(egui::RichText::new("Volume Up / Volume Down").color(self.theme.foreground));
                            ui.end_row();

                            ui.label("⏩");
                            ui.label(egui::RichText::new("Ctrl + Left / Right").strong().color(self.theme.accent));
                            ui.label(egui::RichText::new("Rewind (<<) / Fast Forward (>>)").color(self.theme.foreground));
                            ui.end_row();

                            ui.label("↩");
                            ui.label(egui::RichText::new("Backspace / Esc").strong().color(self.theme.accent));
                            ui.label(egui::RichText::new("Back").color(self.theme.foreground));
                            ui.end_row();

                            ui.label("🏠");
                            ui.label(egui::RichText::new("H").strong().color(self.theme.accent));
                            ui.label(egui::RichText::new("Home").color(self.theme.foreground));
                            ui.end_row();

                            ui.label("▶⏸");
                            ui.label(egui::RichText::new("P").strong().color(self.theme.accent));
                            ui.label(egui::RichText::new("Play / Pause").color(self.theme.foreground));
                            ui.end_row();

                            ui.label("↺");
                            ui.label(egui::RichText::new("R").strong().color(self.theme.accent));
                            ui.label(egui::RichText::new("Instant Replay").color(self.theme.foreground));
                            ui.end_row();

                            ui.label("✱");
                            ui.label(egui::RichText::new("I").strong().color(self.theme.accent));
                            ui.label(egui::RichText::new("Info / Options (*)").color(self.theme.foreground));
                            ui.end_row();

                            ui.label("🔇");
                            ui.label(egui::RichText::new("Ctrl + M").strong().color(self.theme.accent));
                            ui.label(egui::RichText::new("Mute").color(self.theme.foreground));
                            ui.end_row();

                            ui.label("💡");
                            ui.label(egui::RichText::new("Ctrl + ,").strong().color(self.theme.accent));
                            ui.label(egui::RichText::new("Toggle this shortcuts cheat sheet").color(self.theme.foreground));
                            ui.end_row();
                        });

                    ui.add_space(8.0);
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Close (Esc)").clicked() {
                                self.show_shortcuts = false;
                            }
                        });
                    });
                });
        }

        // Device Info Modal Window
        if self.show_device_info {
            egui::Window::new("ℹ Roku Device Details")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                    egui::Grid::new("device_details_grid")
                        .spacing([14.0, 8.0])
                        .show(ui, |ui| {
                            let mut row = |label: &str, val: &str| {
                                ui.label(egui::RichText::new(label).strong().color(self.theme.accent));
                                ui.label(egui::RichText::new(if val.is_empty() { "—" } else { val }).color(self.theme.foreground));
                                ui.end_row();
                            };

                            row("Device Name:", &self.device_name);
                            row("Model Name:", &self.device_details.model_name);
                            row("Model Number:", &self.device_details.model_number);
                            row("Software Version:", &self.device_details.software_version);
                            row("Wi-Fi Network:", &self.device_details.network_name);
                            row("Display Resolution:", &self.device_details.ui_resolution);
                            row("Power Mode:", &self.device_details.power_mode);
                            row("IP Address:", &self.selected_device_ip);
                        });

                    ui.add_space(8.0);
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Close").clicked() {
                                self.show_device_info = false;
                            }
                        });
                    });
                });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            let total_width = ui.available_width();
            let is_wide = total_width >= 680.0;

            // Global Header
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("📺 Roku Remote")
                        .strong()
                        .size(17.0)
                        .color(self.theme.foreground)
                );

                // Display Roku custom friendly name
                ui.label(
                    egui::RichText::new(format!("• {}", self.device_name))
                        .size(13.0)
                        .color(self.theme.dark_foreground)
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new("Scan")).clicked() {
                        self.is_scanning = true;
                        self.status_text = "Scanning network...".into();
                        self.start_scan();
                    }
                    if ui.add(egui::Button::new("Power")).clicked() {
                        self.send_key("Power");
                    }
                    if ui.add(egui::Button::new("Device Info")).clicked() {
                        self.show_device_info = !self.show_device_info;
                    }
                });
            });

            ui.add_space(2.0);

            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Device IP:").color(self.theme.dark_foreground));
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
                        .color(self.theme.accent),
                );

                // Now Playing playback status
                let state_icon = match self.media_player.state.as_str() {
                    "play" => "▶ Playing",
                    "pause" => "⏸ Paused",
                    "buffer" => "⏳ Buffering",
                    _ => "",
                };

                if !state_icon.is_empty() {
                    ui.add_space(6.0);
                    let pos_text = if let Some(ms) = self.media_player.position_ms {
                        let total_secs = ms / 1000;
                        let mins = total_secs / 60;
                        let secs = total_secs % 60;
                        format!(" ({}:{:02})", mins, secs)
                    } else {
                        String::new()
                    };

                    ui.label(
                        egui::RichText::new(format!("{}{}", state_icon, pos_text))
                            .color(if self.media_player.state == "play" {
                                egui::Color32::from_rgb(70, 190, 100)
                            } else {
                                egui::Color32::from_rgb(230, 170, 60)
                            })
                            .strong()
                            .size(11.5),
                    );
                }

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
                // NARROW SCREEN: Top-level ScrollArea
                let content_width = 380.0f32.min(total_width - 12.0).max(280.0);
                let horizontal_margin = ((total_width - content_width) / 2.0).max(0.0);

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add_space(horizontal_margin);
                            ui.vertical(|ui| {
                                ui.set_width(content_width);

                                // Controls section
                                self.render_controls_section(ui, content_width);

                                ui.add_space(10.0);
                                ui.separator();
                                ui.add_space(8.0);

                                // Applications Section
                                self.render_apps_section(ui, false);
                                ui.add_space(20.0);
                            });
                        });
                    });
            }
        });
    }
}

fn main() -> Result<(), eframe::Error> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([420.0, 860.0])
            .with_min_inner_size([320.0, 680.0])
            .with_title("Roku Remote")
            .with_app_id("org.omarchy.roku.remote"),
        ..Default::default()
    };

    eframe::run_native(
        "Roku Remote",
        native_options,
        Box::new(|cc| Ok(Box::new(RokuRemoteApp::new(cc)))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_color() {
        assert_eq!(parse_hex_color("#316ac5"), Some(egui::Color32::from_rgb(0x31, 0x6a, 0xc5)));
        assert_eq!(parse_hex_color("316ac5"), Some(egui::Color32::from_rgb(0x31, 0x6a, 0xc5)));
        assert_eq!(parse_hex_color("#316ac5ff"), Some(egui::Color32::from_rgba_unmultiplied(0x31, 0x6a, 0xc5, 0xff)));
        assert_eq!(parse_hex_color("invalid"), None);
        assert_eq!(parse_hex_color(""), None);
    }

    #[test]
    fn test_clean_html_entities() {
        assert_eq!(clean_html_entities("News &amp; Weather"), "News & Weather");
        assert_eq!(clean_html_entities("It&apos;s a test"), "It's a test");
        assert_eq!(clean_html_entities("&quot;Hello&quot;"), "\"Hello\"");
        assert_eq!(clean_html_entities("&lt;tag&gt;"), "<tag>");
    }

    #[test]
    fn test_parse_active_app_xml() {
        let sample = "<?xml version=\"1.0\" encoding=\"UTF-8\" ?><active-app><app id=\"837\">YouTube</app></active-app>";
        assert_eq!(parse_active_app_xml(sample), Some("YouTube".to_string()));

        let empty = "<?xml version=\"1.0\" encoding=\"UTF-8\" ?><active-app></active-app>";
        assert_eq!(parse_active_app_xml(empty), None);

        let malformed = "just plain text";
        assert_eq!(parse_active_app_xml(malformed), None);
    }

    #[test]
    fn test_parse_device_name_xml() {
        let xml_user = "<device-info><user-device-name>Living Room TV</user-device-name></device-info>";
        assert_eq!(parse_device_name_xml(xml_user), Some("Living Room TV".to_string()));

        let xml_model = "<device-info><model-name>Roku Stick</model-name></device-info>";
        assert_eq!(parse_device_name_xml(xml_model), Some("Roku Stick".to_string()));

        let xml_empty = "<device-info></device-info>";
        assert_eq!(parse_device_name_xml(xml_empty), None);
    }

    #[test]
    fn test_parse_apps_xml() {
        let sample = "<apps><app id=\"837\">YouTube</app><app id=\"12\">Netflix &amp; Chill</app></apps>";
        let apps = parse_apps_xml(sample);
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].id, "837");
        assert_eq!(apps[0].name, "YouTube");
        assert_eq!(apps[1].id, "12");
        assert_eq!(apps[1].name, "Netflix & Chill");

        let malformed = "<apps><app id=\"incomplete";
        assert_eq!(parse_apps_xml(malformed).len(), 0);
    }

    #[test]
    fn test_parse_media_player_xml() {
        let sample = "<?xml version=\"1.0\" encoding=\"UTF-8\" ?><player state=\"play\" error=\"false\"><plugin id=\"837\" name=\"YouTube\" /><format audio=\"aac\" video=\"av1\" /><position>22961735 ms</position></player>";
        let info = parse_media_player_xml(sample);
        assert_eq!(info.state, "play");
        assert_eq!(info.app_name, "YouTube");
        assert_eq!(info.position_ms, Some(22961735));

        let paused = "<player state=\"pause\"><plugin name=\"Netflix\" /><position>5000 ms</position></player>";
        let p_info = parse_media_player_xml(paused);
        assert_eq!(p_info.state, "pause");
        assert_eq!(p_info.app_name, "Netflix");
        assert_eq!(p_info.position_ms, Some(5000));
    }

    #[test]
    fn test_parse_device_details_xml() {
        let sample = "<device-info><model-name>Roku Stick</model-name><model-number>3830R</model-number><software-version>15.3.4</software-version><network-name>HomeWi-Fi</network-name><power-mode>PowerOn</power-mode><ui-resolution>1080p</ui-resolution></device-info>";
        let details = parse_device_details_xml(sample);
        assert_eq!(details.model_name, "Roku Stick");
        assert_eq!(details.model_number, "3830R");
        assert_eq!(details.software_version, "15.3.4");
        assert_eq!(details.network_name, "HomeWi-Fi");
        assert_eq!(details.power_mode, "PowerOn");
        assert_eq!(details.ui_resolution, "1080p");
    }
}
