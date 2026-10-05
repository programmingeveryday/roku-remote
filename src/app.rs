use eframe::egui;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::models::{
    AppItem, BackgroundMessage, DeviceDetails, MediaPlayerInfo, RokuDevice,
};
use crate::roku::client::{
    load_app_icon_worker, refresh_apps_worker, start_scan, update_active_app_worker,
    update_apps_worker, update_device_name_worker, update_media_player_worker,
};
use crate::theme::{apply_theme, load_omarchy_theme, start_theme_watcher, ThemeColors};

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

pub struct RokuRemoteApp {
    pub devices: Vec<RokuDevice>,
    pub selected_device_ip: String,
    pub shared_ip: Arc<Mutex<String>>,
    pub is_active: Arc<AtomicBool>,
    pub last_active: Instant,
    pub device_name: String,
    pub active_app: String,
    pub media_player: MediaPlayerInfo,
    pub device_details: DeviceDetails,
    pub is_device_reachable: bool,
    pub show_device_info: bool,
    pub show_setup_guide: bool,
    pub manual_ip_mode: bool,
    pub apps: Vec<AppItem>,
    pub app_textures: HashMap<String, egui::TextureHandle>,
    pub pending_icons: Vec<(String, egui::ColorImage)>,
    pub is_scanning: bool,
    pub is_refreshing_apps: bool,
    pub status_text: String,
    pub show_shortcuts: bool,
    pub theme: ThemeColors,
    pub ctx: egui::Context,
    pub rx: Receiver<BackgroundMessage>,
    pub tx: Sender<BackgroundMessage>,
}

