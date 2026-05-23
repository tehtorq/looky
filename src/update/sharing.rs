use iced::Task;

use crate::app::{Looky, Message};
use crate::server;
use crate::ui::qr;

pub fn toggle_sharing(state: &mut Looky) {
    if state.server_handle.is_some() {
        if let Some(session) = state.cast_session.take() {
            session.stop();
        }
        state.cast_target_name = None;
        state.cast_devices.clear();
        state.cast_error = None;
        if let Some(handle) = state.server_handle.take() {
            std::thread::spawn(move || handle.stop());
        }
        state.server_url = None;
        state.qr_handle = None;
    } else if !state.image_paths.is_empty() {
        let folder_name = state
            .folder
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Photos".to_string());
        if let Some((handle, url)) =
            server::start_server(state.image_paths.clone(), folder_name)
        {
            state.qr_handle = Some(qr::render_qr(&url));
            state.server_url = Some(url);
            state.server_handle = Some(handle);
        }
    }
}

pub fn start_cast_scan(state: &mut Looky) -> Task<Message> {
    state.cast_scanning = true;
    state.cast_devices.clear();
    state.cast_error = None;
    Task::perform(
        async { server::cast::discover_devices() },
        Message::CastDevicesFound,
    )
}

pub fn cast_select(state: &mut Looky, i: usize) -> Task<Message> {
    let Some(target) = state.cast_devices.get(i).cloned() else {
        return Task::none();
    };
    state.cast_devices.clear();
    state.cast_error = None;
    let image_url = cast_image_url(state);
    Task::perform(
        async move {
            let session = server::cast::CastSession::connect(target)?;
            if let Some(url) = image_url {
                let _ = session.load_image(&url);
            }
            Ok::<_, String>(session)
        },
        |result| match result {
            Ok(session) => Message::CastConnected(session),
            Err(e) => {
                log::warn!("Cast connect failed: {e}");
                Message::StopCast
            }
        },
    )
}

pub fn stop_cast(state: &mut Looky) {
    if let Some(session) = state.cast_session.take() {
        session.stop();
    }
    state.cast_target_name = None;
    state.cast_devices.clear();
    state.cast_error = None;
}

pub fn cast_current_image(state: &Looky) {
    let Some(session) = &state.cast_session else {
        return;
    };
    let Some(image_url) = cast_image_url(state) else {
        return;
    };
    if let Err(e) = session.load_image(&image_url) {
        log::warn!("Cast send failed: {e}");
    }
}

fn cast_image_url(state: &Looky) -> Option<String> {
    let idx = state.viewer.current_index.or(state.selected_thumb)?;
    let url = state.server_url.as_ref()?;
    let path = &state.image_paths[idx];
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    Some(format!("{url}/cast/{idx}/{filename}"))
}
