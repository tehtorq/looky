use std::net::IpAddr;
use std::time::Duration;

use rust_cast::channels::media::{Media, StreamType};
use rust_cast::channels::receiver::CastDeviceApp;
use rust_cast::CastDevice;

const CAST_SERVICE: &str = "_googlecast._tcp.local.";
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone)]
pub struct CastTarget {
    pub name: String,
    pub host: IpAddr,
    pub port: u16,
}

/// Discover Chromecast devices on the LAN (blocking, ~3 seconds).
pub fn discover_devices() -> Vec<CastTarget> {
    let mdns = match mdns_sd::ServiceDaemon::new() {
        Ok(d) => d,
        Err(e) => {
            log::warn!("mDNS daemon failed to start: {e}");
            return Vec::new();
        }
    };

    let receiver = match mdns.browse(CAST_SERVICE) {
        Ok(r) => r,
        Err(e) => {
            log::warn!("mDNS browse failed: {e}");
            let _ = mdns.shutdown();
            return Vec::new();
        }
    };

    let mut devices = Vec::new();
    let deadline = std::time::Instant::now() + DISCOVERY_TIMEOUT;

    loop {
        let remaining = match deadline.checked_duration_since(std::time::Instant::now()) {
            Some(d) => d,
            None => break,
        };
        match receiver.recv_timeout(remaining) {
            Ok(mdns_sd::ServiceEvent::ServiceResolved(info)) => {
                let name = info
                    .get_property_val_str("fn")
                    .unwrap_or_else(|| info.get_fullname())
                    .to_string();

                if let Some(ip) = info.get_addresses_v4().into_iter().next() {
                    devices.push(CastTarget {
                        name,
                        host: IpAddr::V4(ip),
                        port: info.get_port(),
                    });
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }

    let _ = mdns.shutdown();
    devices
}

/// Cast a single image to a Chromecast device (blocking, ~2-4 seconds).
///
/// Opens a fresh connection, launches the Default Media Receiver, loads the
/// image URL, then disconnects. The image stays on screen after disconnect.
pub fn cast_image(device: &CastTarget, url: &str) -> Result<(), String> {
    let host = device.host.to_string();
    let cast = CastDevice::connect_without_host_verification(&host, device.port)
        .map_err(|e| format!("TLS connect: {e}"))?;

    cast.connection
        .connect("receiver-0")
        .map_err(|e| format!("Receiver connect: {e}"))?;

    cast.heartbeat
        .ping()
        .map_err(|e| format!("Ping: {e}"))?;

    // launch_app returns the existing app if already running
    let app = cast
        .receiver
        .launch_app(&CastDeviceApp::DefaultMediaReceiver)
        .map_err(|e| format!("Launch app: {e}"))?;

    cast.connection
        .connect(&app.transport_id)
        .map_err(|e| format!("App connect: {e}"))?;

    let content_type = guess_content_type(url);
    let media = Media {
        content_id: url.to_string(),
        content_type: content_type.to_string(),
        stream_type: StreamType::Buffered,
        duration: None,
        metadata: None,
    };

    cast.media
        .load(&app.transport_id, &app.session_id, &media)
        .map_err(|e| format!("Load: {e}"))?;

    log::info!("Cast to '{}': {url}", device.name);
    Ok(())
}

/// Stop the Default Media Receiver on a Chromecast device.
pub fn stop_casting(device: &CastTarget) {
    let host = device.host.to_string();
    let Ok(cast) = CastDevice::connect_without_host_verification(&host, device.port) else {
        return;
    };
    let _ = cast.connection.connect("receiver-0");
    let _ = cast.heartbeat.ping();
    if let Ok(status) = cast.receiver.get_status() {
        for app in &status.applications {
            let _ = cast.receiver.stop_app(&app.session_id);
        }
    }
}

fn guess_content_type(url: &str) -> &'static str {
    let lower = url.to_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else {
        "image/jpeg"
    }
}
