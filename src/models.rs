use eframe::egui;

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
    pub duration_ms: Option<u64>,
    pub bandwidth_bps: Option<u64>,
    pub video_bitrate_bps: Option<u64>,
    pub video_res: String,
    pub video_codec: String,
    pub audio_codec: String,
    pub container: String,
    pub captions: String,
    pub drm: String,
    pub is_live: bool,
    pub buffer_current: Option<u32>,
    pub buffer_max: Option<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct DeviceDetails {
    pub model_name: String,
    pub model_number: String,
    pub software_version: String,
    pub build_number: String,
    pub network_name: String,
    pub power_mode: String,
    pub ui_resolution: String,
    pub user_location: String,
    pub ecp_setting_mode: String,
    pub is_tv: bool,
    pub is_powered_by_tv: bool,
    pub uptime_seconds: u64,
    pub wifi_driver: String,
    pub has_wifi_5g: bool,
    pub wifi_mac: String,
    pub bluetooth_mac: String,
    pub time_zone: String,
    pub supports_private_listening: bool,
    pub supports_airplay: bool,
    pub supports_ethernet: bool,
}

#[derive(Clone, Debug, Default)]
pub struct DeviceStats {
    pub last_updated: Option<std::time::Instant>,
    pub is_loading: bool,
}

pub enum BackgroundMessage {
    DeviceDiscovered(RokuDevice),
    DeviceNameUpdated(String),
    ActiveAppUpdated(String),
    AppsListUpdated(Vec<AppItem>),
    AppsListRefreshed(Vec<AppItem>),
    AppsRefreshFailed,
    ScanFinished,
    ThemeUpdated(crate::theme::ThemeColors),
    MediaPlayerUpdated(MediaPlayerInfo),
    DeviceDetailsUpdated(DeviceDetails),
    DeviceStatsUpdated {
        media_player: Option<MediaPlayerInfo>,
        device_details: Option<DeviceDetails>,
    },
    PowerStateUpdated(bool),
    AppIconLoaded { id: String, image: egui::ColorImage },
}

#[derive(Clone, Debug)]
pub enum LiveKeyCommand {
    Char(char),
    Backspace,
    Clear(usize),
    Key(&'static str),
}

pub fn default_popular_apps() -> Vec<AppItem> {
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
