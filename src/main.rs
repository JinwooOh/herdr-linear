mod app;
mod config;
mod display;
mod github;
mod launch;
mod linear;
mod merge;
mod process;
mod review;
mod ui;
mod worker;

use anyhow::Result;

fn main() -> Result<()> {
    if std::env::args().any(|a| a == "--opened-tab") {
        use std::io::Read;
        let mut json = String::new();
        std::io::stdin().take(1_000_000).read_to_string(&mut json)?;
        println!("{}", launch::opened_tab(&json));
        return Ok(());
    }
    let tab_mode = std::env::args().any(|a| a == "--launch-decision-tab");
    if tab_mode || std::env::args().any(|a| a == "--launch-decision") {
        use std::io::Read;
        let mut json = String::new();
        std::io::stdin().take(4_000_000).read_to_string(&mut json)?;
        let caller_tab = std::env::var("HERDR_TAB_ID").ok();
        let decide = if tab_mode {
            launch::decision_tab
        } else {
            launch::decision
        };
        println!("{}", decide(&json, caller_tab.as_deref()));
        return Ok(());
    }
    let config = config::load()?;
    let mut app = app::App::new(config.accounts);
    app.merge_options = config.merge;

    let mut terminal = ratatui::init();
    worker::install_panic_hook();
    let result = app.run(&mut terminal);
    ratatui::restore();
    result
}
