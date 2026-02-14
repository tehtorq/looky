use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::dlna;
use super::ServerState;
use crate::thumbnail;

const THUMBS_PER_PAGE: usize = 60;
const THUMB_MAX_SIZE: u32 = 400;
const THUMB_QUALITY: u8 = 80;

type HttpResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

pub fn run(server: tiny_http::Server, state: Arc<ServerState>) {
    let thumb_cache: Arc<Mutex<HashMap<usize, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));

    loop {
        if state.shutdown.load(Ordering::Relaxed) {
            break;
        }
        let request = match server.recv_timeout(Duration::from_secs(1)) {
            Ok(Some(req)) => req,
            Ok(None) => continue,
            Err(_) => break,
        };

        let url = request.url().to_string();
        let method = request.method().to_string();

        log::debug!("HTTP {} {}", method, url);

        let result = route(request, &method, &url, &state, &thumb_cache);

        if let Err(e) = result {
            log::debug!("HTTP response error: {}", e);
        }
    }
}

fn route(
    request: tiny_http::Request,
    method: &str,
    url: &str,
    state: &ServerState,
    thumb_cache: &Arc<Mutex<HashMap<usize, Vec<u8>>>>,
) -> HttpResult {
    match (method, url) {
        ("GET", "/") => serve_gallery(request, state, 0),
        ("GET", path) if path.starts_with("/page/") => {
            let page: usize = path[6..].parse().unwrap_or(0);
            serve_gallery(request, state, page)
        }
        ("GET", path) if path.starts_with("/thumb/") => {
            let index: usize = path[7..].parse().unwrap_or(usize::MAX);
            serve_thumbnail(request, state, index, thumb_cache)
        }
        ("GET", path) if path.starts_with("/image/") => {
            let index: usize = path[7..].parse().unwrap_or(usize::MAX);
            serve_image(request, state, index)
        }
        ("GET", "/dlna/device.xml") => serve_device_xml(request, state),
        ("GET", "/dlna/content.xml") => serve_static_xml(request, dlna::content_directory_scpd()),
        ("GET", "/dlna/connection.xml") => {
            serve_static_xml(request, dlna::connection_manager_scpd())
        }
        ("POST", "/dlna/control/content") => serve_soap_content(request, state),
        ("POST", "/dlna/control/connection") => serve_soap_connection(request),
        ("SUBSCRIBE", _) => serve_subscribe(request),
        _ => serve_404(request),
    }
}

fn respond_html(request: tiny_http::Request, html: String) -> HttpResult {
    let response = tiny_http::Response::from_string(html).with_header(
        "Content-Type: text/html; charset=utf-8"
            .parse::<tiny_http::Header>()
            .unwrap(),
    );
    request.respond(response)?;
    Ok(())
}

fn respond_xml(request: tiny_http::Request, xml: String) -> HttpResult {
    let response = tiny_http::Response::from_string(xml).with_header(
        "Content-Type: text/xml; charset=utf-8"
            .parse::<tiny_http::Header>()
            .unwrap(),
    );
    request.respond(response)?;
    Ok(())
}

fn respond_xml_static(request: tiny_http::Request, xml: &str) -> HttpResult {
    let response = tiny_http::Response::from_string(xml).with_header(
        "Content-Type: text/xml; charset=utf-8"
            .parse::<tiny_http::Header>()
            .unwrap(),
    );
    request.respond(response)?;
    Ok(())
}