impl RokuRemoteApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let theme = load_omarchy_theme();
        apply_theme(&cc.egui_ctx, &theme);

        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(6.0, 6.0);
        style.spacing.button_padding = egui::vec2(8.0, 6.0);
        cc.egui_ctx.set_style(style);

        let is_active = Arc::new(AtomicBool::new(true));
        let (tx, rx) = channel();
        start_theme_watcher(tx.clone(), cc.egui_ctx.clone(), is_active.clone());

        let initial_ip = crate::roku::client::load_cached_last_ip()
            .unwrap_or_else(|| "192.168.0.31".to_string());
        let shared_ip = Arc::new(Mutex::new(initial_ip.clone()));
        Self::start_playback_watcher(
            shared_ip.clone(),
            tx.clone(),
            cc.egui_ctx.clone(),
            is_active.clone(),
        );

        let app = Self {
            devices: Vec::new(),
            selected_device_ip: initial_ip,
            shared_ip,
            is_active,
            last_active: Instant::now(),
            device_name: "Roku Device".to_string(),
            active_app: "Loading...".to_string(),
            media_player: MediaPlayerInfo::default(),
            device_details: DeviceDetails::default(),
            is_device_reachable: false,
            show_device_info: false,
            show_setup_guide: false,
            manual_ip_mode: false,
            apps: Vec::new(),
            app_textures: HashMap::new(),
            pending_icons: Vec::new(),
            is_scanning: true,
            is_refreshing_apps: true,
            status_text: "Discovering Rokus...".to_string(),
            show_shortcuts: false,
            theme,
            ctx: cc.egui_ctx.clone(),
            rx,
            tx,
        };

        app.start_discovery_scan();
        app.refresh_device_info();

        app
    }

    pub fn select_device(&mut self, ip: &str) {
        self.selected_device_ip = ip.to_string();
        if let Some(dev) = self.devices.iter().find(|d| d.ip == ip) {
            self.device_name = dev.name.clone();
        }
        if let Ok(mut guard) = self.shared_ip.lock() {
            *guard = ip.to_string();
        }
        crate::roku::client::save_cached_last_ip(ip);
        self.status_text = format!("Connected to {}", ip);
        // Clear previous applications so we don't display stale apps
        self.apps.clear();
        self.app_textures.clear();
        self.is_refreshing_apps = true;
        self.refresh_device_info();
    }

    fn start_playback_watcher(
        shared_ip: Arc<Mutex<String>>,
        tx: Sender<BackgroundMessage>,
        ctx: egui::Context,
        is_active: Arc<AtomicBool>,
    ) {
        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_secs(2));
                if !is_active.load(Ordering::Relaxed) {
                    continue;
                }
                let ip = {
                    let guard = shared_ip.lock().unwrap();
                    guard.clone()
                };
                if !ip.is_empty() {
                    update_device_name_worker(&ip, &tx, &ctx);
                    update_media_player_worker(&ip, &tx, &ctx);
                }
            }
        });
    }

    pub fn start_discovery_scan(&self) {
        start_scan(self.tx.clone(), self.ctx.clone());
    }

    pub fn send_key(&self, key: &'static str) {
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

    pub fn power_on(&mut self) {
        if self.device_details.is_tv {
            self.device_details.power_mode = "PowerOn".to_string();
        }
        let ip = self.selected_device_ip.clone();
        let is_tv = self.device_details.is_tv;
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();

        thread::spawn(move || {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_millis(1500))
                .build();

            if let Ok(ref c) = client {
                if is_tv {
                    let url = format!("http://{}:8060/keypress/PowerOn", ip);
                    let mut sent = false;
                    if let Ok(resp) = c.post(&url).send() {
                        if resp.status().is_success() {
                            sent = true;
                        }
                    }
                    if !sent {
                        let fallback_url = format!("http://{}:8060/keypress/Power", ip);
                        let _ = c.post(&fallback_url).send();
                    }
                } else {
                    // For streaming sticks and players:
                    // 1. Send Power toggle to signal TV via HDMI-CEC
                    let power_url = format!("http://{}:8060/keypress/Power", ip);
                    let _ = c.post(&power_url).send();

                    // 2. Wait 150ms and send Home to trigger HDMI-CEC 1-Touch Play (Image View On / Active Source)
                    // This explicitly wakes the connected TV screen and switches to Roku input
                    thread::sleep(Duration::from_millis(150));
                    let home_url = format!("http://{}:8060/keypress/Home", ip);
                    let _ = c.post(&home_url).send();
                }
            }

            for delay in [600, 1000, 1600] {
                thread::sleep(Duration::from_millis(delay));
                update_device_name_worker(&ip, &tx, &ctx);
                update_media_player_worker(&ip, &tx, &ctx);
            }
        });
    }

    pub fn power_off(&mut self) {
        if self.device_details.is_tv {
            self.device_details.power_mode = "PowerOff".to_string();
        }
        let ip = self.selected_device_ip.clone();
        let is_tv = self.device_details.is_tv;
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();

        thread::spawn(move || {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_millis(1500))
                .build();

            if let Ok(ref c) = client {
                if is_tv {
                    let url = format!("http://{}:8060/keypress/PowerOff", ip);
                    let mut sent = false;
                    if let Ok(resp) = c.post(&url).send() {
                        if resp.status().is_success() {
                            sent = true;
                        }
                    }
                    if !sent {
                        let fallback_url = format!("http://{}:8060/keypress/Power", ip);
                        let _ = c.post(&fallback_url).send();
                    }
                } else {
                    // For streaming sticks: sending Power transmits HDMI-CEC Standby to the TV
                    let power_url = format!("http://{}:8060/keypress/Power", ip);
                    let _ = c.post(&power_url).send();
                }
            }

            for delay in [600, 1000, 1600] {
                thread::sleep(Duration::from_millis(delay));
                update_device_name_worker(&ip, &tx, &ctx);
                update_media_player_worker(&ip, &tx, &ctx);
            }
        });
    }

    pub fn toggle_power(&mut self) {
        if self.device_details.is_tv {
            let currently_on = self.is_device_reachable
                && self.device_details.power_mode != "PowerOff"
                && self.device_details.power_mode != "Standby";
            if currently_on {
                self.power_off();
            } else {
                self.power_on();
            }
        } else {
            self.power_on();
        }
    }

    pub fn launch_app(&self, app_id: String) {
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

    pub fn refresh_device_info(&self) {
        if self.selected_device_ip.is_empty() {
            return;
        }
        if let Ok(mut guard) = self.shared_ip.lock() {
            *guard = self.selected_device_ip.clone();
        }
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

    pub fn fetch_app_icons(&self, apps: &[AppItem]) {
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

    pub fn refresh_apps(&mut self) {
        if self.selected_device_ip.is_empty() {
            self.status_text = "No device connected".to_string();
            return;
        }
        self.is_refreshing_apps = true;
        self.status_text = "Checking apps...".to_string();
        let ip = self.selected_device_ip.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        thread::spawn(move || {
            refresh_apps_worker(&ip, &tx, &ctx);
        });
    }

    pub fn handle_incoming_messages(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                BackgroundMessage::DeviceDiscovered(dev) => {
                    if let Some(existing) = self.devices.iter_mut().find(|d| d.ip == dev.ip) {
                        existing.name = dev.name.clone();
                    } else {
                        self.devices.push(dev.clone());
                    }
                    if self.selected_device_ip == dev.ip {
                        self.device_name = dev.name.clone();
                    }
                    if (self.selected_device_ip.is_empty() || !self.is_device_reachable) && !self.devices.is_empty() {
                        self.select_device(&dev.ip);
                    }
                }
                BackgroundMessage::DeviceNameUpdated(name) => {
                    self.device_name = name.clone();
                    if let Some(d) = self.devices.iter_mut().find(|d| d.ip == self.selected_device_ip) {
                        d.name = name;
                    }
                }
                BackgroundMessage::ActiveAppUpdated(app) => {
                    self.active_app = app;
                }
                BackgroundMessage::AppsListUpdated(apps) => {
                    self.is_refreshing_apps = false;
                    self.fetch_app_icons(&apps);
                    self.apps = apps;
                }
                BackgroundMessage::AppsListRefreshed(apps) => {
                    self.is_refreshing_apps = false;
                    let count = apps.len();
                    self.fetch_app_icons(&apps);
                    self.apps = apps;
                    self.status_text = format!("Updated {} apps", count);
                }
                BackgroundMessage::AppsRefreshFailed => {
                    self.is_refreshing_apps = false;
                    self.apps.clear();
                    self.app_textures.clear();
                }
                BackgroundMessage::ScanFinished => {
                    self.is_scanning = false;
                    if self.devices.is_empty() {
                        self.status_text = "No Roku devices found".to_string();
                    } else {
                        self.status_text = format!("Found {} Roku(s)", self.devices.len());
                    }
                }
                BackgroundMessage::ThemeUpdated(theme) => {
                    apply_theme(ctx, &theme);
                    self.theme = theme;
                }
                BackgroundMessage::MediaPlayerUpdated(info) => {
                    self.media_player = info;
                }
                BackgroundMessage::DeviceDetailsUpdated(details) => {
                    let is_limited = details.ecp_setting_mode.eq_ignore_ascii_case("limited")
                        || details.ecp_setting_mode.eq_ignore_ascii_case("disabled");
                    if is_limited {
                        self.status_text = "Limited Mode - Setup Required".to_string();
                    } else if self.apps.is_empty() && self.is_device_reachable && !self.is_refreshing_apps {
                        self.refresh_apps();
                    }
                    self.device_details = details;
                }
                BackgroundMessage::PowerStateUpdated(reachable) => {
                    let was_reachable = self.is_device_reachable;
                    self.is_device_reachable = reachable;
                    if !was_reachable && reachable && self.apps.is_empty() && !self.is_refreshing_apps {
                        let is_limited = self.device_details.ecp_setting_mode.eq_ignore_ascii_case("limited")
                            || self.device_details.ecp_setting_mode.eq_ignore_ascii_case("disabled");
                        if !is_limited {
                            self.refresh_apps();
                        }
                    }
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

    pub fn render_controls_section(&self, ui: &mut egui::Ui, width: f32) {
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

            let draw_arrow_button = |ui: &mut egui::Ui, size: egui::Vec2, direction: &'static str| -> bool {
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
                let visuals = ui.style().interact(&response);

                ui.painter().rect(
                    rect,
                    visuals.rounding,
                    visuals.bg_fill,
                    visuals.bg_stroke,
                );

                let center = rect.center();
                let arrow_r = 7.0f32; // Half-size of arrow glyph
                let fg = visuals.text_color();

                let points = match direction {
                    "up" => vec![
                        center + egui::vec2(0.0, -arrow_r),
                        center + egui::vec2(-arrow_r * 1.15, arrow_r * 0.85),
                        center + egui::vec2(arrow_r * 1.15, arrow_r * 0.85),
                    ],
                    "down" => vec![
                        center + egui::vec2(0.0, arrow_r),
                        center + egui::vec2(-arrow_r * 1.15, -arrow_r * 0.85),
                        center + egui::vec2(arrow_r * 1.15, -arrow_r * 0.85),
                    ],
                    "left" => vec![
                        center + egui::vec2(-arrow_r, 0.0),
                        center + egui::vec2(arrow_r * 0.85, -arrow_r * 1.15),
                        center + egui::vec2(arrow_r * 0.85, arrow_r * 1.15),
                    ],
                    "right" => vec![
                        center + egui::vec2(arrow_r, 0.0),
                        center + egui::vec2(-arrow_r * 0.85, -arrow_r * 1.15),
                        center + egui::vec2(-arrow_r * 0.85, arrow_r * 1.15),
                    ],
                    _ => vec![],
                };

                if !points.is_empty() {
                    ui.painter().add(egui::Shape::convex_polygon(
                        points,
                        fg,
                        egui::Stroke::NONE,
                    ));
                }

                response.clicked()
            };

            // Row 2: UP
            if draw_arrow_button(ui, btn_dir, "up") {
                self.send_key("Up");
            }

            ui.add_space(6.0);

            // Row 3: LEFT, OK, RIGHT
            ui.horizontal(|ui| {
                let row_w = btn_dir.x + 8.0 + btn_ok.x + 8.0 + btn_dir.x;
                let pad = ((width - row_w) / 2.0).max(0.0);
                ui.add_space(pad);

                if draw_arrow_button(ui, btn_dir, "left") {
                    self.send_key("Left");
                }

                let ok_btn = egui::Button::new(
                    egui::RichText::new("OK")
                        .strong()
                        .color(egui::Color32::WHITE),
                )
                .fill(self.theme.roku_purple);

                if ui.add_sized(btn_ok, ok_btn).clicked() {
                    self.send_key("Select");
                }

                if draw_arrow_button(ui, btn_dir, "right") {
                    self.send_key("Right");
                }
            });

            ui.add_space(6.0);

            // Row 4: DOWN
            if draw_arrow_button(ui, btn_dir, "down") {
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

                let (play_label, play_fill, play_text_color) = match self.media_player.state.as_str() {
                    "play" => (
                        "⏸ Pause",
                        egui::Color32::from_rgb(38, 150, 78), // Vibrant green when playing
                        egui::Color32::WHITE,
                    ),
                    "pause" => (
                        "▶ Play",
                        egui::Color32::from_rgb(215, 145, 30), // Amber / orange when paused
                        egui::Color32::WHITE,
                    ),
                    _ => (
                        "▶ / ⏸",
                        self.theme.dark_background,
                        self.theme.foreground,
                    ),
                };

                let play_btn_widget = egui::Button::new(
                    egui::RichText::new(play_label)
                        .strong()
                        .color(play_text_color),
                )
                .fill(play_fill);

                if ui.add_sized(play_btn, play_btn_widget).clicked() {
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

    pub fn render_apps_section(&mut self, ui: &mut egui::Ui, is_wide_layout: bool) {
        let mut do_refresh_apps = false;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Quick Launch Apps")
                    .strong()
                    .size(14.0)
                    .color(self.theme.foreground),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let refresh_text = if self.is_refreshing_apps {
                    "⏳ Checking..."
                } else {
                    "🔄 Refresh Apps"
                };
                let btn = egui::Button::new(
                    egui::RichText::new(refresh_text)
                        .size(11.0)
                        .color(self.theme.foreground),
                );
                if ui.add_enabled(!self.is_refreshing_apps, btn).clicked() {
                    do_refresh_apps = true;
                }

                if !self.apps.is_empty() {
                    ui.label(
                        egui::RichText::new(format!("{} apps •", self.apps.len()))
                            .size(11.0)
                            .color(self.theme.dark_foreground),
                    );
                }
            });
        });
        if do_refresh_apps {
            self.refresh_apps();
        }
        ui.add_space(6.0);

        if self.apps.is_empty() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                if self.is_refreshing_apps {
                    ui.spinner();
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("Scanning for installed applications...")
                            .color(self.theme.dark_foreground)
                            .size(12.0),
                    );
                } else {
                    let is_limited = self.is_device_reachable
                        && (self.device_details.ecp_setting_mode.eq_ignore_ascii_case("limited")
                            || self.device_details.ecp_setting_mode.eq_ignore_ascii_case("disabled"));

                    if is_limited {
                        ui.label(
                            egui::RichText::new("No applications available (commands blocked in Limited mode)")
                                .color(self.theme.dark_foreground)
                                .size(12.0),
                        );
                    } else {
                        ui.label(
                            egui::RichText::new("No applications available for this device")
                                .color(self.theme.dark_foreground)
                                .size(12.0),
                        );
                    }
                }
            });
            return;
        }

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

                        let inner_rect = rect.shrink2(egui::vec2(6.0, 6.0));

                        let mut child_ui = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(inner_rect)
                                .layout(egui::Layout::centered_and_justified(egui::Direction::TopDown)),
                        );

                        if let Some(texture) = self.app_textures.get(&app.id) {
                            // Icon takes up the full space of the card with aspect fit
                            let tex_size = texture.size_vec2();
                            let aspect = if tex_size.y > 0.0 { tex_size.x / tex_size.y } else { 1.0 };
                            let max_w = inner_rect.width();
                            let max_h = inner_rect.height();

                            let img_size = if max_w / aspect <= max_h {
                                egui::vec2(max_w, max_w / aspect)
                            } else {
                                egui::vec2(max_h * aspect, max_h)
                            };

                            child_ui.image((texture.id(), img_size));
                        } else {
                            // Fallback when no icon is found: show TV icon + app name
                            child_ui.vertical_centered(|ui| {
                                ui.add_space(8.0);
                                ui.label(
                                    egui::RichText::new("📺")
                                        .size(26.0),
                                );
                                ui.add_space(4.0);
                                let label = egui::Label::new(
                                    egui::RichText::new(&app.name)
                                        .size(11.0)
                                        .strong()
                                        .color(self.theme.foreground),
                                )
                                .wrap_mode(egui::TextWrapMode::Wrap)
                                .selectable(false);
                                ui.add(label);
                            });
                        }

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

    pub fn handle_keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        let (
            ctrl,
            shift,
            key_up,
            key_down,
            key_left,
            key_right,
            key_enter,
            key_space,
            key_backspace,
            key_escape,
            key_h,
            key_r,
            key_i,
            key_m,
            key_p,
            key_a,
            key_comma,
        ) = ctx.input(|i| {
            (
                i.modifiers.ctrl,
                i.modifiers.shift,
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
                i.key_pressed(egui::Key::A),
                i.key_pressed(egui::Key::Comma),
            )
        });

        // Ctrl + , -> Toggle keyboard shortcuts help modal
        if ctrl && key_comma {
            self.show_shortcuts = !self.show_shortcuts;
            return;
        }

        // Escape closes any open modal dialog
        if key_escape && (self.show_shortcuts || self.show_device_info || self.show_setup_guide) {
            self.show_shortcuts = false;
            self.show_device_info = false;
            self.show_setup_guide = false;
            return;
        }

        // Navigation & Media / Volume
        if ctrl {
            if (shift && key_r) || key_a {
                self.refresh_apps();
            } else if key_right {
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

fn is_hyprland_focused() -> Option<bool> {
    let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    let xdg = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/run/user/1000".to_string());
    let sock_path = format!("{}/hypr/{}/.socket.sock", xdg, sig);
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    let mut stream = UnixStream::connect(sock_path).ok()?;
    stream.set_read_timeout(Some(std::time::Duration::from_millis(20))).ok()?;
    stream.set_write_timeout(Some(std::time::Duration::from_millis(20))).ok()?;
    stream.write_all(b"j/activewindow").ok()?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let my_pid = std::process::id();
    Some(text.contains(&format!("\"pid\": {}", my_pid)))
}

fn draw_power_icon(
    painter: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    color: egui::Color32,
    stroke_width: f32,
) {
    let gap = 0.55f32; // opening at the top of the circle
    let start_angle = -std::f32::consts::FRAC_PI_2 + gap;
    let end_angle = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU - gap;
    let segments = 24;
    let arc_points: Vec<egui::Pos2> = (0..=segments)
        .map(|i| {
            let t = i as f32 / segments as f32;
            let angle = start_angle + t * (end_angle - start_angle);
            egui::pos2(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            )
        })
        .collect();

    painter.add(egui::Shape::line(
        arc_points,
        egui::Stroke::new(stroke_width, color),
    ));

    // Vertical line going through the top notch
    let line_bottom = center.y + radius * 0.05;
    let line_top = center.y - radius * 1.15;
    painter.line_segment(
        [
            egui::pos2(center.x, line_bottom),
            egui::pos2(center.x, line_top),
        ],
        egui::Stroke::new(stroke_width, color),
    );
}

impl eframe::App for RokuRemoteApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let window_rect = ctx.screen_rect();
        let has_user_input = ctx.input(|i| {
            i.raw.events.iter().any(|e| match e {
                egui::Event::PointerMoved(pos) => window_rect.contains(*pos),
                egui::Event::PointerButton { .. }
                | egui::Event::Key { .. }
                | egui::Event::Text(_)
                | egui::Event::MouseWheel { .. } => true,
                _ => false,
            })
        });

        let is_minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        let is_focused = is_hyprland_focused().unwrap_or_else(|| ctx.input(|i| i.focused));

        if has_user_input && !is_minimized {
            self.last_active = Instant::now();
        }

        let was_active = self.is_active.load(Ordering::Relaxed);
        let is_active_now = if is_minimized {
            false
        } else if !is_focused {
            self.last_active.elapsed() < Duration::from_secs(3)
        } else {
            self.last_active.elapsed() < Duration::from_secs(10)
        };

        if is_active_now != was_active {
            self.is_active.store(is_active_now, Ordering::Relaxed);
            if is_active_now {
                // Just woke up from sleep state: refresh device state immediately
                self.refresh_device_info();
            }
        }

        // When active and nearing idle timeout, request a repaint so state transition triggers
        if is_active_now {
            let timeout = if is_focused {
                Duration::from_secs(10)
            } else {
                Duration::from_secs(3)
            };
            if self.last_active.elapsed() < timeout {
                ctx.request_repaint_after(Duration::from_secs(1));
            }
        }

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
                            .color(self.theme.foreground),
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

                            ui.label("🔄");
                            ui.label(egui::RichText::new("Ctrl + Shift + R").strong().color(self.theme.accent));
                            ui.label(egui::RichText::new("Refresh Quick Launch Apps").color(self.theme.foreground));
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
                .default_width(380.0)
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
                            let device_type = if self.device_details.is_tv {
                                "Roku TV"
                            } else {
                                "Streaming Stick / Player"
                            };
                            row("Device Type:", device_type);
                            row("Model Name:", &self.device_details.model_name);
                            row("Model Number:", &self.device_details.model_number);
                            row("Location:", &self.device_details.user_location);
                            row("Software Version:", &self.device_details.software_version);
                            row("Wi-Fi Network:", &self.device_details.network_name);
                            row("Display Resolution:", &self.device_details.ui_resolution);
                            row("Power Mode:", &self.device_details.power_mode);
                            row("IP Address:", &self.selected_device_ip);

                            ui.label(egui::RichText::new("Mobile App Control:").strong().color(self.theme.accent));
                            let ecp_mode = if self.device_details.ecp_setting_mode.is_empty() {
                                "—"
                            } else {
                                &self.device_details.ecp_setting_mode
                            };
                            let (ecp_display, ecp_color) = match ecp_mode.to_lowercase().as_str() {
                                "limited" => ("Limited (Commands blocked)", egui::Color32::from_rgb(235, 150, 35)),
                                "disabled" => ("Disabled", egui::Color32::from_rgb(220, 60, 50)),
                                "permissive" | "default" => (ecp_mode, egui::Color32::from_rgb(46, 204, 113)),
                                _ => (ecp_mode, self.theme.foreground),
                            };
                            ui.label(egui::RichText::new(ecp_display).color(ecp_color));
                            ui.end_row();

                            let is_active = self.is_active.load(Ordering::Relaxed);
                            ui.label(egui::RichText::new("App Status:").strong().color(self.theme.accent));
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                let (icon_rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                                let center = icon_rect.center();
                                if is_active {
                                    // Glowing live dot
                                    ui.painter().circle_filled(center, 4.0, egui::Color32::from_rgb(46, 204, 113));
                                    ui.painter().circle_stroke(
                                        center,
                                        6.0,
                                        egui::Stroke::new(1.2f32, egui::Color32::from_rgba_premultiplied(46, 204, 113, 100)),
                                    );
                                    ui.label(
                                        egui::RichText::new("Live (Active)")
                                            .color(egui::Color32::from_rgb(46, 204, 113))
                                            .strong(),
                                    );
                                } else {
                                    // Sleeping/idle indicator ring with inner dot
                                    ui.painter().circle_stroke(
                                        center,
                                        4.5,
                                        egui::Stroke::new(1.5f32, egui::Color32::from_rgb(155, 168, 190)),
                                    );
                                    ui.painter().circle_filled(
                                        center,
                                        1.8,
                                        egui::Color32::from_rgb(155, 168, 190),
                                    );
                                    ui.label(
                                        egui::RichText::new("Sleeping (Idle)")
                                            .color(egui::Color32::from_rgb(155, 168, 190))
                                            .strong(),
                                    );
                                }
                            });
                            ui.end_row();
                        });

                    let is_limited = self.device_details.ecp_setting_mode.eq_ignore_ascii_case("limited")
                        || self.device_details.ecp_setting_mode.eq_ignore_ascii_case("disabled");
                    let is_unreachable = !self.is_device_reachable && !self.selected_device_ip.is_empty();

                    if is_limited || is_unreachable {
                        ui.add_space(6.0);
                        let (title, desc, border_color) = if is_limited {
                            (
                                "Mobile Control is Limited",
                                "Roku is rejecting remote commands.\nEnable 'Control by mobile apps' in Roku TV settings.",
                                egui::Color32::from_rgb(220, 150, 40),
                            )
                        } else {
                            (
                                "Roku Unreachable",
                                "Unable to communicate with Roku over Wi-Fi.\nCheck TV power and verify network connection.",
                                egui::Color32::from_rgb(220, 75, 65),
                            )
                        };

                        egui::Frame::none()
                            .fill(self.theme.lighter_background)
                            .stroke(egui::Stroke::new(1.0f32, border_color))
                            .rounding(6.0)
                            .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new("\u{26A0}")
                                            .size(16.0)
                                            .color(border_color),
                                    );
                                    ui.add_space(4.0);
                                    ui.vertical(|ui| {
                                        ui.label(
                                            egui::RichText::new(title)
                                                .strong()
                                                .size(12.0)
                                                .color(border_color),
                                        );
                                        ui.add_space(1.0);
                                        ui.label(
                                            egui::RichText::new(desc)
                                                .size(11.0)
                                                .color(self.theme.foreground),
                                        );
                                    });
                                });
                            });
                    }

                    ui.add_space(8.0);
                    ui.separator();
                    ui.horizontal(|ui| {
                        if is_limited || is_unreachable {
                            if ui.button("⚙ Setup Guide").clicked() {
                                self.show_setup_guide = true;
                            }
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Close").clicked() {
                                self.show_device_info = false;
                            }
                        });
                    });
                });
        }

        // Roku Setup Guide Modal Window
        if self.show_setup_guide {
            egui::Window::new("⚙ Roku Setup & Troubleshooting Guide")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .default_width(450.0)
                .show(ctx, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);

                    ui.heading("Connect & Enable Roku Remote Access");
                    ui.label(
                        egui::RichText::new("Follow these steps if your Roku is not discovered or commands are not responding:")
                            .color(self.theme.foreground),
                    );
                    ui.separator();

                    egui::ScrollArea::vertical()
                        .max_height(360.0)
                        .show(ui, |ui| {
                            // Section 1: Enable Mobile App Control (ECP)
                            ui.label(
                                egui::RichText::new("1. Enable Mobile App Control (ECP)")
                                    .strong()
                                    .size(13.5)
                                    .color(self.theme.accent),
                            );
                            ui.label(
                                egui::RichText::new("Roku requires external control permission. In 'Limited' mode, commands like Keypress and App launch are rejected by Roku:")
                                    .size(11.5)
                                    .color(self.theme.foreground),
                            );

                            egui::Frame::none()
                                .fill(self.theme.lighter_background)
                                .rounding(4.0)
                                .inner_margin(8.0)
                                .show(ui, |ui| {
                                    egui::Grid::new("setup_steps_grid")
                                        .spacing([8.0, 4.0])
                                        .show(ui, |ui| {
                                            ui.label(egui::RichText::new("Step 1:").strong().color(self.theme.accent));
                                            ui.label("Using your physical Roku remote, press the Home button.");
                                            ui.end_row();

                                            ui.label(egui::RichText::new("Step 2:").strong().color(self.theme.accent));
                                            ui.label("Navigate to Settings > System.");
                                            ui.end_row();

                                            ui.label(egui::RichText::new("Step 3:").strong().color(self.theme.accent));
                                            ui.label("Select Advanced system settings.");
                                            ui.end_row();

                                            ui.label(egui::RichText::new("Step 4:").strong().color(self.theme.accent));
                                            ui.label("Select Control by mobile apps > Network access.");
                                            ui.end_row();

                                            ui.label(egui::RichText::new("Step 5:").strong().color(self.theme.accent));
                                            ui.label("Select 'Default' or 'Permissive' (do not leave on 'Limited').");
                                            ui.end_row();
                                        });
                                });

                            ui.add_space(6.0);

                            // Section 2: Wi-Fi Network & Router
                            ui.label(
                                egui::RichText::new("2. Wi-Fi & Subnet Setup")
                                    .strong()
                                    .size(13.5)
                                    .color(self.theme.accent),
                            );
                            ui.label(
                                egui::RichText::new("• Ensure your computer and Roku are connected to the exact same Wi-Fi network.\n• Verify router does not have 'AP Isolation' / 'Client Isolation' enabled.")
                                    .size(11.5)
                                    .color(self.theme.foreground),
                            );

                            ui.add_space(6.0);

                            // Section 3: Manual IP Entry
                            ui.label(
                                egui::RichText::new("3. Find Your Roku IP Manually")
                                    .strong()
                                    .size(13.5)
                                    .color(self.theme.accent),
                            );
                            ui.label(
                                egui::RichText::new("If your router blocks discovery broadcasts:\n• On Roku: Settings > Network > About > IP address.\n• In this app: Select 'Enter IP manually' in the device selector dropdown.")
                                    .size(11.5)
                                    .color(self.theme.foreground),
                            );

                            ui.add_space(6.0);

                            // Section 4: TV Power & HDMI-CEC Control
                            ui.label(
                                egui::RichText::new("4. TV Power via Roku (HDMI-CEC)")
                                    .strong()
                                    .size(13.5)
                                    .color(self.theme.accent),
                            );
                            ui.label(
                                egui::RichText::new("Roku Streaming Sticks turn on the connected TV screen using HDMI-CEC:\n• On Roku: Settings > System > Control other devices (CEC) > Check '1-touch play'.\n• On your TV: Enable HDMI-CEC in your TV's settings menu (e.g. AnyNet+, Bravia Sync, SimpLink, CEC).")
                                    .size(11.5)
                                    .color(self.theme.foreground),
                            );

                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(4.0);

                            // Status Summary
                            ui.label(
                                egui::RichText::new("Current Device Status:")
                                    .strong()
                                    .color(self.theme.accent),
                            );

                            egui::Grid::new("setup_status_grid")
                                .spacing([10.0, 4.0])
                                .show(ui, |ui| {
                                    ui.label("Selected IP:");
                                    ui.label(egui::RichText::new(&self.selected_device_ip).monospace());
                                    ui.end_row();

                                    ui.label("Connection:");
                                    let (reach_label, reach_color) = if self.is_device_reachable {
                                        ("Reachable / Online", egui::Color32::from_rgb(46, 204, 113))
                                    } else {
                                        ("Unreachable / Offline", egui::Color32::from_rgb(220, 60, 50))
                                    };
                                    ui.label(egui::RichText::new(reach_label).strong().color(reach_color));
                                    ui.end_row();

                                    ui.label("Mobile App Control:");
                                    let mode_str = if self.device_details.ecp_setting_mode.is_empty() {
                                        "Unknown"
                                    } else {
                                        &self.device_details.ecp_setting_mode
                                    };
                                    let mode_color = match mode_str.to_lowercase().as_str() {
                                        "default" | "permissive" => egui::Color32::from_rgb(46, 204, 113),
                                        "limited" | "disabled" => egui::Color32::from_rgb(240, 160, 40),
                                        _ => self.theme.foreground,
                                    };
                                    ui.label(egui::RichText::new(mode_str).strong().color(mode_color));
                                    ui.end_row();
                                });
                        });

                    ui.add_space(8.0);
                    ui.separator();

                    ui.horizontal(|ui| {
                        if ui.button("🔍 Scan Network").clicked() {
                            self.is_scanning = true;
                            self.status_text = "Scanning network...".into();
                            self.start_discovery_scan();
                        }
                        if ui.button("🔄 Re-test Connection").clicked() {
                            self.refresh_device_info();
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Close").clicked() {
                                self.show_setup_guide = false;
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
                        .color(self.theme.foreground),
                );

                // Display Roku custom friendly name
                ui.label(
                    egui::RichText::new(format!("• {}", self.device_name))
                        .size(13.0)
                        .color(self.theme.dark_foreground),
                );

                let is_tv = self.device_details.is_tv;
                let is_powered_on = if !self.is_device_reachable {
                    false
                } else {
                    match self.device_details.power_mode.as_str() {
                        "PowerOn" => true,
                        "DisplayOff" | "Headless" => true,
                        "PowerOff" | "Standby" => false,
                        _ => self.is_device_reachable,
                    }
                };

                let (power_icon_color, power_status_label) = if !self.is_device_reachable {
                    (egui::Color32::from_rgb(220, 60, 50), "Offline")
                } else if !is_tv {
                    (egui::Color32::from_rgb(46, 204, 113), "Online")
                } else if is_powered_on {
                    (egui::Color32::from_rgb(46, 204, 113), "On")
                } else {
                    (egui::Color32::from_rgb(220, 60, 50), "Off")
                };

                let (badge_rect, _) = ui.allocate_exact_size(egui::vec2(13.0, 14.0), egui::Sense::hover());
                draw_power_icon(ui.painter(), badge_rect.center(), 4.5, power_icon_color, 1.6);
                ui.label(
                    egui::RichText::new(power_status_label)
                        .size(11.5)
                        .strong()
                        .color(power_icon_color),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.is_scanning {
                        ui.spinner();
                    }
                    if ui.add(egui::Button::new("Scan")).clicked() {
                        self.is_scanning = true;
                        self.status_text = "Scanning network...".into();
                        self.start_discovery_scan();
                    }

                    if is_tv {
                        let (power_label, power_bg, hover_bg) = if is_powered_on {
                            (
                                "Power Off",
                                egui::Color32::from_rgb(195, 55, 55),
                                egui::Color32::from_rgb(220, 68, 68),
                            )
                        } else {
                            (
                                "Power On",
                                egui::Color32::from_rgb(38, 150, 78),
                                egui::Color32::from_rgb(46, 172, 90),
                            )
                        };

                        let btn_size = egui::vec2(104.0, 26.0);
                        let (rect, response) = ui.allocate_exact_size(btn_size, egui::Sense::click());
                        let visuals = ui.style().interact(&response);

                        let bg = if response.is_pointer_button_down_on() {
                            if is_powered_on {
                                egui::Color32::from_rgb(170, 45, 45)
                            } else {
                                egui::Color32::from_rgb(30, 130, 65)
                            }
                        } else if response.hovered() {
                            hover_bg
                        } else {
                            power_bg
                        };

                        ui.painter().rect(rect, visuals.rounding, bg, visuals.bg_stroke);

                        let icon_center = egui::pos2(rect.min.x + 18.0, rect.center().y);
                        draw_power_icon(ui.painter(), icon_center, 4.8, egui::Color32::WHITE, 1.8);

                        let text_pos = egui::pos2(rect.min.x + 30.0, rect.center().y);
                        ui.painter().text(
                            text_pos,
                            egui::Align2::LEFT_CENTER,
                            power_label,
                            egui::FontId::proportional(12.0),
                            egui::Color32::WHITE,
                        );

                        if response.clicked() {
                            self.toggle_power();
                        }
                    } else {
                        // Streaming stick / external player: provide explicit Power Off and Power On buttons (HDMI-CEC)
                        // In right_to_left layout, add Power Off first, then Power On so Power On renders on the left
                        let off_btn_size = egui::vec2(84.0, 26.0);
                        let (off_rect, off_response) = ui.allocate_exact_size(off_btn_size, egui::Sense::click());
                        let off_vis = ui.style().interact(&off_response);
                        let off_bg = if off_response.is_pointer_button_down_on() {
                            egui::Color32::from_rgb(170, 45, 45)
                        } else if off_response.hovered() {
                            egui::Color32::from_rgb(220, 68, 68)
                        } else {
                            egui::Color32::from_rgb(195, 55, 55)
                        };
                        ui.painter().rect(off_rect, off_vis.rounding, off_bg, off_vis.bg_stroke);
                        let off_icon_center = egui::pos2(off_rect.min.x + 14.0, off_rect.center().y);
                        draw_power_icon(ui.painter(), off_icon_center, 4.2, egui::Color32::WHITE, 1.6);
                        let off_text_pos = egui::pos2(off_rect.min.x + 23.0, off_rect.center().y);
                        ui.painter().text(
                            off_text_pos,
                            egui::Align2::LEFT_CENTER,
                            "Power Off",
                            egui::FontId::proportional(11.5),
                            egui::Color32::WHITE,
                        );
                        if off_response.on_hover_text("Turn off TV screen (HDMI-CEC Standby)").clicked() {
                            self.power_off();
                        }

                        let on_btn_size = egui::vec2(84.0, 26.0);
                        let (on_rect, on_response) = ui.allocate_exact_size(on_btn_size, egui::Sense::click());
                        let on_vis = ui.style().interact(&on_response);
                        let on_bg = if on_response.is_pointer_button_down_on() {
                            egui::Color32::from_rgb(30, 130, 65)
                        } else if on_response.hovered() {
                            egui::Color32::from_rgb(46, 172, 90)
                        } else {
                            egui::Color32::from_rgb(38, 150, 78)
                        };
                        ui.painter().rect(on_rect, on_vis.rounding, on_bg, on_vis.bg_stroke);
                        let on_icon_center = egui::pos2(on_rect.min.x + 14.0, on_rect.center().y);
                        draw_power_icon(ui.painter(), on_icon_center, 4.2, egui::Color32::WHITE, 1.6);
                        let on_text_pos = egui::pos2(on_rect.min.x + 23.0, on_rect.center().y);
                        ui.painter().text(
                            on_text_pos,
                            egui::Align2::LEFT_CENTER,
                            "Power On",
                            egui::FontId::proportional(11.5),
                            egui::Color32::WHITE,
                        );
                        if on_response.on_hover_text("Turn on TV screen (HDMI-CEC 1-Touch Play)").clicked() {
                            self.power_on();
                        }
                    }

                    if ui.add(egui::Button::new("⚙ Setup")).clicked() {
                        self.show_setup_guide = !self.show_setup_guide;
                    }
                    if ui.add(egui::Button::new("Device Info")).clicked() {
                        self.show_device_info = !self.show_device_info;
                    }
                });
            });

            ui.add_space(2.0);

            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Roku:").color(self.theme.dark_foreground));

                if self.manual_ip_mode {
                    let text_edit = ui.add(
                        egui::TextEdit::singleline(&mut self.selected_device_ip)
                            .desired_width(120.0)
                            .hint_text("192.168.x.x"),
                    );
                    if text_edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.select_device(&self.selected_device_ip.clone());
                    }
                    if ui.button("Connect").clicked() {
                        self.select_device(&self.selected_device_ip.clone());
                    }
                    if ui.button("Discovered List").clicked() {
                        self.manual_ip_mode = false;
                    }
                } else {
                    let current_label = if let Some(d) = self.devices.iter().find(|d| d.ip == self.selected_device_ip) {
                        format!("📺 {} ({})", d.name, d.ip)
                    } else if !self.selected_device_ip.is_empty() {
                        format!("📺 Custom ({})", self.selected_device_ip)
                    } else if self.is_scanning {
                        "Searching for devices...".to_string()
                    } else {
                        "No Roku detected".to_string()
                    };

                    let mut newly_selected_ip = None;
                    let mut switch_to_manual = false;

                    egui::ComboBox::from_id_salt("roku_device_combo")
                        .selected_text(current_label)
                        .width(220.0)
                        .show_ui(ui, |ui| {
                            if self.devices.is_empty() {
                                let empty_msg = if self.is_scanning {
                                    "⏳ Scanning network..."
                                } else {
                                    "No Rokus found on network"
                                };
                                ui.label(egui::RichText::new(empty_msg).color(self.theme.dark_foreground));
                            } else {
                                for dev in &self.devices {
                                    let is_current = dev.ip == self.selected_device_ip;
                                    let item_label = format!("📺 {} ({})", dev.name, dev.ip);
                                    if ui.selectable_label(is_current, item_label).clicked() {
                                        newly_selected_ip = Some(dev.ip.clone());
                                    }
                                }
                            }
                            ui.separator();
                            if ui.selectable_label(false, "+ Enter IP manually...").clicked() {
                                switch_to_manual = true;
                            }
                        });

                    if let Some(ip) = newly_selected_ip {
                        self.select_device(&ip);
                    }
                    if switch_to_manual {
                        self.manual_ip_mode = true;
                    }

                    if ui.button("🔄").on_hover_text("Refresh connection & device status").clicked() {
                        self.refresh_device_info();
                    }
                }

                ui.add_space(8.0);
                ui.label(egui::RichText::new("Current:").color(self.theme.dark_foreground));
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
                    ui.label(
                        egui::RichText::new(state_icon)
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
                    ui.label(
                        egui::RichText::new(&self.status_text)
                            .color(self.theme.dark_foreground)
                            .size(11.0),
                    );
                });
            });

            // Banner for Limited / Unreachable Mode
            let is_limited = self.is_device_reachable
                && (self.device_details.ecp_setting_mode.eq_ignore_ascii_case("limited")
                    || self.device_details.ecp_setting_mode.eq_ignore_ascii_case("disabled"));

            if is_limited {
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("\u{26A0} Roku Mobile App Control is 'Limited' (remote keypresses blocked).")
                            .color(egui::Color32::from_rgb(240, 160, 40))
                            .size(12.0)
                            .strong(),
                    );
                    if ui.button(egui::RichText::new("⚙ Setup Instructions").color(egui::Color32::from_rgb(240, 160, 40))).clicked() {
                        self.show_setup_guide = true;
                    }
                });
            } else if !self.is_device_reachable && !self.is_scanning && !self.selected_device_ip.is_empty() {
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("\u{26A0} Roku unreachable at {}. Verify network & power.", self.selected_device_ip))
                            .color(egui::Color32::from_rgb(220, 80, 70))
                            .size(12.0),
                    );
                    if ui.button("⚙ Setup Guide").clicked() {
                        self.show_setup_guide = true;
                    }
                });
            }

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
