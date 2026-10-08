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
        build_number: extract("build-number"),
        network_name: extract("network-name"),
        power_mode: extract("power-mode"),
        ui_resolution: extract("ui-resolution"),
        user_location: extract("user-device-location"),
        ecp_setting_mode: extract("ecp-setting-mode"),
        is_tv: extract("is-tv").eq_ignore_ascii_case("true"),
        is_powered_by_tv: extract("is-powered-by-tv").eq_ignore_ascii_case("true"),
        uptime_seconds: extract("uptime").parse::<u64>().unwrap_or(0),
        wifi_driver: extract("wifi-driver"),
        has_wifi_5g: extract("has-wifi-5G-support").eq_ignore_ascii_case("true"),
        wifi_mac: extract("wifi-mac"),
        bluetooth_mac: extract("bluetooth-mac"),
        time_zone: extract("time-zone-name"),
        supports_private_listening: extract("supports-private-listening").eq_ignore_ascii_case("true"),
        supports_airplay: extract("supports-airplay").eq_ignore_ascii_case("true"),
        supports_ethernet: extract("supports-ethernet").eq_ignore_ascii_case("true"),
    }
}

pub fn parse_media_player_xml(xml: &str) -> MediaPlayerInfo {
    let mut info = MediaPlayerInfo::default();
    
    let extract_attr = |tag_name: &str, attr_name: &str| -> Option<String> {
        let tag_open = format!("<{}", tag_name);
        if let Some(tag_start) = xml.find(&tag_open) {
            let after_tag = &xml[tag_start..];
            if let Some(tag_end) = after_tag.find('>') {
                let tag_slice = &after_tag[..tag_end];
                let attr_pattern = format!(" {}=\"", attr_name);
                if let Some(attr_idx) = tag_slice.find(&attr_pattern) {
                    let val_start = attr_idx + attr_pattern.len();
                    if let Some(quote_end) = tag_slice[val_start..].find('"') {
                        return Some(clean_html_entities(&tag_slice[val_start..val_start + quote_end]));
                    }
                }
            }
        }
        None
    };

    let extract_tag = |tag: &str| -> Option<String> {
        let open = format!("<{}>", tag);
        let close = format!("</{}>", tag);
        if let Some(start) = xml.find(&open) {
            let s = start + open.len();
            if let Some(end) = xml[s..].find(&close) {
                return Some(clean_html_entities(xml[s..s + end].trim()));
            }
        }
        None
    };

    // State: <player state="play" ...>
    if let Some(state) = extract_attr("player", "state") {
        info.state = state;
    }

    // App name: <plugin id="..." name="YouTube" />
    if let Some(name) = extract_attr("plugin", "name") {
        info.app_name = name;
    }

    // Bandwidth: <plugin ... bandwidth="97372298 bps" /> or bitrate in stream_segment
    if let Some(bw_str) = extract_attr("plugin", "bandwidth") {
        let cleaned = bw_str.trim().trim_end_matches(" bps").trim();
        if let Ok(bps) = cleaned.parse::<u64>() {
            info.bandwidth_bps = Some(bps);
        }
    }

    // Video bitrate: <stream_segment ... bitrate="15136323" />
    if let Some(br_str) = extract_attr("stream_segment", "bitrate") {
        let cleaned = br_str.trim().trim_end_matches(" bps").trim();
        if let Ok(bps) = cleaned.parse::<u64>() {
            info.video_bitrate_bps = Some(bps);
        }
    }

    // Resolution: from stream_segment width & height, or format video_res
    let width = extract_attr("stream_segment", "width");
    let height = extract_attr("stream_segment", "height");
    if let (Some(w), Some(h)) = (width, height) {
        let label = match (w.as_str(), h.as_str()) {
            ("3840", "2160") => " (4K UHD)",
            ("1920", "1080") => " (1080p FHD)",
            ("1280", "720") => " (720p HD)",
            _ => "",
        };
        info.video_res = format!("{}x{}{}", w, h, label);
    } else if let Some(vres) = extract_attr("format", "video_res") {
        info.video_res = vres;
    }

    // Video Codec: <format video="hevc_b" ...>
    if let Some(vc) = extract_attr("format", "video") {
        let friendly = match vc.to_lowercase().as_str() {
            s if s.starts_with("hevc") => "HEVC (H.265 4K HDR)",
            s if s.starts_with("av1") => "AV1 Next-Gen",
            s if s.starts_with("vp9") => "VP9",
            s if s.starts_with("mpeg4") || s.starts_with("h264") || s.starts_with("avc") => "H.264 / AVC",
            _ => &vc,
        };
        info.video_codec = friendly.to_string();
    }

    // Audio Codec: <format audio="eac3" ...>
    if let Some(ac) = extract_attr("format", "audio") {
        let friendly = match ac.to_lowercase().as_str() {
            "eac3" => "Dolby Digital Plus (E-AC-3)",
            "ac3" => "Dolby Digital (AC-3)",
            "aac" => "AAC Stereo",
            "atmos" => "Dolby Atmos",
            "dts" => "DTS Surround",
            _ => &ac,
        };
        info.audio_codec = friendly.to_string();
    }

    // Container: <format container="dash" ...>
    if let Some(cont) = extract_attr("format", "container") {
        let friendly = match cont.to_lowercase().as_str() {
            "dash" => "MPEG-DASH",
            "hls" => "Apple HLS",
            "mp4" => "MP4",
            _ => &cont,
        };
        info.container = friendly.to_string();
    }

    // Buffering: <buffering target="0" max="1000" current="1000" />
    if let Some(cur) = extract_attr("buffering", "current").and_then(|s| s.parse::<u32>().ok()) {
        info.buffer_current = Some(cur);
    }
    if let Some(max) = extract_attr("buffering", "max").and_then(|s| s.parse::<u32>().ok()) {
        info.buffer_max = Some(max);
    }

    // Position & Duration: <position>...</position> and <duration>...</duration>
    if let Some(pos_str) = extract_tag("position") {
        let cleaned = pos_str.trim().trim_end_matches(" ms").trim();
        if let Ok(ms) = cleaned.parse::<u64>() {
            info.position_ms = Some(ms);
        }
    }
    if let Some(dur_str) = extract_tag("duration") {
        let cleaned = dur_str.trim().trim_end_matches(" ms").trim();
        if let Ok(ms) = cleaned.parse::<u64>() {
            info.duration_ms = Some(ms);
        }
    }

    // Live stream
    if let Some(live) = extract_attr("format", "live").or_else(|| extract_tag("is_live")).or_else(|| extract_tag("live")) {
        info.is_live = live.eq_ignore_ascii_case("true");
    }

    // Captions
    if let Some(caps) = extract_attr("format", "captions").or_else(|| extract_tag("captions")).or_else(|| extract_tag("closed-captions")) {
        info.captions = caps;
    }

    // DRM
    if let Some(drm) = extract_attr("format", "drm").or_else(|| extract_tag("drm")) {
        info.drm = drm;
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
        let sample = "<?xml version=\"1.0\" encoding=\"UTF-8\" ?><player state=\"play\" error=\"false\"><plugin id=\"837\" name=\"YouTube\" bandwidth=\"97372298 bps\" /><format audio=\"eac3\" video=\"hevc_b\" container=\"dash\" captions=\"English [CC]\" drm=\"Widevine\" live=\"true\" /><buffering current=\"1000\" max=\"1000\" /><position>22961735 ms</position><duration>45000000 ms</duration><stream_segment bitrate=\"15136323\" width=\"3840\" height=\"2160\" /></player>";
        let info = parse_media_player_xml(sample);
        assert_eq!(info.state, "play");
        assert_eq!(info.app_name, "YouTube");
        assert_eq!(info.bandwidth_bps, Some(97372298));
        assert_eq!(info.video_bitrate_bps, Some(15136323));
        assert_eq!(info.video_res, "3840x2160 (4K UHD)");
        assert_eq!(info.video_codec, "HEVC (H.265 4K HDR)");
        assert_eq!(info.audio_codec, "Dolby Digital Plus (E-AC-3)");
        assert_eq!(info.container, "MPEG-DASH");
        assert_eq!(info.buffer_current, Some(1000));
        assert_eq!(info.buffer_max, Some(1000));
        assert_eq!(info.position_ms, Some(22961735));
        assert_eq!(info.duration_ms, Some(45000000));
        assert_eq!(info.is_live, true);
        assert_eq!(info.captions, "English [CC]");
        assert_eq!(info.drm, "Widevine");

        let paused = "<player state=\"pause\"><plugin name=\"Netflix\" /><position>5000 ms</position></player>";
        let p_info = parse_media_player_xml(paused);
        assert_eq!(p_info.state, "pause");
        assert_eq!(p_info.app_name, "Netflix");
        assert_eq!(p_info.position_ms, Some(5000));
        assert_eq!(p_info.is_live, false);
    }

    #[test]
    fn test_parse_device_details_xml() {
        let sample = "<device-info><model-name>Roku Stick</model-name><model-number>3830R</model-number><software-version>15.3.4</software-version><network-name>HomeWi-Fi</network-name><power-mode>PowerOn</power-mode><ui-resolution>1080p</ui-resolution><user-device-location>Living room</user-device-location><ecp-setting-mode>limited</ecp-setting-mode><time-zone-name>US/Pacific</time-zone-name><supports-private-listening>true</supports-private-listening><supports-airplay>true</supports-airplay><supports-ethernet>false</supports-ethernet></device-info>";
        let details = parse_device_details_xml(sample);
        assert_eq!(details.model_name, "Roku Stick");
        assert_eq!(details.model_number, "3830R");
        assert_eq!(details.software_version, "15.3.4");
        assert_eq!(details.network_name, "HomeWi-Fi");
        assert_eq!(details.power_mode, "PowerOn");
        assert_eq!(details.ui_resolution, "1080p");
        assert_eq!(details.user_location, "Living room");
        assert_eq!(details.ecp_setting_mode, "limited");
        assert_eq!(details.is_tv, false);
        assert_eq!(details.time_zone, "US/Pacific");
        assert_eq!(details.supports_private_listening, true);
        assert_eq!(details.supports_airplay, true);
        assert_eq!(details.supports_ethernet, false);
    }
}
