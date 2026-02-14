use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::ServerState;

const MULTICAST_ADDR: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);
const SSDP_PORT: u16 = 1900;
const NOTIFY_INTERVAL: Duration = Duration::from_secs(60);

pub fn run(state: Arc<ServerState>) {
    let multicast = SocketAddrV4::new(MULTICAST_ADDR, SSDP_PORT);

    // Try to bind to the standard SSDP port; fall back to random if another server owns it.
    let sock = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, SSDP_PORT))
        .or_else(|_| UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)));

    let sock = match sock {
        Ok(s) => s,
        Err(e) => {
            log::warn!("SSDP: failed to bind socket: {}", e);
            return;
        }
    };

    // Join multicast group
    if let Err(e) = sock.join_multicast_v4(&MULTICAST_ADDR, &Ipv4Addr::UNSPECIFIED) {
        log::warn!("SSDP: failed to join multicast: {}", e);
        // Continue anyway — we can still send NOTIFYs
    }

    let _ = sock.set_read_timeout(Some(Duration::from_secs(2)));

    // Initial alive burst (send 3 times for reliability)
    for _ in 0..3 {
        send_alive(&sock, &state, multicast);
        std::thread::sleep(Duration::from_millis(100));
    }

    let mut last_notify = Instant::now();

    loop {
        if state.shutdown.load(Ordering::Relaxed) {
            break;
        }

        let mut buf = [0u8; 2048];
        match sock.recv_from(&mut buf) {
            Ok((len, src)) => {
                let msg = String::from_utf8_lossy(&buf[..len]);
                if msg.contains("M-SEARCH") {
                    handle_msearch(&sock, &state, &msg, src);
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => {}
        }

        // Periodic NOTIFY alive
        if last_notify.elapsed() >= NOTIFY_INTERVAL {
            send_alive(&sock, &state, multicast);
            last_notify = Instant::now();
        }
    }

    // Bye-bye on shutdown
    send_byebye(&sock, &state, multicast);
}

fn location(state: &ServerState) -> String {
    format!("http://{}/dlna/device.xml", state.server_addr)
}

fn send_alive(sock: &UdpSocket, state: &ServerState, dest: SocketAddrV4) {
    let loc = location(state);
    let uuid = &state.device_uuid;

    let notifications = [
        ("upnp:rootdevice", format!("uuid:{uuid}::upnp:rootdevice")),
        (&format!("uuid:{uuid}"), format!("uuid:{uuid}")),
        (
            "urn:schemas-upnp-org:device:MediaServer:1",
            format!("uuid:{uuid}::urn:schemas-upnp-org:device:MediaServer:1"),
        ),
        (
            "urn:schemas-upnp-org:service:ContentDirectory:1",
            format!("uuid:{uuid}::urn:schemas-upnp-org:service:ContentDirectory:1"),
        ),
        (
            "urn:schemas-upnp-org:service:ConnectionManager:1",
            format!("uuid:{uuid}::urn:schemas-upnp-org:service:ConnectionManager:1"),
        ),
    ];

    for (nt, usn) in &notifications {
        let msg = format!(
            "NOTIFY * HTTP/1.1\r\n\
             HOST: 239.255.255.250:1900\r\n\
             CACHE-CONTROL: max-age=1800\r\n\
             LOCATION: {loc}\r\n\
             NT: {nt}\r\n\
             NTS: ssdp:alive\r\n\
             SERVER: Looky/1.0 UPnP/1.0\r\n\
             USN: {usn}\r\n\
             \r\n"
        );
        let _ = sock.send_to(msg.as_bytes(), dest);
    }
}

fn send_byebye(sock: &UdpSocket, state: &ServerState, dest: SocketAddrV4) {
    let uuid = &state.device_uuid;

    let notifications = [
        ("upnp:rootdevice", format!("uuid:{uuid}::upnp:rootdevice")),
        (
            "urn:schemas-upnp-org:device:MediaServer:1",
            format!("uuid:{uuid}::urn:schemas-upnp-org:device:MediaServer:1"),
        ),
    ];

    for (nt, usn) in &notifications {
        let msg = format!(
            "NOTIFY * HTTP/1.1\r\n\
             HOST: 239.255.255.250:1900\r\n\
             NT: {nt}\r\n\
             NTS: ssdp:byebye\r\n\
             USN: {usn}\r\n\
             \r\n"
        );
        let _ = sock.send_to(msg.as_bytes(), dest);
    }
}

fn handle_msearch(sock: &UdpSocket, state: &ServerState, msg: &str, src: SocketAddr) {
    let st = extract_header(msg, "ST").unwrap_or_default();

    let should_respond = matches!(
        st.as_str(),
        "ssdp:all"
            | "upnp:rootdevice"
            | "urn:schemas-upnp-org:device:MediaServer:1"
            | "urn:schemas-upnp-org:service:ContentDirectory:1"
    ) || st.starts_with("uuid:");

    if !should_respond {
        return;
    }

    let loc = location(state);
    let uuid = &state.device_uuid;
    let usn = if st == "ssdp:all" || st.starts_with("uuid:") {
        format!("uuid:{uuid}")
    } else {
        format!("uuid:{uuid}::{st}")
    };
    let response_st = if st == "ssdp:all" {
        "upnp:rootdevice"
    } else {
        &st
    };

    let response = format!(
        "HTTP/1.1 200 OK\r\n\
         CACHE-CONTROL: max-age=1800\r\n\
         LOCATION: {loc}\r\n\
         SERVER: Looky/1.0 UPnP/1.0\r\n\
         ST: {response_st}\r\n\
         USN: {usn}\r\n\
         EXT:\r\n\
         \r\n"
    );
    let _ = sock.send_to(response.as_bytes(), src);
}

fn extract_header(msg: &str, name: &str) -> Option<String> {
    let search = format!("{}:", name);
    for line in msg.lines() {
        let trimmed = line.trim();
        if trimmed.len() > search.len()
            && trimmed[..search.len()].eq_ignore_ascii_case(&search)
        {
            return Some(trimmed[search.len()..].trim().to_string());
        }
    }
    None
}
