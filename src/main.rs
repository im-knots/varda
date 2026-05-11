use clap::Parser;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .filter_module("naga", log::LevelFilter::Error)
        .filter_module("egui_wgpu", log::LevelFilter::Error)
        .init();

    let config = varda::app::AppConfig::parse();

    log::info!("Varda VJ Software - Starting up...");
    if config.headless {
        log::info!("Headless mode enabled (API port {})", config.api_port);
    }

    varda::usecases::ui::runner::UIRunner::new(config).run()
}
