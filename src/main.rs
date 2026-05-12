mod error;
mod html;
mod http;
mod render;
mod url;

use error::Result;
use html::parse_document;
use http::fetch;
use render::render_document;
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

    let Some(raw_url) = args.next() else {
        print_usage(&program);
        return Ok(());
    };

    let url = Url::parse(&raw_url)?;
    let response = fetch(&url)?;
    let document = parse_document(&String::from_utf8_lossy(&response.body));
    let rendered = render_document(&document);

    println!("URL: {}", response.final_url);
    println!(
        "Status: {} {}",
        response.status_code, response.reason_phrase
    );
    if let Some(content_type) = response.header("content-type") {
        println!("Content-Type: {content_type}");
    }
    println!();

    if rendered.trim().is_empty() {
        println!("[empty document]");
    } else {
        println!("{}", rendered.trim_end());
    }

    Ok(())
}

fn print_usage(program: &str) {
    println!("Scratch Browser");
    println!();
    println!("Usage:");
    println!("  {program} http://example.com");
    println!();
    println!("What it does right now:");
    println!("  - Downloads a page with a hand-rolled HTTP client");
    println!("  - Parses HTML into a tiny DOM tree");
    println!("  - Renders readable text in the terminal");
    println!();
    println!("No Chromium. No WebView. No browser SDK.");
}
