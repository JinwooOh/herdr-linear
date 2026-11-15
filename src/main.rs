mod app;
mod config;
mod display;
mod github;
mod launch;
mod linear;
mod process;
mod review;
mod ui;
mod worker;

use anyhow::Result;

fn main() -> Result<()> {
    if std::env::args().any(|a| a == "--launch-decision") {
        use std::io::Read;
        let mut json = String::new();
        std::io::stdin().take(4_000_000).read_to_string(&mut json)?;
        println!(
            "{}",
            launch::decision(&json, std::env::var("HERDR_TAB_ID").ok().as_deref())
        );
        return Ok(());
    }
    let accounts = config::load()?;
    let mut app = app::App::new(accounts);

    let mut terminal = ratatui::init();
    worker::install_panic_hook();
    let result = app.run(&mut terminal);
    ratatui::restore();
    result
}
