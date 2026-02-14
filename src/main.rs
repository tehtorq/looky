mod app;
mod catalog;
mod duplicates;
mod key_listener;
mod metadata;
mod server;
mod thumbnail;
mod viewer;
mod watcher;

fn main() -> iced::Result {
    env_logger::init();
    app::run()
}
