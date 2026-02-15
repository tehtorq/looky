use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use rust_cast::channels::heartbeat::HeartbeatResponse;
use rust_cast::channels::media::{Media, StreamType};
use rust_cast::channels::receiver::CastDeviceApp;
use rust_cast::{CastDevice, ChannelMessage};

const CAST_SERVICE: &str = "_googlecast._tcp.local.";
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone)]
pub struct CastTarget {
    pub name: String,
    pub host: IpAddr,
    pub port: u16,
}

pub enum CastCommand {
    LoadImage(String),
    Stop,
}

pub struct CastHandle {
    pub device_name: String,
    command_tx: mpsc::Sender<CastCommand>,
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
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

impl CastHandle {
    /// Connect to a Cast device and load the initial image (blocking).
    pub fn connect(device: CastTarget, initial_url: String) -> Result<CastHandle, String> {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown2 = Arc::clone(&shutdown);
        let device_name = device.name.clone();

        let thread = std::thread::Builder::new()
            .name("looky-cast".into())
            .spawn(move || cast_worker(device, initial_url, cmd_rx, shutdown2))
            .map_err(|e| format!("Failed to spawn cast thread: {e}"))?;

        Ok(CastHandle {
            device_name,
            command_tx: cmd_tx,
            shutdown,
            thread: Some(thread),
        })
    }

    pub fn load_image(&self, url: String) {
        let _ = self.command_tx.send(CastCommand::LoadImage(url));
    }

    pub fn stop(mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        let _ = self.command_tx.send(CastCommand::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for CastHandle {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        let _ = self.command_tx.send(CastCommand::Stop);
    }
}

fn cast_worker(
    device: CastTarget,
    initial_url: String,
    cmd_rx: mpsc::Receiver<CastCommand>,
    shutdown: Arc<AtomicBool>,
) {
    if let Err(e) = cast_worker_inner(&device, &initial_url, &cmd_rx, &shutdown) {
        log::warn!("Cast session to '{}' ended: {e}", device.name);
    }
}

fn cast_worker_inner(
    device: &CastTarget,
    initial_url: &str,
    cmd_rx: &mpsc::Receiver<CastCommand>,
    shutdown: &AtomicBool,
) -> Result<(), String> {
    let host = device.host.to_string();
    let cast = CastDevice::connect_without_host_verification(&host, device.port)
        .map_err(|e| format!("TLS connect failed: {e}"))?;

    cast.connection
        .connect("receiver-0")
        .map_err(|e| format!("Receiver connect failed: {e}"))?;

    cast.heartbeat
        .ping()
        .map_err(|e| format!("Initial ping failed: {e}"))?;

    let app = cast
        .receiver
        .launch_app(&CastDeviceApp::DefaultMediaReceiver)
        .map_err(|e| format!("Launch app failed: {e}"))?;

    cast.connection
        .connect(&app.transport_id)
        .map_err(|e| format!("App connect failed: {e}"))?;

    load_url(&cast, &app.transport_id, &app.session_id, initial_url)?;

    log::info!("Casting to '{}' — initial image loaded", device.name);

    // Main loop: handle heartbeat + commands
    // receive() has no timeout, so we ping every ~1s and rely on receiving pongs
    let mut last_ping = std::time::Instant::now();

    loop {
        if shutdown.load(Ordering::Relaxed) {
            let _ = cast.receiver.stop_app(&app.session_id);
            return Ok(());
        }

        // Send ping if needed
        if last_ping.elapsed() > Duration::from_secs(5) {
            if let Err(e) = cast.heartbeat.ping() {
                return Err(format!("Ping failed: {e}"));
            }
            last_ping = std::time::Instant::now();
        }

        // Check for commands (non-blocking)
        match cmd_rx.try_recv() {
            Ok(CastCommand::LoadImage(url)) => {
                if let Err(e) = load_url(&cast, &app.transport_id, &app.session_id, &url) {
                    log::warn!("Cast load failed: {e}");
                }
            }
            Ok(CastCommand::Stop) => {
                let _ = cast.receiver.stop_app(&app.session_id);
                return Ok(());
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                let _ = cast.receiver.stop_app(&app.session_id);
                return Ok(());
            }
        }

        // Receive cast messages (non-blocking via short timeout)
        match cast.receive() {
            Ok(ChannelMessage::Heartbeat(HeartbeatResponse::Ping)) => {
                let _ = cast.heartbeat.pong();
            }
            Ok(_) => {}
            Err(e) => {
                let err = format!("{e}");
                if err.contains("timed out") || err.contains("WouldBlock") {
                    // Normal — no message ready
                } else {
                    return Err(format!("Receive error: {e}"));
                }
            }
        }

        // Small sleep to avoid busy-waiting
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn load_url(
    cast: &CastDevice<'_>,
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

    cast.media
        .load(transport_id, session_id, &media)
        .map_err(|e| format!("Media load failed: {e}"))?;

    Ok(())
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
