mod browser;
mod css;
mod error;
mod font;
mod gui;
mod html;
mod http;
mod js;
mod layout;
mod render;
mod text;
mod url;

use browser::load_page;
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

    let Some(raw_url) = raw_url else {
        print_usage(&program);
        return Ok(());
    };

    let url = Url::parse(&raw_url)?;

    if cli_mode {
        let page = load_page(&url)?;
        println!("{}", page.to_cli_output().trim_end());
    } else {
        gui::run(url)?;
    }

    Ok(())
}

fn print_usage(program: &str) {
    println!("Scratch Browser");
    println!();
    println!("Usage:");
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
