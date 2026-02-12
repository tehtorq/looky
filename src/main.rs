mod app;
mod catalog;
mod duplicates;
mod metadata;
mod thumbnail;
mod viewer;
mod watcher;

fn main() -> iced::Result {
    env_logger::init();
    app::run()
}
