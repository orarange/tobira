mod browser;
mod css;
mod js_common;
mod error;
mod font;
mod gui;
mod html;
mod http;
mod image;
mod js;
mod js_host;
mod layout;
mod render;
mod site_state;
mod text;
mod url;

use browser::load_page_for_cli;
use error::Result;
use url::Url;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = std::env::args();
    let program = args.next().unwrap_or_else(|| "scratch_browser".to_string());
    let mut cli_mode = false;
    let mut raw_url = None;

    for arg in args {
        match arg.as_str() {
            "--cli" => cli_mode = true,
            "--gui" => cli_mode = false,
            _ if raw_url.is_none() => raw_url = Some(arg),
            _ => {
                print_usage(&program);
                return Ok(());
            }
        }
    }

    if cli_mode {
        let Some(raw_url) = raw_url else {
            print_usage(&program);
            return Ok(());
        };
        let url = Url::parse(&raw_url)?;
        let page = load_page_for_cli(&url)?;
        println!("{}", page.to_cli_output().trim_end());
    } else {
        let initial_url = match raw_url {
            Some(raw_url) => Some(Url::parse(&raw_url)?),
            None => None,
        };
        gui::run(initial_url)?;
    }

    Ok(())
}

fn print_usage(program: &str) {
    println!("Tobira");
    println!();
    println!("Usage:");
    println!("  {program}");
    println!("  {program} http://example.com");
    println!("  {program} --cli http://example.com");
    println!();
    println!("What it does right now:");
    println!("  - Downloads a page with a hand-rolled HTTP client");
    println!("  - Parses HTML into a tiny DOM tree");
    println!("  - Opens a lightweight GUI window with winit + software rendering");
    println!("  - Keeps the terminal renderer behind --cli");
    println!();
    println!("No Chromium. No WebView. No browser SDK.");
}
