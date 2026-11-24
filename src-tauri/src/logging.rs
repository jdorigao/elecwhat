use tracing_subscriber::filter::LevelFilter;

pub fn init(level: Option<&str>) {
    let filter = match level.unwrap_or("info").to_ascii_lowercase().as_str() {
        "trace" => LevelFilter::TRACE,
        "debug" => LevelFilter::DEBUG,
        "warn" => LevelFilter::WARN,
        "error" => LevelFilter::ERROR,
        "off" | "silent" => LevelFilter::OFF,
        _ => LevelFilter::INFO,
    };

    let _ = tracing_subscriber::fmt()
        .with_max_level(filter)
        .with_target(false)
        .try_init();
}
