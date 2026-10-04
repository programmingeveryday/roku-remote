use crate::models::{AppItem, DeviceDetails, MediaPlayerInfo};

pub fn clean_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&apos;", "'")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

pub fn parse_active_app_xml(xml: &str) -> Option<String> {
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

pub fn parse_device_name_xml(xml: &str) -> Option<String> {
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

pub fn parse_device_details_xml(xml: &str) -> DeviceDetails {
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
        user_location: extract("user-device-location"),
        ecp_setting_mode: extract("ecp-setting-mode"),
    }
}

pub fn parse_media_player_xml(xml: &str) -> MediaPlayerInfo {
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

pub fn parse_apps_xml(xml: &str) -> Vec<AppItem> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let sample = "<device-info><model-name>Roku Stick</model-name><model-number>3830R</model-number><software-version>15.3.4</software-version><network-name>HomeWi-Fi</network-name><power-mode>PowerOn</power-mode><ui-resolution>1080p</ui-resolution><user-device-location>Living room</user-device-location><ecp-setting-mode>limited</ecp-setting-mode></device-info>";
        let details = parse_device_details_xml(sample);
        assert_eq!(details.model_name, "Roku Stick");
        assert_eq!(details.model_number, "3830R");
        assert_eq!(details.software_version, "15.3.4");
        assert_eq!(details.network_name, "HomeWi-Fi");
        assert_eq!(details.power_mode, "PowerOn");
        assert_eq!(details.ui_resolution, "1080p");
        assert_eq!(details.user_location, "Living room");
        assert_eq!(details.ecp_setting_mode, "limited");
    }
}
