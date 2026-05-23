mod app;
mod catalog;
mod duplicates;
mod fs_scan;
mod key_listener;
mod metadata;
mod server;
mod tasks;
mod thumbnail;
mod ui;
mod update;
mod viewer;

fn main() -> iced::Result {
    env_logger::init();
    app::run()
}
