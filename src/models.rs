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
    PowerStateUpdated(bool),
    AppIconLoaded { id: String, image: egui::ColorImage },
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
