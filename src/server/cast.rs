use std::net::IpAddr;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use rust_cast::channels::media::{Media, StreamType};
use rust_cast::channels::receiver::CastDeviceApp;
use rust_cast::CastDevice;

const CAST_SERVICE: &str = "_googlecast._tcp.local.";
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(3);
const WORKER_POLL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub struct CastTarget {
    pub name: String,
    pub host: IpAddr,
    pub port: u16,
}

/// Handle to a Chromecast session backed by a dedicated worker thread.
///
/// The worker thread owns the TLS connection and auto-reconnects when it
/// drops. `load_image()` sends a URL to the worker via a channel and returns
/// instantly.
#[derive(Clone)]
pub struct CastSession {
    tx: mpsc::Sender<CastCommand>,
    pub target: CastTarget,
}

impl std::fmt::Debug for CastSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CastSession")
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

enum CastCommand {
    Load(String),
    Stop,
}

impl CastSession {
    /// Connect to a Chromecast and spawn a worker thread. Blocking (~2-4s).
    pub fn connect(target: CastTarget) -> Result<Self, String> {
        // Do the initial connection on the caller's thread so errors propagate.
        let (device, transport_id, session_id) = connect_device(&target)?;

        let (tx, rx) = mpsc::channel();
        let worker_target = target.clone();
        std::thread::Builder::new()
            .name("cast-worker".into())
            .spawn(move || {
                cast_worker(device, transport_id, session_id, worker_target, rx);
            })
            .map_err(|e| format!("Spawn cast worker: {e}"))?;

        Ok(Self { tx, target })
    }

    /// Queue an image load on the Chromecast. Returns immediately.
    pub fn load_image(&self, url: &str) -> Result<(), String> {
        self.tx
            .send(CastCommand::Load(url.to_string()))
            .map_err(|_| "Cast session closed".to_string())
    }

    /// Stop all apps on the receiver and shut down the worker thread.
    pub fn stop(&self) {
        let _ = self.tx.send(CastCommand::Stop);
    }
}

// ---------------------------------------------------------------------------
// Worker thread
// ---------------------------------------------------------------------------

/// Open TLS, connect receiver, launch Default Media Receiver, connect to app.
fn connect_device(target: &CastTarget) -> Result<(CastDevice<'static>, String, String), String> {
    let device: CastDevice<'static> =
        CastDevice::connect_without_host_verification(target.host.to_string(), target.port)
            .map_err(|e| format!("TLS connect: {e}"))?;

    device
        .connection
        .connect("receiver-0")
        .map_err(|e| format!("Receiver connect: {e}"))?;

    device
        .heartbeat
        .ping()
        .map_err(|e| format!("Ping: {e}"))?;

    let app = device
        .receiver
        .launch_app(&CastDeviceApp::DefaultMediaReceiver)
        .map_err(|e| format!("Launch app: {e}"))?;

    device
        .connection
        .connect(&app.transport_id)
        .map_err(|e| format!("App connect: {e}"))?;

    log::info!("Cast connected to '{}'", target.name);
    Ok((device, app.transport_id, app.session_id))
}

fn load_media(
    device: &CastDevice<'static>,
    transport_id: &str,
    session_id: &str,
    url: &str,
) -> Result<(), String> {
    let content_type = guess_content_type(url);
    let media = Media {
        content_id: url.to_string(),
        content_type: content_type.to_string(),
        stream_type: StreamType::Buffered,
        duration: None,
        metadata: None,
    };
    device
        .media
        .load(transport_id, session_id, &media)
        .map_err(|e| format!("Load: {e}"))?;
    Ok(())
}

fn stop_apps(device: &CastDevice<'static>) {
    if let Ok(status) = device.receiver.get_status() {
        for app in &status.applications {
            let _ = device.receiver.stop_app(&app.session_id);
        }
    }
}

/// Try to load media. If it fails, reconnect and retry once.
fn load_or_reconnect(
    device: &mut CastDevice<'static>,
    transport_id: &mut String,
    session_id: &mut String,
    target: &CastTarget,
    url: &str,
) -> bool {
    if load_media(device, transport_id, session_id, url).is_ok() {
        return true;
    }

    log::info!("Cast connection lost, reconnecting to '{}'...", target.name);
    match connect_device(target) {
        Ok((d, tid, sid)) => {
            *device = d;
            *transport_id = tid;
            *session_id = sid;
            if let Err(e) = load_media(device, transport_id, session_id, url) {
                log::warn!("Cast retry failed: {e}");
                false
            } else {
                true
            }
        }
        Err(e) => {
            log::warn!("Cast reconnect failed: {e}");
            false
        }
    }
}

fn cast_worker(
    mut device: CastDevice<'static>,
    mut transport_id: String,
    mut session_id: String,
    target: CastTarget,
    rx: mpsc::Receiver<CastCommand>,
) {
    let mut last_ping = Instant::now();

    loop {
        match rx.recv_timeout(WORKER_POLL) {
            Ok(CastCommand::Load(url)) => {
                if load_or_reconnect(
                    &mut device,
                    &mut transport_id,
                    &mut session_id,
                    &target,
                    &url,
                ) {
                    log::info!("Cast to '{}': {url}", target.name);
                }
                last_ping = Instant::now();
            }
            Ok(CastCommand::Stop) => {
                stop_apps(&device);
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Periodic ping — keeps our side of the TCP connection alive
                // and may detect a dead connection early.
                if last_ping.elapsed() >= HEARTBEAT_INTERVAL {
                    if device.heartbeat.ping().is_err() {
                        log::debug!("Cast heartbeat ping failed, proactive reconnect");
                        match connect_device(&target) {
                            Ok((d, tid, sid)) => {
                                device = d;
                                transport_id = tid;
                                session_id = sid;
                            }
                            Err(e) => log::debug!("Proactive reconnect failed: {e}"),
                        }
                    }
                    last_ping = Instant::now();
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // Session handle dropped — clean up
                stop_apps(&device);
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

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
                let friendly = info
                    .get_property_val_str("fn")
                    .unwrap_or_else(|| info.get_fullname());
                let name = match info.get_property_val_str("md") {
                    Some(model) => format!("{friendly} ({model})"),
                    None => friendly.to_string(),
                };

                if let Some(ip) = info.get_addresses_v4().into_iter().next() {
                    let addr = IpAddr::V4(ip);
                    if !devices.iter().any(|d: &CastTarget| d.host == addr) {
                        devices.push(CastTarget {
                            name,
                            host: addr,
                            port: info.get_port(),
                        });
                    }
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }

    let _ = mdns.shutdown();
    devices
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
