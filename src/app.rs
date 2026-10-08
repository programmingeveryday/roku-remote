use eframe::egui;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::models::{
    AppItem, BackgroundMessage, DeviceDetails, DeviceStats, LiveKeyCommand, MediaPlayerInfo,
    RokuDevice,
};
use crate::roku::client::{
    fetch_device_stats_worker, load_app_icon_worker, refresh_apps_worker, start_scan,
    update_active_app_worker, update_device_name_worker, update_media_player_worker,
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
    pub tv_powered_on: Arc<AtomicBool>,
    pub show_device_info: bool,
    pub show_device_stats: bool,
    pub show_setup_guide: bool,
    pub manual_ip_mode: bool,
    pub apps: Vec<AppItem>,
    pub app_textures: HashMap<String, egui::TextureHandle>,
    pub pending_icons: Vec<(String, egui::ColorImage)>,
    pub is_scanning: bool,
    pub is_refreshing_apps: bool,
    pub status_text: String,
    pub show_shortcuts: bool,
    pub pending_restore_to_min: u8,
    pub is_always_on_top: bool,
    pub show_text_dialog: bool,
    pub focus_text_input: bool,
    pub text_entry: String,
    pub live_key_tx: Sender<LiveKeyCommand>,
    pub device_stats: DeviceStats,
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

        let (live_key_tx, live_key_rx) = channel::<LiveKeyCommand>();
        let live_key_ip = shared_ip.clone();
        thread::spawn(move || {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_millis(1500))
                .build()
                .ok();
            while let Ok(cmd) = live_key_rx.recv() {
                let ip = if let Ok(guard) = live_key_ip.lock() {
                    guard.clone()
                } else {
                    continue;
                };
                if ip.is_empty() {
                    continue;
                }
                if let Some(ref c) = client {
                    match cmd {
                        LiveKeyCommand::Char(ch) => {
                            let lit = crate::roku::client::encode_char_for_lit(ch);
                            let url = format!("http://{}:8060/keypress/{}", ip, lit);
                            let _ = c.post(&url).send();
                            thread::sleep(Duration::from_millis(25));
                        }
                        LiveKeyCommand::Backspace => {
                            let url = format!("http://{}:8060/keypress/Backspace", ip);
                            let _ = c.post(&url).send();
                            thread::sleep(Duration::from_millis(25));
                        }
                        LiveKeyCommand::Clear(count) => {
                            let url = format!("http://{}:8060/keypress/Backspace", ip);
                            for _ in 0..count {
                                let _ = c.post(&url).send();
                                thread::sleep(Duration::from_millis(20));
                            }
                        }
                        LiveKeyCommand::Key(key) => {
                            let url = format!("http://{}:8060/keypress/{}", ip, key);
                            let _ = c.post(&url).send();
                        }
                    }
                }
            }
        });

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
            tv_powered_on: Arc::new(AtomicBool::new(true)),
            show_device_info: false,
            show_device_stats: false,
            show_setup_guide: false,
            manual_ip_mode: false,
            apps: Vec::new(),
            app_textures: HashMap::new(),
            pending_icons: Vec::new(),
            is_scanning: true,
            is_refreshing_apps: true,
            status_text: "Discovering Rokus...".to_string(),
            show_shortcuts: false,
            pending_restore_to_min: 0,
            is_always_on_top: false,
            show_text_dialog: false,
            focus_text_input: false,
            text_entry: String::new(),
            live_key_tx,
            device_stats: DeviceStats::default(),
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
        self.tv_powered_on.store(true, Ordering::Relaxed);
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

    pub fn replay(&self) {
        self.tv_powered_on.store(true, Ordering::Relaxed);
        let ip = self.selected_device_ip.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        thread::spawn(move || {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_millis(1000))
                .build();
            if let Ok(c) = client {
                // Hit rewind 5 times (~5 seconds back)
                for _ in 0..5 {
                    let rev_url = format!("http://{}:8060/keypress/Rev", ip);
                    let _ = c.post(&rev_url).send();
                    thread::sleep(Duration::from_millis(80));
                }
                // Resume playback
                let play_url = format!("http://{}:8060/keypress/Play", ip);
                let _ = c.post(&play_url).send();
            }
            thread::sleep(Duration::from_millis(800));
            update_active_app_worker(&ip, &tx, &ctx);
            update_media_player_worker(&ip, &tx, &ctx);
        });
    }

    pub fn send_key(&self, key: &'static str) {
        if key == "InstantReplay" || key == "Replay" {
            self.replay();
            return;
        }
        if key != "Power" && key != "PowerOff" {
            self.tv_powered_on.store(true, Ordering::Relaxed);
        }
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

    pub fn send_text(&self, text: &str, submit_enter: bool) {
        if text.is_empty() && !submit_enter {
            return;
        }
        self.tv_powered_on.store(true, Ordering::Relaxed);
        let ip = self.selected_device_ip.clone();
        let text = text.to_string();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();

        thread::spawn(move || {
            let client = match reqwest::blocking::Client::builder()
                .timeout(Duration::from_millis(1500))
                .build()
            {
                Ok(c) => c,
                Err(_) => return,
            };

            for ch in text.chars().filter(|c| *c != '\r' && *c != '\n') {
                let lit = crate::roku::client::encode_char_for_lit(ch);
                let url = format!("http://{}:8060/keypress/{}", ip, lit);
                let _ = client.post(&url).send();
                thread::sleep(Duration::from_millis(45));
            }

            if submit_enter {
                if !text.is_empty() {
                    thread::sleep(Duration::from_millis(60));
                }
                let url = format!("http://{}:8060/keypress/Enter", ip);
                let _ = client.post(&url).send();
            }

            thread::sleep(Duration::from_millis(600));
            update_active_app_worker(&ip, &tx, &ctx);
            update_media_player_worker(&ip, &tx, &ctx);
        });
    }

    pub fn send_char(&self, ch: char) {
        if ch == '\r' || ch == '\n' {
            return;
        }
        self.tv_powered_on.store(true, Ordering::Relaxed);
        let _ = self.live_key_tx.send(LiveKeyCommand::Char(ch));
    }

    pub fn send_backspace_fast(&self) {
        self.tv_powered_on.store(true, Ordering::Relaxed);
        let _ = self.live_key_tx.send(LiveKeyCommand::Backspace);
    }

    pub fn clear_text_on_roku(&self, count: usize) {
        if count == 0 {
            return;
        }
        self.tv_powered_on.store(true, Ordering::Relaxed);
        let _ = self.live_key_tx.send(LiveKeyCommand::Clear(count));
    }

    pub fn send_live_key(&self, key: &'static str) {
        self.tv_powered_on.store(true, Ordering::Relaxed);
        let _ = self.live_key_tx.send(LiveKeyCommand::Key(key));
    }

    pub fn handle_live_text_diff(&mut self, prev_text: &str, ui: &mut egui::Ui) {
        let delete_pressed = ui.input(|i| i.key_pressed(egui::Key::Delete));
        let backspace_pressed = ui.input(|i| i.key_pressed(egui::Key::Backspace));
        let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));
        let esc_pressed = ui.input(|i| i.key_pressed(egui::Key::Escape));

        if esc_pressed {
            self.show_text_dialog = false;
            return;
        }

        if delete_pressed {
            let count = prev_text.chars().count().max(self.text_entry.chars().count());
            self.text_entry.clear();
            if count > 0 {
                self.clear_text_on_roku(count);
            }
            return;
        }

        if enter_pressed {
            self.send_live_key("Enter");
        }

        if self.text_entry != prev_text {
            if self.text_entry.starts_with(prev_text) {
                // Characters added at the end (typed or pasted)
                let added = &self.text_entry[prev_text.len()..];
                for ch in added.chars() {
                    self.send_char(ch);
                }
            } else if prev_text.starts_with(&self.text_entry) {
                // Characters deleted from the end
                let removed_count = prev_text.chars().count().saturating_sub(self.text_entry.chars().count());
                for _ in 0..removed_count {
                    self.send_backspace_fast();
                }
            } else {
                // Text replaced (e.g. selection overwritten)
                let old_count = prev_text.chars().count();
                self.clear_text_on_roku(old_count);
                for ch in self.text_entry.chars() {
                    self.send_char(ch);
                }
            }
        } else if backspace_pressed && prev_text.is_empty() {
            // Buffer was already empty, but user hit backspace to delete TV content
            self.send_backspace_fast();
        }
    }

    pub fn render_live_keyboard_content(&mut self, ui: &mut egui::Ui, _available_width: f32) {
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);

        // Header Row
        ui.horizontal(|ui| {
            ui.heading(
                egui::RichText::new("Live Roku Keyboard")
                    .size(16.0)
                    .color(self.theme.foreground),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close (Esc)").clicked() {
                    self.show_text_dialog = false;
                }
            });
        });

        // Live Mode Status Banner
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 4.0, egui::Color32::from_rgb(46, 204, 113));
            ui.label(
                egui::RichText::new("Live Typing: Keystrokes transmit to TV as you type")
                    .size(11.5)
                    .color(egui::Color32::from_rgb(46, 204, 113)),
            );
        });

        ui.separator();
        ui.add_space(2.0);

        // Prominent Text Input Box
        let prev_text = self.text_entry.clone();
        let edit = egui::TextEdit::singleline(&mut self.text_entry)
            .hint_text("Start typing search, password, or URL...")
            .desired_width(ui.available_width());
        let response = ui.add(edit);

        if self.focus_text_input {
            response.request_focus();
            self.focus_text_input = false;
        }

        self.handle_live_text_diff(&prev_text, ui);

        ui.add_space(4.0);

        // Dedicated Action Buttons: Backspace, Clear All (Del), Enter
        ui.horizontal(|ui| {
            let btn_w = ((ui.available_width() - 16.0) / 3.0).max(60.0);
            let btn_size = egui::vec2(btn_w, 32.0);

            if ui.add_sized(btn_size, egui::Button::new("Backspace"))
                .on_hover_text("Delete previous character on Roku (Backspace key)")
                .clicked()
            {
                if !self.text_entry.is_empty() {
                    self.text_entry.pop();
                }
                self.send_backspace_fast();
            }

            if ui.add_sized(btn_size, egui::Button::new("Clear All (Del)"))
                .on_hover_text("Clear entire text field on Roku (Delete key)")
                .clicked()
            {
                let count = self.text_entry.chars().count();
                self.text_entry.clear();
                self.clear_text_on_roku(count);
            }

            let enter_btn = egui::Button::new(
                egui::RichText::new("Enter")
                    .strong()
                    .color(egui::Color32::WHITE),
            )
            .fill(self.theme.roku_purple);
            if ui.add_sized(btn_size, enter_btn)
                .on_hover_text("Send Enter key to submit search or select (Enter key)")
                .clicked()
            {
                self.send_live_key("Enter");
            }
        });

        ui.add_space(8.0);

        // Helpful Keyboard Cheat Sheet Card
        egui::Frame::none()
            .fill(self.theme.lighter_background)
            .rounding(6.0)
            .inner_margin(egui::Margin::symmetric(10.0, 8.0))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("Live Keyboard Controls:")
                        .strong()
                        .size(11.5)
                        .color(self.theme.accent),
                );
                ui.add_space(2.0);
                egui::Grid::new("live_keyboard_tips_grid")
                    .spacing([10.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("Type:").strong().color(self.theme.accent).size(11.0));
                        ui.label(egui::RichText::new("Every key you press is sent to TV in real-time").color(self.theme.foreground).size(11.0));
                        ui.end_row();

                        ui.label(egui::RichText::new("Backspace:").strong().color(self.theme.accent).size(11.0));
                        ui.label(egui::RichText::new("Deletes the last character on TV").color(self.theme.foreground).size(11.0));
                        ui.end_row();

                        ui.label(egui::RichText::new("Delete:").strong().color(self.theme.accent).size(11.0));
                        ui.label(egui::RichText::new("Clears the entire text field on TV").color(self.theme.foreground).size(11.0));
                        ui.end_row();

                        ui.label(egui::RichText::new("Enter:").strong().color(self.theme.accent).size(11.0));
                        ui.label(egui::RichText::new("Submits search or activates selected item").color(self.theme.foreground).size(11.0));
                        ui.end_row();

                    });
            });
    }

    pub fn fetch_device_stats(&mut self) {
        if self.selected_device_ip.is_empty() { return; }
        self.device_stats.is_loading = true;
        let ip = self.selected_device_ip.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        thread::spawn(move || {
            fetch_device_stats_worker(&ip, &tx, &ctx);
        });
    }

    pub fn render_sparkline_graph(
        &self,
        ui: &mut egui::Ui,
        title: &str,
        current_val_str: &str,
        history: &[f32],
        max_bound: f32,
        line_color: egui::Color32,
        height: f32,
    ) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(title)
                    .strong()
                    .size(12.5)
                    .color(self.theme.foreground),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(current_val_str)
                        .strong()
                        .size(13.0)
                        .color(line_color),
                );
            });
        });

        let available_w = ui.available_width().max(160.0);
        let (response, painter) = ui.allocate_painter(
            egui::vec2(available_w, height),
            egui::Sense::hover(),
        );
        let rect = response.rect;

        painter.rect_filled(rect, 6.0, self.theme.lighter_background);
        painter.rect_stroke(rect, 6.0, egui::Stroke::new(1.0_f32, self.theme.dark_background));

        if history.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Awaiting Telemetry Samples...",
                egui::FontId::proportional(11.0),
                self.theme.dark_foreground,
            );
            return;
        }

        let grid_stroke = egui::Stroke::new(0.8_f32, self.theme.dark_background);
        for fraction in [0.25f32, 0.5f32, 0.75f32] {
            let y = rect.bottom() - (rect.height() * fraction);
            painter.line_segment(
                [egui::pos2(rect.left() + 4.0, y), egui::pos2(rect.right() - 4.0, y)],
                grid_stroke,
            );
        }

        let padding = 8.0f32;
        let plot_w = (rect.width() - (padding * 2.0)).max(10.0);
        let plot_h = (rect.height() - (padding * 2.0)).max(10.0);

        let max_y = max_bound.max(history.iter().copied().fold(1.0f32, f32::max));
        let n = history.len();
        let dx = if n > 1 { plot_w / (n - 1) as f32 } else { plot_w };

        let mut points: Vec<egui::Pos2> = Vec::with_capacity(n);
        for (i, &val) in history.iter().enumerate() {
            let norm_y = (val / max_y).clamp(0.0, 1.0);
            let x = rect.left() + padding + (i as f32 * dx);
            let y = rect.bottom() - padding - (norm_y * plot_h);
            points.push(egui::pos2(x, y));
        }

        if points.len() >= 2 {
            let mut poly = points.clone();
            poly.push(egui::pos2(points.last().unwrap().x, rect.bottom() - padding));
            poly.push(egui::pos2(points.first().unwrap().x, rect.bottom() - padding));

            let fill_color = egui::Color32::from_rgba_premultiplied(
                line_color.r() / 5,
                line_color.g() / 5,
                line_color.b() / 5,
                40,
            );
            painter.add(egui::Shape::convex_polygon(poly, fill_color, egui::Stroke::NONE));

            painter.add(egui::Shape::line(
                points,
                egui::Stroke::new(1.8f32, line_color),
            ));
        } else if let Some(&p) = points.first() {
            painter.circle_filled(p, 3.5, line_color);
        }
    }

    pub fn render_device_stats_content(&mut self, ui: &mut egui::Ui, _content_width: f32) {
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);

        // Header Row
        ui.horizontal(|ui| {
            ui.heading(
                egui::RichText::new("📊 Roku Stats & Telemetry")
                    .color(self.theme.foreground)
                    .size(16.0),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close (Esc)").clicked() {
                    self.show_device_stats = false;
                }
                if self.device_stats.is_loading {
                    ui.spinner();
                } else if ui.button("🔄 Refresh").clicked() {
                    self.fetch_device_stats();
                }
                if ui.button("ℹ Device Info").clicked() {
                    self.show_device_stats = false;
                    self.show_device_info = true;
                }
            });
        });

        ui.separator();

        // Running App & Target Device Banner
        egui::Frame::none()
            .fill(self.theme.lighter_background)
            .rounding(6.0)
            .inner_margin(egui::Margin::symmetric(10.0, 6.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Target:").strong().size(12.0).color(self.theme.accent));
                    ui.label(egui::RichText::new(&self.device_name).size(12.0).color(self.theme.foreground));
                    ui.label(egui::RichText::new(format!("({})", self.selected_device_ip)).size(11.0).color(self.theme.dark_foreground));

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let state_str = match self.media_player.state.as_str() {
                            "play" => "▶ Playing",
                            "pause" => "⏸ Paused",
                            "buffer" => "⏳ Buffering",
                            _ => "⏹ Idle",
                        };
                        let state_color = match self.media_player.state.as_str() {
                            "play" => egui::Color32::from_rgb(46, 204, 113),
                            "pause" => egui::Color32::from_rgb(241, 196, 15),
                            "buffer" => egui::Color32::from_rgb(52, 152, 219),
                            _ => self.theme.dark_foreground,
                        };
                        ui.label(egui::RichText::new(state_str).strong().size(11.5).color(state_color));

                        let app_display = if self.active_app.is_empty() { "Home / System" } else { &self.active_app };
                        ui.label(egui::RichText::new(app_display).strong().size(12.0).color(self.theme.accent));
                        ui.label(egui::RichText::new("App:").size(12.0).color(self.theme.foreground));
                    });
                });
            });

        ui.add_space(2.0);

        // 1. PRIMARY LIVE GRAPH: Network Streaming Bandwidth (Mbps)
        let latest_mbps = self.device_stats.bandwidth_history.last().copied().unwrap_or(0.0);
        let bw_val_str = if latest_mbps > 0.0 {
            if let Some(bps) = self.media_player.video_bitrate_bps {
                let v_mbps = (bps as f32) / 1_000_000.0;
                format!("{:.1} Mbps (video: {:.1} Mbps)", latest_mbps, v_mbps)
            } else {
                format!("{:.1} Mbps", latest_mbps)
            }
        } else if self.media_player.state == "play" || self.media_player.state == "pause" {
            "Active (Measuring...)".to_string()
        } else {
            "0.0 Mbps (Idle / No Stream)".to_string()
        };

        let max_bw = self.device_stats.bandwidth_history.iter().copied().fold(50.0f32, f32::max).max(20.0);
        self.render_sparkline_graph(
            ui,
            "📡 Network Streaming Bandwidth",
            &bw_val_str,
            &self.device_stats.bandwidth_history,
            max_bw,
            egui::Color32::from_rgb(52, 152, 219),
            65.0,
        );

        ui.add_space(4.0);

        // 2. Active Media Stream Telemetry (if video/audio is active)
        let has_stream_info = !self.media_player.video_res.is_empty()
            || !self.media_player.video_codec.is_empty()
            || !self.media_player.audio_codec.is_empty()
            || self.media_player.bandwidth_bps.is_some();

        if has_stream_info {
            egui::Frame::none()
                .fill(self.theme.lighter_background)
                .rounding(6.0)
                .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("🎬 Live Stream Telemetry").strong().size(12.0).color(self.theme.accent));
                    ui.add_space(3.0);
                    egui::Grid::new("stream_telemetry_grid")
                        .spacing([12.0, 5.0])
                        .show(ui, |ui| {
                            if !self.media_player.video_res.is_empty() {
                                ui.label(egui::RichText::new("Stream Resolution:").strong().size(11.5).color(self.theme.accent));
                                ui.label(egui::RichText::new(&self.media_player.video_res).size(11.5).color(self.theme.foreground));
                                ui.end_row();
                            }
                            if !self.media_player.video_codec.is_empty() {
                                ui.label(egui::RichText::new("Video Codec:").strong().size(11.5).color(self.theme.accent));
                                ui.label(egui::RichText::new(&self.media_player.video_codec).size(11.5).color(self.theme.foreground));
                                ui.end_row();
                            }
                            if !self.media_player.audio_codec.is_empty() {
                                ui.label(egui::RichText::new("Audio Codec:").strong().size(11.5).color(self.theme.accent));
                                ui.label(egui::RichText::new(&self.media_player.audio_codec).size(11.5).color(self.theme.foreground));
                                ui.end_row();
                            }
                            if !self.media_player.container.is_empty() {
                                ui.label(egui::RichText::new("Container / Protocol:").strong().size(11.5).color(self.theme.accent));
                                ui.label(egui::RichText::new(&self.media_player.container).size(11.5).color(self.theme.foreground));
                                ui.end_row();
                            }
                            if let (Some(cur), Some(max)) = (self.media_player.buffer_current, self.media_player.buffer_max) {
                                ui.label(egui::RichText::new("Buffer Health:").strong().size(11.5).color(self.theme.accent));
                                let pct = if max > 0 { (cur as f32 / max as f32) * 100.0 } else { 100.0 };
                                ui.label(egui::RichText::new(format!("{:.0}% full ({}/{} ms)", pct, cur, max)).size(11.5).color(self.theme.foreground));
                                ui.end_row();
                            }
                            if let (Some(pos), Some(dur)) = (self.media_player.position_ms, self.media_player.duration_ms) {
                                if dur > 0 {
                                    let pos_s = pos / 1000;
                                    let dur_s = dur / 1000;
                                    ui.label(egui::RichText::new("Playback Progress:").strong().size(11.5).color(self.theme.accent));
                                    let prog_str = format!("{}:{:02} / {}:{:02} ({:.0}%)", pos_s / 60, pos_s % 60, dur_s / 60, dur_s % 60, (pos as f32 / dur as f32) * 100.0);
                                    ui.label(egui::RichText::new(prog_str).size(11.5).color(self.theme.foreground));
                                    ui.end_row();
                                }
                            }
                        });
                });
            ui.add_space(4.0);
        }

        // 3. Hardware & System Diagnostics
        egui::Frame::none()
            .fill(self.theme.lighter_background)
            .rounding(6.0)
            .inner_margin(egui::Margin::symmetric(10.0, 8.0))
            .show(ui, |ui| {
                ui.label(egui::RichText::new("⚙ Hardware & System Diagnostics").strong().size(12.0).color(self.theme.accent));
                ui.add_space(3.0);
                egui::Grid::new("stats_hardware_grid")
                    .spacing([12.0, 5.0])
                    .show(ui, |ui| {
                        // System Uptime
                        ui.label(egui::RichText::new("System Uptime:").strong().size(11.5).color(self.theme.accent));
                        let uptime_s = self.device_details.uptime_seconds;
                        let uptime_str = if uptime_s == 0 {
                            "—".to_string()
                        } else {
                            let days = uptime_s / 86400;
                            let hours = (uptime_s % 86400) / 3600;
                            let mins = (uptime_s % 3600) / 60;
                            let secs = uptime_s % 60;
                            if days > 0 {
                                format!("{}d {}h {}m", days, hours, mins)
                            } else if hours > 0 {
                                format!("{}h {}m {}s", hours, mins, secs)
                            } else {
                                format!("{}m {}s", mins, secs)
                            }
                        };
                        ui.label(egui::RichText::new(uptime_str).size(11.5).color(self.theme.foreground));
                        ui.end_row();

                        // Power Source
                        ui.label(egui::RichText::new("Power Source:").strong().size(11.5).color(self.theme.accent));
                        let power_src = if self.device_details.is_powered_by_tv {
                            "USB Port (Powered by TV)"
                        } else {
                            "External AC Power Adapter"
                        };
                        ui.label(egui::RichText::new(power_src).size(11.5).color(self.theme.foreground));
                        ui.end_row();

                        // Wi-Fi Hardware
                        ui.label(egui::RichText::new("Wi-Fi Hardware:").strong().size(11.5).color(self.theme.accent));
                        let wifi_str = format!("Driver: {} (5GHz Band: {})", 
                            if self.device_details.wifi_driver.is_empty() { "standard" } else { &self.device_details.wifi_driver },
                            if self.device_details.has_wifi_5g { "Supported" } else { "No" }
                        );
                        ui.label(egui::RichText::new(wifi_str).size(11.5).color(self.theme.foreground));
                        ui.end_row();

                        // MAC Addresses
                        if !self.device_details.wifi_mac.is_empty() {
                            ui.label(egui::RichText::new("MAC Address:").strong().size(11.5).color(self.theme.accent));
                            let mac_str = if !self.device_details.bluetooth_mac.is_empty() {
                                format!("Wi-Fi: {} | BT: {}", self.device_details.wifi_mac, self.device_details.bluetooth_mac)
                            } else {
                                self.device_details.wifi_mac.clone()
                            };
                            ui.label(egui::RichText::new(mac_str).size(11.5).color(self.theme.foreground));
                            ui.end_row();
                        }

                        // Firmware Build
                        if !self.device_details.build_number.is_empty() {
                            ui.label(egui::RichText::new("OS Build:").strong().size(11.5).color(self.theme.accent));
                            let build_str = format!("Roku OS {} (Build {})", self.device_details.software_version, self.device_details.build_number);
                            ui.label(egui::RichText::new(build_str).size(11.5).color(self.theme.foreground));
                            ui.end_row();
                        }
                    });
            });

        ui.add_space(4.0);

        // 4. Developer Mode CPU & RAM Profiling
        if self.device_details.developer_enabled {
            // DEVELOPER MODE IS ACTIVE: Show live CPU and RAM graphs!
            let cpu_val_str = self.device_stats.chanperf.as_ref().map_or_else(
                || if self.device_stats.cpu_history.is_empty() { "— %".to_string() } else { format!("{:.1}%", self.device_stats.cpu_history.last().unwrap()) },
                |cp| format!("{:.1}% (usr: {:.1}%, sys: {:.1}%)", cp.cpu_percent, cp.user_cpu_percent, cp.sys_cpu_percent),
            );
            self.render_sparkline_graph(
                ui,
                "⚡ CPU Utilization",
                &cpu_val_str,
                &self.device_stats.cpu_history,
                100.0,
                egui::Color32::from_rgb(46, 204, 113),
                65.0,
            );

            ui.add_space(4.0);

            let ram_val_str = self.device_stats.chanperf.as_ref().map_or_else(
                || if self.device_stats.ram_history.is_empty() { "— MB".to_string() } else { format!("{:.1} MB", self.device_stats.ram_history.last().unwrap()) },
                |cp| format!("{:.1} MB", cp.memory_mb),
            );
            self.render_sparkline_graph(
                ui,
                "💾 RAM Memory Footprint",
                &ram_val_str,
                &self.device_stats.ram_history,
                128.0,
                self.theme.roku_purple,
                65.0,
            );
        } else {
            // DEVELOPER MODE IS DISABLED: Explain clearly why and how to enable via physical remote
            egui::Frame::none()
                .fill(self.theme.lighter_background)
                .rounding(6.0)
                .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("ℹ").size(15.0).color(self.theme.accent));
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new("Optional: Internal CPU & RAM Profiling")
                                    .strong()
                                    .size(12.0)
                                    .color(self.theme.accent),
                            );
                            ui.label(
                                egui::RichText::new("Roku OS restricts internal hardware CPU, RAM, and FPS profiling to Developer Mode. Because Roku blocks secret codes over Wi-Fi for security, it must be enabled using your physical Roku remote:")
                                    .size(11.0)
                                    .color(self.theme.foreground),
                            );
                            ui.add_space(2.0);
                            ui.label(
                                egui::RichText::new("1. On your physical Roku remote, press: Home (3x) > Up (2x) > Right > Left > Right > Left > Right\n2. The 'Developer Settings' screen will appear on your TV.\n3. Choose 'Enable installer and restart', set a password, and allow Roku to reboot.")
                                    .size(11.0)
                                    .color(self.theme.dark_foreground),
                            );
                            ui.add_space(1.0);
                            ui.label(
                                egui::RichText::new("Once enabled, live CPU % and RAM MB graphs will automatically activate here.")
                                    .size(10.5)
                                    .italics()
                                    .color(self.theme.dark_foreground),
                            );
                        });
                    });
                });
        }
    }

    pub fn render_device_stats_narrow(&mut self, ui: &mut egui::Ui, content_width: f32) {
        self.render_device_stats_content(ui, content_width);
    }

    pub fn power_on(&mut self) {
        self.tv_powered_on.store(true, Ordering::Relaxed);
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
        self.tv_powered_on.store(false, Ordering::Relaxed);
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
        let is_currently_on = if !self.is_device_reachable {
            false
        } else if self.device_details.is_tv {
            self.device_details.power_mode != "PowerOff"
                && self.device_details.power_mode != "Standby"
        } else {
            self.tv_powered_on.load(Ordering::Relaxed)
        };

        if is_currently_on {
            self.power_off();
        } else {
            self.power_on();
        }
    }

    pub fn launch_app(&self, app_id: String) {
        self.tv_powered_on.store(true, Ordering::Relaxed);
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

    pub fn refresh_all(&mut self) {
        self.is_scanning = true;
        self.is_refreshing_apps = true;
        self.status_text = "Refreshing device & scanning network...".into();
        self.start_discovery_scan();
        self.refresh_device_info();
        if !self.selected_device_ip.is_empty() {
            let ip = self.selected_device_ip.clone();
            let tx = self.tx.clone();
            let ctx = self.ctx.clone();
            thread::spawn(move || {
                refresh_apps_worker(&ip, &tx, &ctx);
            });
        }
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
            refresh_apps_worker(&ip, &tx, &ctx);
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
                    if info.state == "play" {
                        self.tv_powered_on.store(true, Ordering::Relaxed);
                    }
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
                    if details.is_tv {
                        self.tv_powered_on.store(details.power_mode != "PowerOff" && details.power_mode != "Standby", Ordering::Relaxed);
                    } else if details.power_mode == "PowerOff" || details.power_mode == "Standby" {
                        self.tv_powered_on.store(false, Ordering::Relaxed);
                    }
                    self.device_details = details;
                }
                BackgroundMessage::DeviceStatsUpdated {
                    chanperf,
                    frame_rate,
                    bitmaps,
                    sgnodes,
                    media_player,
                    device_details,
                } => {
                    self.device_stats.is_loading = false;
                    self.device_stats.last_updated = Some(Instant::now());

                    if let Some(details) = device_details {
                        self.device_details = details;
                    }

                    if let Some(mp) = media_player {
                        let mbps = if let Some(bps) = mp.bandwidth_bps {
                            (bps as f32) / 1_000_000.0
                        } else if let Some(bps) = mp.video_bitrate_bps {
                            (bps as f32) / 1_000_000.0
                        } else {
                            0.0
                        };
                        self.device_stats.bandwidth_history.push(mbps);
                        if self.device_stats.bandwidth_history.len() > 40 {
                            self.device_stats.bandwidth_history.remove(0);
                        }
                        self.media_player = mp;
                    }

                    if chanperf.status == "OK" {
                        self.device_stats.cpu_history.push(chanperf.cpu_percent);
                        if self.device_stats.cpu_history.len() > 40 {
                            self.device_stats.cpu_history.remove(0);
                        }
                        self.device_stats.ram_history.push(chanperf.memory_mb);
                        if self.device_stats.ram_history.len() > 40 {
                            self.device_stats.ram_history.remove(0);
                        }
                    }

                    if frame_rate.status == "OK" && frame_rate.fps > 0.0 {
                        self.device_stats.fps_history.push(frame_rate.fps);
                        if self.device_stats.fps_history.len() > 40 {
                            self.device_stats.fps_history.remove(0);
                        }
                    }

                    self.device_stats.chanperf = Some(chanperf);
                    self.device_stats.frame_rate = Some(frame_rate);
                    self.device_stats.bitmaps = Some(bitmaps);
                    self.device_stats.sgnodes = Some(sgnodes);
                }
                BackgroundMessage::PowerStateUpdated(reachable) => {
                    let was_reachable = self.is_device_reachable;
                    self.is_device_reachable = reachable;
                    if !reachable {
                        self.tv_powered_on.store(false, Ordering::Relaxed);
                    } else if !was_reachable {
                        self.tv_powered_on.store(true, Ordering::Relaxed);
                    }
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

    pub fn render_controls_section(&mut self, ui: &mut egui::Ui, width: f32) {
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
                    self.replay();
                }
                ui.add_space(spacing);
                if ui.add_sized(btn_nav, egui::Button::new("Options (*)")).on_hover_text("Roku Options / Asterisk menu (O)").clicked() {
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

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);

            // Clean single button to open Live Keyboard View / Modal
            let text_btn = egui::Button::new(
                egui::RichText::new("⌨ Live Keyboard (K)")
                    .size(12.5)
                    .color(self.theme.foreground),
            );
            let btn_w = (width - 24.0).clamp(160.0, 240.0);
            if ui.add_sized(egui::vec2(btn_w, 28.0), text_btn)
                .on_hover_text("Open Live Keyboard: Keystrokes are sent to TV as you type (K)")
                .clicked()
            {
                self.show_text_dialog = true;
                self.focus_text_input = true;
                self.show_setup_guide = false;
                self.show_device_info = false;
                self.show_shortcuts = false;
            }
        });
    }

    pub fn render_apps_section(&mut self, ui: &mut egui::Ui, is_wide_layout: bool) -> Option<f32> {
        let mut do_refresh_apps = false;
        let is_compact = ui.available_width() < 360.0;
        ui.horizontal(|ui| {
            let title_text = if is_compact {
                "Apps"
            } else {
                "Quick Launch Apps"
            };
            ui.label(
                egui::RichText::new(title_text)
                    .strong()
                    .size(14.0)
                    .color(self.theme.foreground),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(4.0);
                let refresh_text = if self.is_refreshing_apps {
                    "⏳"
                } else if is_compact {
                    "🔄 Refresh"
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

                if !self.apps.is_empty() && !is_compact {
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
        let buttons_top_y = ui.cursor().top();

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
            return Some(buttons_top_y);
        }

        let mut app_to_launch = None;

        let render_grid = |ui: &mut egui::Ui, app_to_launch: &mut Option<String>| {
            let avail_w = ui.available_width() - 8.0;
            let spacing = 8.0f32;
            let (btn_w, btn_h, cols) = if is_wide_layout {
                // Wide view: 138px is the largest card size desired.
                // As the view becomes narrower, gracefully accommodate by scaling down towards ~100px.
                let max_card_w = 138.0f32;
                let aspect_ratio = 0.75f32;
                let cols = ((avail_w + spacing) / (max_card_w + spacing)).ceil().max(2.0) as usize;
                let btn_w = ((avail_w - (spacing * (cols as f32 - 1.0))) / (cols as f32)).clamp(85.0, max_card_w);
                let btn_h = (btn_w * aspect_ratio).clamp(76.0, 104.0);
                (btn_w, btn_h, cols)
            } else {
                // Narrow view: keep existing sizing untouched
                let min_card_w = 96.0f32;
                let aspect_ratio = 0.92f32;
                let cols = ((avail_w / min_card_w).floor() as usize).max(2);
                let btn_w = ((avail_w - (spacing * (cols as f32 - 1.0))) / (cols as f32)).max(85.0);
                let btn_h = (btn_w * aspect_ratio).clamp(88.0, 102.0);
                (btn_w, btn_h, cols)
            };

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
                                let icon_size = if is_wide_layout { (btn_h * 0.28).clamp(22.0, 30.0) } else { 26.0 };
                                let font_size = if is_wide_layout { (btn_h * 0.11).clamp(10.0, 11.5) } else { 11.0 };
                                let top_pad = if is_wide_layout { (btn_h * 0.06).clamp(3.0, 8.0) } else { 8.0 };
                                ui.add_space(top_pad);
                                ui.label(
                                    egui::RichText::new("📺")
                                        .size(icon_size),
                                );
                                ui.add_space(4.0);
                                let label = egui::Label::new(
                                    egui::RichText::new(&app.name)
                                        .size(font_size)
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

        Some(buttons_top_y)
    }

    pub fn render_device_info_narrow(&mut self, ui: &mut egui::Ui, content_width: f32) {
        ui.horizontal(|ui| {
            ui.heading(
                egui::RichText::new("ℹ Roku Device Details")
                    .color(self.theme.foreground)
                    .size(16.0),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    self.show_device_info = false;
                }
            });
        });
        ui.separator();
        ui.add_space(4.0);

        egui::Frame::none()
            .fill(self.theme.lighter_background)
            .rounding(6.0)
            .inner_margin(egui::Margin::symmetric(10.0, 8.0))
            .show(ui, |ui| {
                let col1_w = 110.0f32;
                let col2_w = (content_width - col1_w - 32.0).max(110.0);

                egui::Grid::new("device_details_narrow_grid")
                    .spacing([8.0, 6.0])
                    .min_col_width(col1_w)
                    .max_col_width(col2_w)
                    .show(ui, |ui| {
                        let mut row = |label: &str, val: &str| {
                            ui.label(egui::RichText::new(label).strong().color(self.theme.accent).size(12.0));
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(if val.is_empty() { "—" } else { val })
                                        .color(self.theme.foreground)
                                        .size(12.0),
                                )
                                .wrap_mode(egui::TextWrapMode::Wrap),
                            );
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
                        if self.device_details.is_tv {
                            row("Power Mode:", &self.device_details.power_mode);
                        } else {
                            let dev_status = if self.is_device_reachable {
                                "Online"
                            } else {
                                "Offline"
                            };
                            row("Device Status:", dev_status);
                            let tv_status = if self.tv_powered_on.load(Ordering::Relaxed) {
                                "On"
                            } else {
                                "Off"
                            };
                            row("TV Status:", tv_status);
                        }
                        row("IP Address:", &self.selected_device_ip);

                        ui.label(egui::RichText::new("Mobile Control:").strong().color(self.theme.accent).size(12.0));
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
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(ecp_display)
                                    .color(ecp_color)
                                    .size(12.0),
                            )
                            .wrap_mode(egui::TextWrapMode::Wrap),
                        );
                        ui.end_row();

                        let is_active = self.is_active.load(Ordering::Relaxed);
                        ui.label(egui::RichText::new("App Status:").strong().color(self.theme.accent).size(12.0));
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            let (icon_rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                            let center = icon_rect.center();
                            if is_active {
                                ui.painter().circle_filled(center, 3.5, egui::Color32::from_rgb(46, 204, 113));
                                ui.painter().circle_stroke(
                                    center,
                                    5.5,
                                    egui::Stroke::new(1.0f32, egui::Color32::from_rgba_premultiplied(46, 204, 113, 100)),
                                );
                                ui.label(
                                    egui::RichText::new("Live (Active)")
                                        .color(egui::Color32::from_rgb(46, 204, 113))
                                        .strong()
                                        .size(12.0),
                                );
                            } else {
                                ui.painter().circle_stroke(
                                    center,
                                    4.0,
                                    egui::Stroke::new(1.3f32, egui::Color32::from_rgb(155, 168, 190)),
                                );
                                ui.painter().circle_filled(
                                    center,
                                    1.6,
                                    egui::Color32::from_rgb(155, 168, 190),
                                );
                                ui.label(
                                    egui::RichText::new("Sleeping (Idle)")
                                        .color(egui::Color32::from_rgb(155, 168, 190))
                                        .strong()
                                        .size(12.0),
                                );
                            }
                        });
                        ui.end_row();
                    });
            });

        let is_limited = self.device_details.ecp_setting_mode.eq_ignore_ascii_case("limited")
            || self.device_details.ecp_setting_mode.eq_ignore_ascii_case("disabled");
        let is_unreachable = !self.is_device_reachable && !self.selected_device_ip.is_empty();

        if is_limited || is_unreachable {
            ui.add_space(8.0);
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
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(desc)
                                        .size(11.0)
                                        .color(self.theme.foreground),
                                )
                                .wrap_mode(egui::TextWrapMode::Wrap),
                            );
                        });
                    });
                });
        }

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
            if is_limited || is_unreachable {
                if ui.button("⚙ Setup Guide").clicked() {
                    self.show_setup_guide = true;
                    self.show_device_info = false;
                }
            }
            if ui.button("📊 Stats").clicked() {
                self.show_device_stats = true;
                self.show_device_info = false;
                self.fetch_device_stats();
            }
            if ui.button("🔄 Refresh Info").clicked() {
                self.refresh_device_info();
            }
            if ui.button("Close").clicked() {
                self.show_device_info = false;
            }
        });
        ui.add_space(16.0);
    }

    pub fn render_setup_guide_narrow(&mut self, ui: &mut egui::Ui, content_width: f32) {
        ui.horizontal(|ui| {
            ui.heading(
                egui::RichText::new("⚙ Setup & Troubleshooting")
                    .color(self.theme.foreground)
                    .size(16.0),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    self.show_setup_guide = false;
                }
            });
        });
        ui.add(
            egui::Label::new(
                egui::RichText::new("Follow these steps if your Roku is not discovered or commands are not responding:")
                    .color(self.theme.dark_foreground)
                    .size(11.5),
            )
            .wrap_mode(egui::TextWrapMode::Wrap),
        );
        ui.separator();
        ui.add_space(6.0);

        // Section 1: Enable Mobile App Control (ECP)
        ui.label(
            egui::RichText::new("1. Enable Mobile App Control (ECP)")
                .strong()
                .size(13.0)
                .color(self.theme.accent),
        );
        ui.add(
            egui::Label::new(
                egui::RichText::new("Roku requires external control permission. In 'Limited' mode, commands like Keypress and App launch are rejected by Roku:")
                    .size(11.5)
                    .color(self.theme.foreground),
            )
            .wrap_mode(egui::TextWrapMode::Wrap),
        );
        ui.add_space(4.0);

        egui::Frame::none()
            .fill(self.theme.lighter_background)
            .rounding(6.0)
            .inner_margin(8.0)
            .show(ui, |ui| {
                let steps = [
                    ("Step 1", "Using physical Roku remote, press the Home button."),
                    ("Step 2", "Navigate to Settings > System."),
                    ("Step 3", "Select Advanced system settings."),
                    ("Step 4", "Select Control by mobile apps > Network access."),
                    ("Step 5", "Select 'Default' or 'Permissive' (do not leave on 'Limited')."),
                ];
                for (step, desc) in steps {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new(format!("{}:", step)).strong().color(self.theme.accent).size(11.5));
                        ui.label(egui::RichText::new(desc).size(11.5).color(self.theme.foreground));
                    });
                    ui.add_space(2.0);
                }
            });

        ui.add_space(8.0);

        // Section 2: Wi-Fi Network & Router
        ui.label(
            egui::RichText::new("2. Wi-Fi & Subnet Setup")
                .strong()
                .size(13.0)
                .color(self.theme.accent),
        );
        ui.add(
            egui::Label::new(
                egui::RichText::new("• Ensure computer and Roku are connected to the exact same Wi-Fi network.\n• Verify router does not have 'AP Isolation' / 'Client Isolation' enabled.")
                    .size(11.5)
                    .color(self.theme.foreground),
            )
            .wrap_mode(egui::TextWrapMode::Wrap),
        );

        ui.add_space(8.0);

        // Section 3: Manual IP Entry
        ui.label(
            egui::RichText::new("3. Find Your Roku IP Manually")
                .strong()
                .size(13.0)
                .color(self.theme.accent),
        );
        ui.add(
            egui::Label::new(
                egui::RichText::new("If your router blocks discovery broadcasts:\n• On Roku: Settings > Network > About > IP address.\n• In this app: Select 'Enter IP manually' in the device dropdown.")
                    .size(11.5)
                    .color(self.theme.foreground),
            )
            .wrap_mode(egui::TextWrapMode::Wrap),
        );

        ui.add_space(8.0);

        // Section 4: TV Power & HDMI-CEC Control
        ui.label(
            egui::RichText::new("4. TV Power via Roku (HDMI-CEC)")
                .strong()
                .size(13.0)
                .color(self.theme.accent),
        );
        ui.add(
            egui::Label::new(
                egui::RichText::new("Roku Streaming Sticks turn on the connected TV screen using HDMI-CEC:\n• On Roku: Settings > System > Control other devices (CEC) > Check '1-touch play'.\n• On your TV: Enable HDMI-CEC in your TV's settings menu (e.g. AnyNet+, Bravia Sync, SimpLink).")
                    .size(11.5)
                    .color(self.theme.foreground),
            )
            .wrap_mode(egui::TextWrapMode::Wrap),
        );

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(4.0);

        // Status Summary Card
        ui.label(
            egui::RichText::new("Current Device Status:")
                .strong()
                .color(self.theme.accent)
                .size(12.5),
        );
        egui::Frame::none()
            .fill(self.theme.lighter_background)
            .rounding(6.0)
            .inner_margin(8.0)
            .show(ui, |ui| {
                let col1_w = 110.0f32;
                let col2_w = (content_width - col1_w - 32.0).max(110.0);
                egui::Grid::new("setup_status_narrow_grid")
                    .spacing([8.0, 4.0])
                    .min_col_width(col1_w)
                    .max_col_width(col2_w)
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("Selected IP:").size(11.5));
                        ui.label(egui::RichText::new(&self.selected_device_ip).monospace().size(11.5));
                        ui.end_row();

                        ui.label(egui::RichText::new("Connection:").size(11.5));
                        let (reach_label, reach_color) = if self.is_device_reachable {
                            ("Reachable / Online", egui::Color32::from_rgb(46, 204, 113))
                        } else {
                            ("Unreachable / Offline", egui::Color32::from_rgb(220, 60, 50))
                        };
                        ui.label(egui::RichText::new(reach_label).strong().color(reach_color).size(11.5));
                        ui.end_row();

                        ui.label(egui::RichText::new("Mobile Control:").size(11.5));
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
                        ui.label(egui::RichText::new(mode_str).strong().color(mode_color).size(11.5));
                        ui.end_row();
                    });
            });

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);

        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
            if ui.button("🔍 Scan Network").clicked() {
                self.is_scanning = true;
                self.status_text = "Scanning network...".into();
                self.start_discovery_scan();
            }
            if ui.button("🔄 Re-test Connection").clicked() {
                self.refresh_device_info();
            }
            if ui.button("ℹ Device Details").clicked() {
                self.show_device_info = true;
                self.show_setup_guide = false;
            }
            if ui.button("Close").clicked() {
                self.show_setup_guide = false;
            }
        });
        ui.add_space(16.0);
    }

    pub fn render_shortcuts_narrow(&mut self, ui: &mut egui::Ui, content_width: f32) {
        ui.horizontal(|ui| {
            ui.heading(
                egui::RichText::new("⌨ Keyboard Shortcuts")
                    .color(self.theme.foreground)
                    .size(16.0),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    self.show_shortcuts = false;
                }
            });
        });
        ui.label(
            egui::RichText::new("Control Roku directly with your keyboard:")
                .size(11.5)
                .color(self.theme.foreground),
        );
        ui.separator();
        ui.add_space(4.0);

        egui::Frame::none()
            .fill(self.theme.lighter_background)
            .rounding(6.0)
            .inner_margin(egui::Margin::symmetric(8.0, 8.0))
            .show(ui, |ui| {
                let col2_w = 95.0f32;
                let col3_w = (content_width - col2_w - 55.0).max(90.0);

                egui::Grid::new("shortcuts_narrow_grid")
                    .spacing([8.0, 6.0])
                    .max_col_width(col3_w)
                    .show(ui, |ui| {
                        let shortcuts = [
                            ("🎯", "Arrow Keys", "Navigate Up / Down / Left / Right"),
                            ("🔘", "Enter / Space", "OK / Select"),
                            ("🔊", "Ctrl + Up / Down", "Volume Up / Down"),
                            ("⏩", "Ctrl + Left / Right", "Rewind (<<) / Fast Forward (>>)"),
                            ("↩", "Backspace / Esc", "Back"),
                            ("🏠", "H", "Home"),
                            ("▶⏸", "P", "Play / Pause"),
                            ("↺", "R", "Instant Replay"),
                            ("✱", "O / *", "Options (*) menu on Roku"),
                            ("🔇", "M", "Mute"),
                            ("ℹ", "I", "Toggle Device Info dialog"),
                            ("⚙", "S", "Toggle Setup & Troubleshooting Guide"),
                            ("🖥", "Ctrl + M", "Toggle window size (Min / Full Screen)"),
                            ("📌", "Ctrl + Shift + M", "Toggle Always-on-Top (compact size)"),
                            ("🔄", "Ctrl + Shift + R", "Refresh Quick Launch Apps"),
                            ("⌨", "K", "Open Live Keyboard (real-time typing, backspace, clear)"),
                            ("💡", "Ctrl + ,", "Toggle shortcuts guide"),
                        ];
                        for (icon, keys, desc) in shortcuts {
                            ui.label(icon);
                            ui.label(egui::RichText::new(keys).strong().color(self.theme.accent).size(11.5));
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(desc)
                                        .color(self.theme.foreground)
                                        .size(11.5),
                                )
                                .wrap_mode(egui::TextWrapMode::Wrap),
                            );
                            ui.end_row();
                        }
                    });
            });

        ui.add_space(12.0);
    }

    pub fn handle_keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }

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
            key_o,
            key_m,
            key_p,
            key_a,
            key_s,
            key_k,
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
                i.key_pressed(egui::Key::O) || (i.modifiers.shift && i.key_pressed(egui::Key::Num8)),
                i.key_pressed(egui::Key::M),
                i.key_pressed(egui::Key::P),
                i.key_pressed(egui::Key::A),
                i.key_pressed(egui::Key::S),
                i.key_pressed(egui::Key::K),
                i.key_pressed(egui::Key::Comma),
            )
        });

        // Ctrl + , -> Toggle keyboard shortcuts help modal
        if ctrl && key_comma {
            self.show_shortcuts = !self.show_shortcuts;
            return;
        }

        // Ctrl + Shift + M -> Set to minimum size (320x680) & toggle Always on Top on/off
        if ctrl && shift && key_m {
            if self.is_always_on_top {
                // Toggle Always on Top OFF -> Return to normal window level
                self.is_always_on_top = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(egui::WindowLevel::Normal));
            } else {
                // Toggle Always on Top ON -> Bring down to minimum size (320x680) and set Always on Top
                self.pending_restore_to_min = 3;
                self.is_always_on_top = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop));
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(320.0, 680.0)));
            }
            return;
        }

        // Ctrl + M -> Toggle window size between minimum size (320x680) and maximized full screen, disabling Always on Top
        if ctrl && !shift && key_m {
            if self.is_always_on_top {
                self.is_always_on_top = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(egui::WindowLevel::Normal));
            }

            let is_maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false))
                || ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            let current_w = ctx.screen_rect().width();

            if is_maximized || current_w >= 680.0 {
                self.pending_restore_to_min = 3;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(320.0, 680.0)));
            } else {
                self.pending_restore_to_min = 0;
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            }
            return;
        }

        // If live text dialog is open, allow Esc to close it, but block all remote control shortcuts
        if self.show_text_dialog {
            if key_escape {
                self.show_text_dialog = false;
            }
            return;
        }

        // S -> Toggle Setup & Troubleshooting Guide
        if !ctrl && key_s {
            self.show_setup_guide = !self.show_setup_guide;
            if self.show_setup_guide {
                self.show_device_info = false;
                self.show_device_stats = false;
                self.show_shortcuts = false;
                self.show_text_dialog = false;
            }
            return;
        }

        // I -> Toggle Device Info dialog box
        if !ctrl && key_i {
            self.show_device_info = !self.show_device_info;
            if self.show_device_info {
                self.show_setup_guide = false;
                self.show_device_stats = false;
                self.show_shortcuts = false;
                self.show_text_dialog = false;
            }
            return;
        }

        // K -> Toggle Keyboard / Text Entry dialog box
        if !ctrl && key_k {
            self.show_text_dialog = true;
            self.focus_text_input = true;
            self.show_setup_guide = false;
            self.show_device_info = false;
            self.show_device_stats = false;
            self.show_shortcuts = false;
            return;
        }

        // Escape closes any open modal dialog
        if key_escape && (self.show_shortcuts || self.show_device_info || self.show_device_stats || self.show_setup_guide) {
            self.show_shortcuts = false;
            self.show_device_info = false;
            self.show_device_stats = false;
            self.show_setup_guide = false;
            return;
        }

        // When a modal or in-page dialog is open, do not forward remote control keys
        if self.show_shortcuts || self.show_device_info || self.show_device_stats || self.show_setup_guide {
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
                self.replay(); // Replay Button (5x Rev + Play)
            } else if key_o {
                self.send_key("Info"); // Options / Asterisk (*) button on Roku
            } else if key_p {
                self.send_key("Play"); // Play / Pause
            } else if key_m {
                self.send_key("VolumeMute"); // Mute
            }
        }
    }
}

