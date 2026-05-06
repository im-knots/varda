use varda::usecases::ui::runner::UIRunner;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .filter_module("naga", log::LevelFilter::Error)
        .filter_module("egui_wgpu", log::LevelFilter::Error)
        .init();

    log::info!("Varda VJ Software - Starting up...");

    UIRunner::new().run()
}
