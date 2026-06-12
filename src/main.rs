mod browser;
mod css;
mod engine_host;
mod error;
mod font;
mod gui;
mod html;
mod http;
mod image;
mod js;
mod layout;
mod render;
mod site_state;
mod text;
mod url;

use browser::load_page_for_cli;
use error::Result;
use url::Url;

/// Build provenance, e.g. `0.1.0 (g6f32cd9, 2026-06-12)`. The git short hash —
/// with a `-dirty` suffix when the build carried uncommitted changes — is the
/// reliable signal that a specific patch is actually in the running binary.
/// (`TOBIRA_GIT_HASH` / `TOBIRA_COMMIT_DATE` are injected by `build.rs`.)
pub fn version_string() -> String {
    let semver = env!("CARGO_PKG_VERSION");
    let hash = option_env!("TOBIRA_GIT_HASH").unwrap_or("unknown");
    let date = option_env!("TOBIRA_COMMIT_DATE").unwrap_or("");
    if date.is_empty() {
        format!("{semver} (g{hash})")
    } else {
        format!("{semver} (g{hash}, {date})")
    }
}

/// Compact build badge for the on-screen title bar, e.g. `v0.1.0 g6f32cd9`.
pub fn version_badge() -> String {
    let semver = env!("CARGO_PKG_VERSION");
    let hash = option_env!("TOBIRA_GIT_HASH").unwrap_or("unknown");
    format!("v{semver} g{hash}")
}

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
            "--version" | "-V" => {
                println!("Tobira {}", version_string());
                return Ok(());
            }
            "--cli" => cli_mode = true,
            "--gui" => cli_mode = false,
            _ if raw_url.is_none() => raw_url = Some(arg),
            _ => {
                print_usage(&program);
                return Ok(());
            }
        }
    }

    // Startup banner so the running revision is visible in the launching shell.
    eprintln!("Tobira {}", version_string());

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
    println!("Tobira {}", version_string());
    println!();
    println!("Usage:");
    println!("  {program}");
    println!("  {program} http://example.com");
    println!("  {program} --cli http://example.com");
    println!("  {program} --version");
    println!();
    println!("What it does right now:");
    println!("  - Downloads a page with a hand-rolled HTTP client");
    println!("  - Parses HTML into a tiny DOM tree");
    println!("  - Opens a lightweight GUI window with winit + software rendering");
    println!("  - Keeps the terminal renderer behind --cli");
    println!();
    println!("No Chromium. No WebView. No browser SDK.");
}
