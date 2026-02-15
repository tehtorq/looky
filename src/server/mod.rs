pub mod cast;
pub mod dlna;
pub mod http;
pub mod ssdp;

use std::net::{SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

pub struct ServerState {
    pub image_paths: Vec<PathBuf>,
    pub server_addr: SocketAddr,
    pub device_uuid: String,
    pub folder_name: String,
    pub shutdown: AtomicBool,
}

pub struct ServerHandle {
    state: Arc<ServerState>,
    http_thread: Option<JoinHandle<()>>,
    ssdp_thread: Option<JoinHandle<()>>,
}

impl ServerHandle {
    pub fn stop(mut self) {
        self.state.shutdown.store(true, Ordering::Relaxed);
        if let Some(t) = self.http_thread.take() {
            let _ = t.join();
        }
        if let Some(t) = self.ssdp_thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.state.shutdown.store(true, Ordering::Relaxed);
    }
}

/// Detect the local LAN IP by connecting a UDP socket to an external address.
fn local_ip() -> Option<std::net::IpAddr> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    Some(sock.local_addr().ok()?.ip())
}

/// Start the HTTP + DLNA server. Returns the handle and the gallery URL.
pub fn start_server(
    image_paths: Vec<PathBuf>,
    folder_name: String,
) -> Option<(ServerHandle, String)> {
    let ip = local_ip()?;
    let bind_addr: SocketAddr = format!("{ip}:0").parse().ok()?;
    let server = tiny_http::Server::http(bind_addr).ok()?;
    let server_addr = server.server_addr().to_ip().unwrap();
    let url = format!("http://{server_addr}");

    let device_uuid = uuid::Uuid::new_v4().to_string();

    let state = Arc::new(ServerState {
        image_paths,
        server_addr,
        device_uuid,
        folder_name,
        shutdown: AtomicBool::new(false),
    });

    let http_state = Arc::clone(&state);
    let http_thread = std::thread::Builder::new()
        .name("looky-http".into())
        .spawn(move || http::run(server, http_state))
        .ok()?;

    let ssdp_state = Arc::clone(&state);
    let ssdp_thread = std::thread::Builder::new()
        .name("looky-ssdp".into())
        .spawn(move || ssdp::run(ssdp_state))
        .ok()?;

    Some((
        ServerHandle {
            state,
            http_thread: Some(http_thread),
            ssdp_thread: Some(ssdp_thread),
        },
        url,
    ))
}