fn serve_gallery(request: tiny_http::Request, state: &ServerState, page: usize) -> HttpResult {
    let total = state.image_paths.len();
    let total_pages = (total + THUMBS_PER_PAGE - 1).max(1) / THUMBS_PER_PAGE.max(1);
    let page = page.min(total_pages.saturating_sub(1));
    let start = page * THUMBS_PER_PAGE;
    let end = (start + THUMBS_PER_PAGE).min(total);

    let mut thumbs_html = String::new();
    for i in start..end {
        if let Some(path) = state.image_paths.get(i) {
            let title = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let title_escaped = html_escape(&title);
            thumbs_html.push_str(&format!(
                r#"<a href="/image/{i}" title="{title_escaped}"><img src="/thumb/{i}" loading="lazy" alt="{title_escaped}"></a>"#,
            ));
        }
    }

    let mut pagination = String::new();
    if total_pages > 1 {
        pagination.push_str("<div class=\"pages\">");
        if page > 0 {
            pagination.push_str(&format!(
                r#"<a href="/page/{}">&laquo; Prev</a> "#,
                page - 1
            ));
        }
        pagination.push_str(&format!("Page {} of {}", page + 1, total_pages));
        if page + 1 < total_pages {
            pagination.push_str(&format!(
                r#" <a href="/page/{}">Next &raquo;</a>"#,
                page + 1
            ));
        }
        pagination.push_str("</div>");
    }

    let html = format!(
        r#"<!DOCTYPE html>
<html><head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Looky — {folder}</title>
<style>
body {{ margin: 0; background: #1a1a1a; color: #ccc; font-family: system-ui, sans-serif; }}
.header {{ padding: 12px 16px; background: #222; border-bottom: 1px solid #333; }}
.header h1 {{ margin: 0; font-size: 18px; font-weight: 500; }}
.header .count {{ color: #888; font-size: 14px; }}
.grid {{ display: grid; grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); gap: 4px; padding: 4px; }}
.grid a {{ display: block; aspect-ratio: 1; overflow: hidden; }}
.grid img {{ width: 100%; height: 100%; object-fit: cover; display: block; }}
.pages {{ text-align: center; padding: 16px; }}
.pages a {{ color: #6af; text-decoration: none; margin: 0 8px; }}
</style>
</head><body>
<div class="header">
  <h1>Looky — {folder}</h1>
  <span class="count">{total} photos</span>
</div>
<div class="grid">{thumbs_html}</div>
{pagination}
</body></html>"#,
        folder = html_escape(&state.folder_name),
    );

    respond_html(request, html)
}

fn serve_thumbnail(
    request: tiny_http::Request,
    state: &ServerState,
    index: usize,
    cache: &Arc<Mutex<HashMap<usize, Vec<u8>>>>,
) -> HttpResult {
    if index >= state.image_paths.len() {
        return serve_404(request);
    }

    // Check cache
    {
        let lock = cache.lock().unwrap();
        if let Some(bytes) = lock.get(&index) {
            let response = tiny_http::Response::from_data(bytes.clone())
                .with_header(
                    "Content-Type: image/jpeg"
                        .parse::<tiny_http::Header>()
                        .unwrap(),
                )
                .with_header(
                    "Cache-Control: public, max-age=3600"
                        .parse::<tiny_http::Header>()
                        .unwrap(),
                );
            request.respond(response)?;
            return Ok(());
        }
    }

    // Generate
    let path = &state.image_paths[index];
    let jpeg_bytes = thumbnail::thumbnail_jpeg_bytes(path, THUMB_MAX_SIZE, THUMB_QUALITY);

    // Store in cache
    {
        let mut lock = cache.lock().unwrap();
        lock.insert(index, jpeg_bytes.clone());
    }

    let response = tiny_http::Response::from_data(jpeg_bytes)
        .with_header(
            "Content-Type: image/jpeg"
                .parse::<tiny_http::Header>()
                .unwrap(),
        )
        .with_header(
            "Cache-Control: public, max-age=3600"
                .parse::<tiny_http::Header>()
                .unwrap(),
        );
    request.respond(response)?;
    Ok(())
}

fn serve_image(request: tiny_http::Request, state: &ServerState, index: usize) -> HttpResult {
    if index >= state.image_paths.len() {
        return serve_404(request);
    }

    let path = &state.image_paths[index];
    let file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let mime = dlna::mime_for_path(path);

    let reader = std::io::BufReader::new(file);
    let response = tiny_http::Response::new(
        tiny_http::StatusCode(200),
        vec![
            format!("Content-Type: {mime}")
                .parse::<tiny_http::Header>()
                .unwrap(),
            "Cache-Control: public, max-age=3600"
                .parse::<tiny_http::Header>()
                .unwrap(),
        ],
        reader,
        Some(len as usize),
        None,
    );
    request.respond(response)?;
    Ok(())
}

fn serve_device_xml(request: tiny_http::Request, state: &ServerState) -> HttpResult {
    let xml = dlna::device_xml(&state.device_uuid, &state.folder_name, state.server_addr);
    respond_xml(request, xml)
}

fn serve_static_xml(request: tiny_http::Request, xml: &str) -> HttpResult {
    respond_xml_static(request, xml)
}

fn serve_soap_content(mut request: tiny_http::Request, state: &ServerState) -> HttpResult {
    let mut body = String::new();
    request.as_reader().read_to_string(&mut body)?;
    let xml = dlna::handle_content_directory(&body, state.server_addr, &state.image_paths);
    respond_xml(request, xml)
}

fn serve_soap_connection(mut request: tiny_http::Request) -> HttpResult {
    let mut body = String::new();
    request.as_reader().read_to_string(&mut body)?;
    let xml = dlna::handle_connection_manager(&body);
    respond_xml(request, xml)
}

fn serve_subscribe(request: tiny_http::Request) -> HttpResult {
    let response = tiny_http::Response::from_string("")
        .with_status_code(200)
        .with_header("SID: uuid:dummy".parse::<tiny_http::Header>().unwrap())
        .with_header(
            "TIMEOUT: Second-300"
                .parse::<tiny_http::Header>()
                .unwrap(),
        );
    request.respond(response)?;
    Ok(())
}

fn serve_404(request: tiny_http::Request) -> HttpResult {
    let response = tiny_http::Response::from_string("Not Found").with_status_code(404);
    request.respond(response)?;
    Ok(())
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
