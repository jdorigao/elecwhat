mod cli;
mod config;
mod logging;
mod paths;
mod state;
mod window;

use tauri::Manager;

use state::AppState;

pub fn run() {
    let argv: Vec<String> = std::env::args().collect();
    let cli = cli::Cli::parse(&argv);

    let paths = paths::Paths::resolve();
    let config = config::JsonConfig::load(&paths.config_file);
    let config_level = config.get_opt_string("log-level");
    logging::init(cli.log_level.as_deref().or(config_level.as_deref()));

    tracing::info!("{} {}", env!("CARGO_PKG_NAME"), window::version_info());
    tracing::info!("config file {}", paths.config_file.display());

    let state = state::AppState::new(paths, config, &cli);

    tauri::Builder::default()
        .plugin(
            tauri_plugin_single_instance::Builder::new()
                .dbus_id("fr.carru.elecwhat")
                .callback(|app, argv, _cwd| {
                    let cli = cli::Cli::parse(&argv);
                    window::handle_second_instance(app, cli);
                })
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .setup(|app| {
            let state = app.state::<AppState>().inner().clone();
            if state.quit_on_start() {
                tracing::info!("quit requested on the command line");
                app.handle().exit(0);
                return Ok(());
            }
            window::create(app).map_err(|err| err.to_string())?;
            window::maybe_show_config_error(app);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