#[cfg(unix)]
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

#[cfg(not(unix))]
fn is_hyprland_focused() -> Option<bool> {
    None
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
        if self.pending_restore_to_min > 0 {
            self.pending_restore_to_min -= 1;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(320.0, 680.0)));
            if self.is_always_on_top {
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop));
            }
            if self.pending_restore_to_min > 0 {
                ctx.request_repaint();
            }
        }

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

        // When stats view is open, poll live telemetry every ~2 seconds
        if self.show_device_stats && !self.selected_device_ip.is_empty() {
            let should_refresh = match self.device_stats.last_updated {
                Some(last) => last.elapsed() >= Duration::from_millis(2000),
                None => true,
            };
            if should_refresh && !self.device_stats.is_loading {
                self.fetch_device_stats();
            }
            ctx.request_repaint_after(Duration::from_millis(1000));
        }

        // In wide view (>= 680px), show dialogs as centered floating modal windows.
        // In narrow view (< 680px), dialogs are rendered cleanly in-page inside CentralPanel.
        let is_wide = ctx.screen_rect().width() >= 680.0;
        // Live Roku Keyboard Modal Dialog (Wide View)
        if is_wide && self.show_text_dialog {
            let modal_w = (ctx.screen_rect().width() - 40.0).clamp(320.0, 440.0);
            let frame = egui::Frame::window(&ctx.style())
                .fill(self.theme.dark_background)
                .rounding(8.0)
                .stroke(egui::Stroke::new(1.2f32, self.theme.lighter_background))
                .inner_margin(egui::Margin::symmetric(14.0, 12.0));

            egui::Window::new("Live Roku Keyboard")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .fixed_size(egui::vec2(modal_w, 0.0))
                .frame(frame)
                .show(ctx, |ui| {
                    self.render_live_keyboard_content(ui, modal_w);
                });
        }

        let max_modal_w = (ctx.screen_rect().width() - 40.0).max(300.0);

        if is_wide {
            // Keyboard Shortcuts Modal Window
            if self.show_shortcuts {
                egui::Window::new("⌨ Keyboard Shortcuts")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .max_width(max_modal_w)
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
                                ui.label(egui::RichText::new("O / *").strong().color(self.theme.accent));
                                ui.label(egui::RichText::new("Options (*) menu on Roku").color(self.theme.foreground));
                                ui.end_row();

                                ui.label("🔇");
                                ui.label(egui::RichText::new("M").strong().color(self.theme.accent));
                                ui.label(egui::RichText::new("Mute").color(self.theme.foreground));
                                ui.end_row();

                                ui.label("ℹ");
                                ui.label(egui::RichText::new("I").strong().color(self.theme.accent));
                                ui.label(egui::RichText::new("Toggle Device Info dialog").color(self.theme.foreground));
                                ui.end_row();

                                ui.label("⚙");
                                ui.label(egui::RichText::new("S").strong().color(self.theme.accent));
                                ui.label(egui::RichText::new("Toggle Setup & Troubleshooting Guide").color(self.theme.foreground));
                                ui.end_row();

                                ui.label("🖥");
                                ui.label(egui::RichText::new("Ctrl + M").strong().color(self.theme.accent));
                                ui.label(egui::RichText::new("Toggle window size (Min / Full Screen)").color(self.theme.foreground));
                                ui.end_row();

                                ui.label("📌");
                                ui.label(egui::RichText::new("Ctrl + Shift + M").strong().color(self.theme.accent));
                                ui.label(egui::RichText::new("Toggle Always-on-Top (compact size)").color(self.theme.foreground));
                                ui.end_row();

                                ui.label("🔄");
                                ui.label(egui::RichText::new("Ctrl + Shift + R").strong().color(self.theme.accent));
                                ui.label(egui::RichText::new("Refresh Quick Launch Apps").color(self.theme.foreground));
                                ui.end_row();

                                ui.label("⌨");
                                ui.label(egui::RichText::new("K").strong().color(self.theme.accent));
                                ui.label(egui::RichText::new("Open Live Keyboard (real-time typing, backspace, clear)").color(self.theme.foreground));
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
                    .max_width(max_modal_w)
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
                                if self.device_details.is_tv {
                                    row("Power Mode:", &self.device_details.power_mode);
                                } else {
                                    let dev_status = if self.is_device_reachable {
                                        "Online"
                                    } else {
                                        "Offline"
                                    };
                                    row("Device Status:", dev_status);
                                    let tv_status = if self.tv_powered_on.load(Ordering::Relaxed) {
                                        "On"
                                    } else {
                                        "Off"
                                    };
                                    row("TV Status:", tv_status);
                                }
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
                                    self.show_device_info = false;
                                }
                            }
                            if ui.button("📊 Stats").clicked() {
                                self.show_device_stats = true;
                                self.show_device_info = false;
                                self.fetch_device_stats();
                            }

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button("Close").clicked() {
                                    self.show_device_info = false;
                                }
                            });
                        });
                    });
            }

            // Roku Stats & Performance Modal Window
            if self.show_device_stats {
                egui::Window::new("📊 Roku Stats & Performance")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .default_width(390.0)
                    .max_width(max_modal_w.max(390.0))
                    .show(ctx, |ui| {
                        self.render_device_stats_content(ui, 380.0);
                    });
            }

            // Roku Setup Guide Modal Window
            if self.show_setup_guide {
                egui::Window::new("⚙ Roku Setup & Troubleshooting Guide")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .default_width(450.0)
                    .max_width(max_modal_w)
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

                        ui.horizontal_wrapped(|ui| {
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
        }

        let panel_frame = egui::Frame::none()
            .fill(self.theme.background)
            .inner_margin(egui::Margin::symmetric(14.0, 10.0));
        egui::CentralPanel::default().frame(panel_frame).show(ctx, |ui| {
            let total_width = ui.available_width();
            let is_wide = total_width >= 680.0;

            let is_powered_on = if !self.is_device_reachable {
                false
            } else if self.device_details.is_tv {
                match self.device_details.power_mode.as_str() {
                    "PowerOn" => true,
                    "DisplayOff" | "Headless" => true,
                    "PowerOff" | "Standby" => false,
                    _ => self.is_device_reachable,
                }
            } else {
                // For streaming sticks / players: if the television is off, consider Roku as being off
                self.tv_powered_on.load(Ordering::Relaxed)
            };

            let (power_icon_color, power_status_label) = if !self.is_device_reachable {
                (egui::Color32::from_rgb(220, 60, 50), "Offline")
            } else if is_powered_on {
                (egui::Color32::from_rgb(46, 204, 113), "On")
            } else {
                (egui::Color32::from_rgb(220, 60, 50), "Off")
            };

            let state_icon = match self.media_player.state.as_str() {
                "play" => "▶ Playing",
                "pause" => "⏸ Paused",
                "buffer" => "⏳ Buffering",
                _ => "",
            };

            let mut do_toggle_power = false;
            let mut do_refresh_all = false;
            let mut do_select_ip = None;
            let mut switch_to_manual = false;

            if is_wide {
                // Global Header - Wide View
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

                    let (badge_rect, _) = ui.allocate_exact_size(egui::vec2(13.0, 14.0), egui::Sense::hover());
                    draw_power_icon(ui.painter(), badge_rect.center(), 4.5, power_icon_color, 1.6);
                    ui.label(
                        egui::RichText::new(power_status_label)
                            .size(11.5)
                            .strong()
                            .color(power_icon_color),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(4.0);

                        // Power button
                        let (power_label, power_bg, hover_bg) = if is_powered_on {
                            ("Power Off", egui::Color32::from_rgb(195, 55, 55), egui::Color32::from_rgb(220, 68, 68))
                        } else {
                            ("Power On", egui::Color32::from_rgb(38, 150, 78), egui::Color32::from_rgb(46, 172, 90))
                        };
                        let (rect, response) = ui.allocate_exact_size(egui::vec2(104.0, 26.0), egui::Sense::click());
                        let visuals = ui.style().interact(&response);
                        let bg = if response.is_pointer_button_down_on() {
                            if is_powered_on { egui::Color32::from_rgb(170, 45, 45) } else { egui::Color32::from_rgb(30, 130, 65) }
                        } else if response.hovered() { hover_bg } else { power_bg };
                        ui.painter().rect(rect, visuals.rounding, bg, visuals.bg_stroke);
                        let icon_center = egui::pos2(rect.min.x + 18.0, rect.center().y);
                        draw_power_icon(ui.painter(), icon_center, 4.8, egui::Color32::WHITE, 1.8);
                        let text_pos = egui::pos2(rect.min.x + 30.0, rect.center().y);
                        ui.painter().text(text_pos, egui::Align2::LEFT_CENTER, power_label, egui::FontId::proportional(12.0), egui::Color32::WHITE);
                        let tooltip = if self.device_details.is_tv {
                            if is_powered_on { "Turn off Roku TV" } else { "Turn on Roku TV" }
                        } else {
                            if is_powered_on { "Turn off TV (HDMI-CEC Standby)" } else { "Turn on TV (HDMI-CEC 1-Touch Play)" }
                        };
                        if response.on_hover_text(tooltip).clicked() {
                            do_toggle_power = true;
                        }

                        if ui.add(egui::Button::new("⚙ Setup").selected(self.show_setup_guide)).on_hover_text("Setup & Troubleshooting Guide (S)").clicked() {
                            self.show_setup_guide = !self.show_setup_guide;
                            if self.show_setup_guide {
                                self.show_device_info = false;
                                self.show_shortcuts = false;
                            }
                        }
                        if ui.add(egui::Button::new("ℹ Device Info").selected(self.show_device_info)).on_hover_text("Roku Device Details (I)").clicked() {
                            self.show_device_info = !self.show_device_info;
                            if self.show_device_info {
                                self.show_setup_guide = false;
                                self.show_shortcuts = false;
                            }
                        }
                    });
                });

                ui.add_space(2.0);

                // Row 2 - Wide View
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Roku:").color(self.theme.dark_foreground));

                    if self.manual_ip_mode {
                        let text_edit = ui.add(
                            egui::TextEdit::singleline(&mut self.selected_device_ip)
                                .desired_width(120.0)
                                .hint_text("192.168.x.x"),
                        );
                        if text_edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            do_select_ip = Some(self.selected_device_ip.clone());
                        }
                        if ui.button("Connect").clicked() {
                            do_select_ip = Some(self.selected_device_ip.clone());
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

                        egui::ComboBox::from_id_salt("roku_device_combo_wide")
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
                                            do_select_ip = Some(dev.ip.clone());
                                        }
                                    }
                                }
                                ui.separator();
                                if ui.selectable_label(false, "+ Enter IP manually...").clicked() {
                                    switch_to_manual = true;
                                }
                            });

                        if self.is_scanning || self.is_refreshing_apps {
                            ui.spinner();
                        } else if ui.button("🔄").on_hover_text("Refresh device info & scan network").clicked() {
                            do_refresh_all = true;
                        }
                    }

                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Current:").color(self.theme.dark_foreground));
                    ui.label(
                        egui::RichText::new(&self.active_app)
                            .strong()
                            .color(self.theme.accent),
                    );

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
                        ui.add_space(4.0);
                        if self.is_always_on_top {
                            ui.label(
                                egui::RichText::new("📌 Always on Top")
                                    .color(self.theme.accent)
                                    .size(11.0)
                                    .strong(),
                            );
                        } else {
                            ui.label(
                                egui::RichText::new(&self.status_text)
                                    .color(self.theme.dark_foreground)
                                    .size(11.0),
                            );
                        }
                    });
                });
            } else {
                // Global Header - Compact & Responsive View
                let is_very_narrow = total_width < 370.0;

                // Row 1: App Title + Power Status on Left, Clean Power Toggle Button on Right
                ui.horizontal(|ui| {
                    let title_size = if is_very_narrow { 14.5 } else { 16.0 };
                    ui.label(
                        egui::RichText::new("📺 Roku Remote")
                            .strong()
                            .size(title_size)
                            .color(self.theme.foreground),
                    );

                    let (badge_rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 12.0), egui::Sense::hover());
                    draw_power_icon(ui.painter(), badge_rect.center(), 3.8, power_icon_color, 1.4);
                    ui.label(
                        egui::RichText::new(power_status_label)
                            .size(10.5)
                            .strong()
                            .color(power_icon_color),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(4.0);
                        let (power_label, power_bg, hover_bg) = if is_powered_on {
                            ("Power Off", egui::Color32::from_rgb(195, 55, 55), egui::Color32::from_rgb(220, 68, 68))
                        } else {
                            ("Power On", egui::Color32::from_rgb(38, 150, 78), egui::Color32::from_rgb(46, 172, 90))
                        };
                        let btn_w = if is_very_narrow { 74.0 } else { 82.0 };
                        let (rect, response) = ui.allocate_exact_size(egui::vec2(btn_w, 24.0), egui::Sense::click());
                        let visuals = ui.style().interact(&response);
                        let bg = if response.is_pointer_button_down_on() {
                            if is_powered_on { egui::Color32::from_rgb(170, 45, 45) } else { egui::Color32::from_rgb(30, 130, 65) }
                        } else if response.hovered() { hover_bg } else { power_bg };
                        ui.painter().rect(rect, visuals.rounding, bg, visuals.bg_stroke);
                        let icon_center = egui::pos2(rect.min.x + 11.0, rect.center().y);
                        draw_power_icon(ui.painter(), icon_center, 3.8, egui::Color32::WHITE, 1.4);
                        let text_pos = egui::pos2(rect.min.x + 19.0, rect.center().y);
                        ui.painter().text(text_pos, egui::Align2::LEFT_CENTER, power_label, egui::FontId::proportional(10.5), egui::Color32::WHITE);
                        let tooltip = if self.device_details.is_tv {
                            if is_powered_on { "Turn off Roku TV" } else { "Turn on Roku TV" }
                        } else {
                            if is_powered_on { "Turn off TV (HDMI-CEC Standby)" } else { "Turn on TV (HDMI-CEC 1-Touch Play)" }
                        };
                        if response.on_hover_text(tooltip).clicked() {
                            do_toggle_power = true;
                        }
                    });
                });

                ui.add_space(3.0);

                // Row 2: Device Selector on Left, Action Toolbar on Right, Space in Between
                ui.horizontal(|ui| {
                    if self.manual_ip_mode {
                        let text_edit = ui.add(
                            egui::TextEdit::singleline(&mut self.selected_device_ip)
                                .desired_width(105.0)
                                .hint_text("192.168.x.x"),
                        );
                        if text_edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            do_select_ip = Some(self.selected_device_ip.clone());
                        }
                        if ui.button("Connect").clicked() {
                            do_select_ip = Some(self.selected_device_ip.clone());
                        }
                        if ui.button("List").clicked() {
                            self.manual_ip_mode = false;
                        }
                    } else {
                        let current_label = if let Some(d) = self.devices.iter().find(|d| d.ip == self.selected_device_ip) {
                            if is_very_narrow {
                                if d.name.len() > 11 {
                                    format!("📺 {}…", &d.name[..10])
                                } else {
                                    format!("📺 {}", d.name)
                                }
                            } else if total_width >= 460.0 {
                                if d.name.len() > 18 {
                                    format!("📺 {}…", &d.name[..17])
                                } else {
                                    format!("📺 {}", d.name)
                                }
                            } else if d.name.len() > 13 {
                                format!("📺 {}…", &d.name[..12])
                            } else {
                                format!("📺 {}", d.name)
                            }
                        } else if !self.selected_device_ip.is_empty() {
                            format!("📺 {}", self.selected_device_ip)
                        } else if self.is_scanning {
                            "Scanning...".to_string()
                        } else {
                            "No Roku".to_string()
                        };

                        let combo_w = if total_width >= 460.0 {
                            175.0
                        } else if is_very_narrow {
                            132.0
                        } else {
                            145.0
                        };

                        egui::ComboBox::from_id_salt("roku_device_combo_narrow")
                            .selected_text(current_label)
                            .width(combo_w)
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
                                            do_select_ip = Some(dev.ip.clone());
                                        }
                                    }
                                }
                                ui.separator();
                                if ui.selectable_label(false, "+ Enter IP manually...").clicked() {
                                    switch_to_manual = true;
                                }
                            });
                    }

                    // Right side: Action toolbar buttons anchored to right edge
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(4.0);
                        let btn_size = egui::vec2(24.0, 24.0);

                        // In right_to_left, items are placed rightmost first: [⚙], then [ℹ], then [⌨], then [🔄]
                        if total_width >= 460.0 {
                            if ui.add(egui::Button::new("⚙ Setup").selected(self.show_setup_guide)).on_hover_text("Setup & Troubleshooting Guide (S)").clicked() {
                                if self.show_setup_guide {
                                    self.show_setup_guide = false;
                                } else {
                                    self.show_setup_guide = true;
                                    self.show_device_info = false;
                                    self.show_shortcuts = false;
                                    self.show_text_dialog = false;
                                }
                            }
                            if ui.add(egui::Button::new("ℹ Device Info").selected(self.show_device_info)).on_hover_text("Roku Device Details (I)").clicked() {
                                if self.show_device_info {
                                    self.show_device_info = false;
                                } else {
                                    self.show_device_info = true;
                                    self.show_setup_guide = false;
                                    self.show_shortcuts = false;
                                    self.show_text_dialog = false;
                                }
                            }
                            if ui.add(egui::Button::new("⌨ Keyboard").selected(self.show_text_dialog)).on_hover_text("Open Live Keyboard (K)").clicked() {
                                if self.show_text_dialog {
                                    self.show_text_dialog = false;
                                } else {
                                    self.show_text_dialog = true;
                                    self.focus_text_input = true;
                                    self.show_setup_guide = false;
                                    self.show_device_info = false;
                                    self.show_shortcuts = false;
                                }
                            }
                        } else {
                            if ui.add_sized(btn_size, egui::Button::new("⚙").selected(self.show_setup_guide)).on_hover_text("Setup & Troubleshooting Guide (S)").clicked() {
                                if self.show_setup_guide {
                                    self.show_setup_guide = false;
                                } else {
                                    self.show_setup_guide = true;
                                    self.show_device_info = false;
                                    self.show_shortcuts = false;
                                    self.show_text_dialog = false;
                                }
                            }
                            if ui.add_sized(btn_size, egui::Button::new("ℹ").selected(self.show_device_info)).on_hover_text("Roku Device Details (I)").clicked() {
                                if self.show_device_info {
                                    self.show_device_info = false;
                                } else {
                                    self.show_device_info = true;
                                    self.show_setup_guide = false;
                                    self.show_shortcuts = false;
                                    self.show_text_dialog = false;
                                }
                            }
                            if ui.add_sized(btn_size, egui::Button::new("⌨").selected(self.show_text_dialog)).on_hover_text("Open Live Keyboard (K)").clicked() {
                                if self.show_text_dialog {
                                    self.show_text_dialog = false;
                                } else {
                                    self.show_text_dialog = true;
                                    self.focus_text_input = true;
                                    self.show_setup_guide = false;
                                    self.show_device_info = false;
                                    self.show_shortcuts = false;
                                }
                            }
                        }

                        if self.is_scanning || self.is_refreshing_apps {
                            ui.add_sized(btn_size, egui::Spinner::new());
                        } else if ui.add_sized(btn_size, egui::Button::new("🔄")).on_hover_text("Refresh device info & scan network").clicked() {
                            do_refresh_all = true;
                        }
                    });
                });

                ui.add_space(2.0);

                // Row 3: Active App / Now Playing & Status
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let has_active = !self.active_app.is_empty() && self.active_app != "Loading...";
                    if has_active {
                        ui.label(egui::RichText::new("Current:").size(11.0).color(self.theme.dark_foreground));
                        let max_len = if is_very_narrow { 12 } else { 18 };
                        let app_display = if self.active_app.len() > max_len {
                            format!("{}…", &self.active_app[..max_len - 1])
                        } else {
                            self.active_app.clone()
                        };
                        ui.label(
                            egui::RichText::new(app_display)
                                .strong()
                                .size(11.0)
                                .color(self.theme.accent),
                        );

                        if !state_icon.is_empty() {
                            ui.label(
                                egui::RichText::new(state_icon)
                                    .color(if self.media_player.state == "play" {
                                        egui::Color32::from_rgb(70, 190, 100)
                                    } else {
                                        egui::Color32::from_rgb(230, 170, 60)
                                    })
                                    .strong()
                                    .size(10.5),
                            );
                        }
                    }

                    if self.is_always_on_top || !has_active || total_width >= 350.0 {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(4.0);
                            if self.is_always_on_top {
                                ui.label(
                                    egui::RichText::new("📌 Always on Top")
                                        .color(self.theme.accent)
                                        .size(10.5)
                                        .strong(),
                                );
                            } else {
                                ui.label(
                                    egui::RichText::new(&self.status_text)
                                        .color(self.theme.dark_foreground)
                                        .size(10.5),
                                );
                            }
                        });
                    }
                });
            }

            if do_toggle_power {
                self.toggle_power();
            }
            if do_refresh_all {
                self.refresh_all();
            }
            if let Some(ip) = do_select_ip {
                self.select_device(&ip);
            }
            if switch_to_manual {
                self.manual_ip_mode = true;
            }

            // Banner for Limited / Unreachable Mode (only shown on home remote screen, not when a sub-view is already open)
            let is_limited = self.is_device_reachable
                && (self.device_details.ecp_setting_mode.eq_ignore_ascii_case("limited")
                    || self.device_details.ecp_setting_mode.eq_ignore_ascii_case("disabled"));
            let is_dialog_open_narrow = !is_wide && (self.show_device_info || self.show_device_stats || self.show_setup_guide || self.show_shortcuts || self.show_text_dialog);

            if !is_dialog_open_narrow {
                if is_limited {
                    ui.add_space(2.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            egui::RichText::new("\u{26A0} Roku Mobile App Control is 'Limited'.")
                                .color(egui::Color32::from_rgb(240, 160, 40))
                                .size(11.5)
                                .strong(),
                        );
                        if ui.button(egui::RichText::new("⚙ Setup").color(egui::Color32::from_rgb(240, 160, 40))).clicked() {
                            self.show_setup_guide = true;
                            self.show_device_info = false;
                            self.show_device_stats = false;
                            self.show_shortcuts = false;
                            self.show_text_dialog = false;
                        }
                    });
                } else if !self.is_device_reachable && !self.is_scanning && !self.selected_device_ip.is_empty() {
                    ui.add_space(2.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            egui::RichText::new(format!("\u{26A0} Roku unreachable at {}.", self.selected_device_ip))
                                .color(egui::Color32::from_rgb(220, 80, 70))
                                .size(11.5),
                        );
                        if ui.button("⚙ Setup Guide").clicked() {
                            self.show_setup_guide = true;
                            self.show_device_info = false;
                            self.show_device_stats = false;
                            self.show_shortcuts = false;
                            self.show_text_dialog = false;
                        }
                    });
                }
            }

            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);

            if is_wide {
                // WIDE SCREEN: Controls Left (300px), Apps Grid Right
                let controls_width = 300.0f32;
                ui.horizontal_top(|ui| {
                    let section_top_y = ui.cursor().top();
                    let offset_id = egui::Id::new("roku_app_buttons_top_offset");

                    ui.vertical(|ui| {
                        ui.set_width(controls_width);

                        // Align remote controls with where the app buttons begin on the right
                        let top_padding: f32 = ui.ctx().data_mut(|d| d.get_temp(offset_id)).unwrap_or(34.0);
                        ui.add_space(top_padding);

                        self.render_controls_section(ui, controls_width);
                    });

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    ui.vertical(|ui| {
                        let apps_buttons_top_y = self.render_apps_section(ui, true);
                        if let Some(buttons_y) = apps_buttons_top_y {
                            let measured_offset = (buttons_y - section_top_y).max(0.0);
                            ui.ctx().data_mut(|d| d.insert_temp(offset_id, measured_offset));
                        }
                    });
                });
            } else {
                // NARROW SCREEN: Top-level ScrollArea
                let content_width = (total_width - 12.0).clamp(260.0, 380.0);
                let horizontal_margin = ((total_width - content_width) / 2.0).max(0.0);

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add_space(horizontal_margin);
                            ui.vertical(|ui| {
                                ui.set_width(content_width);

                                if self.show_device_info {
                                    self.render_device_info_narrow(ui, content_width);
                                } else if self.show_device_stats {
                                    self.render_device_stats_narrow(ui, content_width);
                                } else if self.show_setup_guide {
                                    self.render_setup_guide_narrow(ui, content_width);
                                } else if self.show_shortcuts {
                                    self.render_shortcuts_narrow(ui, content_width);
                                } else if self.show_text_dialog {
                                    self.render_live_keyboard_content(ui, content_width);
                                } else {
                                    // Controls section
                                    self.render_controls_section(ui, content_width);

                                    ui.add_space(10.0);
                                    ui.separator();
                                    ui.add_space(8.0);

                                    // Applications Section
                                    self.render_apps_section(ui, false);
                                    ui.add_space(20.0);
                                }
                            });
                        });
                    });
            }
        });
    }
}
