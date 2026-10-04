use eframe::egui;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::models::{
    default_popular_apps, AppItem, BackgroundMessage, DeviceDetails, MediaPlayerInfo, RokuDevice,
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

        let shared_ip = Arc::new(Mutex::new("192.168.0.108".to_string()));
        Self::start_playback_watcher(
            shared_ip.clone(),
            tx.clone(),
            cc.egui_ctx.clone(),
            is_active.clone(),
        );

        let initial_apps = crate::roku::client::load_cached_apps()
            .unwrap_or_else(default_popular_apps);

        let app = Self {
            devices: Vec::new(),
            selected_device_ip: "192.168.0.108".to_string(),
            shared_ip,
            is_active,
            last_active: Instant::now(),
            device_name: "Roku Streaming Stick Plus".to_string(),
            active_app: "Loading...".to_string(),
            media_player: MediaPlayerInfo::default(),
            device_details: DeviceDetails::default(),
            is_device_reachable: true,
            show_device_info: false,
            apps: initial_apps.clone(),
            app_textures: HashMap::new(),
            pending_icons: Vec::new(),
            is_scanning: false,
            is_refreshing_apps: false,
            status_text: "Ready".to_string(),
            show_shortcuts: false,
            theme,
            ctx: cc.egui_ctx.clone(),
            rx,
            tx,
        };

        // Immediately load any cached icons for the apps list so they are ready on launch
        app.fetch_app_icons(&initial_apps);

        app.start_discovery_scan();
        app.refresh_device_info();

        app
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

    pub fn toggle_power(&mut self) {
        let currently_on = self.is_device_reachable
            && self.device_details.power_mode != "PowerOff"
            && self.device_details.power_mode != "Standby";
        // Optimistic UI update: flip immediately so the user sees instant feedback
        self.is_device_reachable = !currently_on;
        if !currently_on {
            self.device_details.power_mode = "PowerOn".to_string();
        } else {
            self.device_details.power_mode = "PowerOff".to_string();
        }

        let ip = self.selected_device_ip.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();

        thread::spawn(move || {
            let key = if currently_on {
                "PowerOff"
            } else {
                "PowerOn"
            };

            // Attempt primary key (PowerOff or PowerOn)
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_millis(1200))
                .build();

            let mut sent = false;
            if let Ok(ref c) = client {
                let url = format!("http://{}:8060/keypress/{}", ip, key);
                if let Ok(resp) = c.post(&url).send() {
                    if resp.status().is_success() {
                        sent = true;
                    }
                }
                // Fallback to standard toggle "Power" key if device didn't accept PowerOn/PowerOff
                if !sent {
                    let fallback_url = format!("http://{}:8060/keypress/Power", ip);
                    let _ = c.post(&fallback_url).send();
                }
            }

            // Progressive validation checks at 600ms, 1600ms, and 3200ms
            for delay in [600, 1000, 1600] {
                thread::sleep(Duration::from_millis(delay));
                update_device_name_worker(&ip, &tx, &ctx);
                update_media_player_worker(&ip, &tx, &ctx);
            }
        });
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
                BackgroundMessage::AppsListRefreshed(apps) => {
                    self.is_refreshing_apps = false;
                    let count = apps.len();
                    self.fetch_app_icons(&apps);
                    self.apps = apps;
                    self.status_text = format!("Updated {} apps", count);
                }
                BackgroundMessage::AppsRefreshFailed => {
                    self.is_refreshing_apps = false;
                    self.status_text = "Failed to refresh apps".to_string();
                }
                BackgroundMessage::ScanFinished => {
                    self.is_scanning = false;
                    self.status_text = format!("Found {} device(s)", self.devices.len());
                }
                BackgroundMessage::ThemeUpdated(theme) => {
                    apply_theme(ctx, &theme);
                    self.theme = theme;
                }
                BackgroundMessage::MediaPlayerUpdated(info) => {
                    self.media_player = info;
                }
                BackgroundMessage::DeviceDetailsUpdated(details) => {
                    self.device_details = details;
                }
                BackgroundMessage::PowerStateUpdated(reachable) => {
                    self.is_device_reachable = reachable;
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

                ui.label(
                    egui::RichText::new(format!("{} apps •", self.apps.len()))
                        .size(11.0)
                        .color(self.theme.dark_foreground),
                );
            });
        });
        if do_refresh_apps {
            self.refresh_apps();
        }
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

        // Escape closes shortcuts dialog if open
        if key_escape && self.show_shortcuts {
            self.show_shortcuts = false;
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

                            let is_active = self.is_active.load(Ordering::Relaxed);
                            ui.label(egui::RichText::new("App Status:").strong().color(self.theme.accent));
                            if is_active {
                                ui.label(egui::RichText::new("● Live (Active)").color(egui::Color32::from_rgb(50, 185, 90)).strong());
                            } else {
                                ui.label(egui::RichText::new("💤 Sleeping (Idle)").color(egui::Color32::from_rgb(150, 160, 180)).strong());
                            }
                            ui.end_row();
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
                        .color(self.theme.foreground),
                );

                // Display Roku custom friendly name
                ui.label(
                    egui::RichText::new(format!("• {}", self.device_name))
                        .size(13.0)
                        .color(self.theme.dark_foreground),
                );

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

                let (power_dot_color, power_status_label) = if is_powered_on {
                    (egui::Color32::from_rgb(46, 204, 113), "On")
                } else {
                    (egui::Color32::from_rgb(220, 60, 50), "Off")
                };

                ui.label(egui::RichText::new("●").color(power_dot_color).size(10.0));
                ui.label(
                    egui::RichText::new(power_status_label)
                        .size(11.5)
                        .strong()
                        .color(power_dot_color),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new("Scan")).clicked() {
                        self.is_scanning = true;
                        self.status_text = "Scanning network...".into();
                        self.start_discovery_scan();
                    }

                    let (power_text, power_bg) = if is_powered_on {
                        (
                            "⏻ Power Off",
                            egui::Color32::from_rgb(195, 55, 55), // Red button to power off / shut down
                        )
                    } else {
                        (
                            "⏻ Power On",
                            egui::Color32::from_rgb(38, 150, 78), // Green button to power on
                        )
                    };

                    let power_btn = egui::Button::new(
                        egui::RichText::new(power_text)
                            .strong()
                            .color(egui::Color32::WHITE),
                    )
                    .fill(power_bg);

                    if ui.add(power_btn).clicked() {
                        self.toggle_power();
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
                        .desired_width(120.0),
                );
                if text_edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.refresh_device_info();
                }
                if ui.button("Connect").clicked() {
                    self.refresh_device_info();
                }

                ui.add_space(10.0);
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
