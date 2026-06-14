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
    let mut dump_styled = false;
    let mut raw_url = None;

    for arg in args {
        match arg.as_str() {
            "--version" | "-V" => {
                println!("Tobira {}", version_string());
                return Ok(());
            }
            "--cli" => cli_mode = true,
            "--gui" => cli_mode = false,
            "--dump-styled" => dump_styled = true,
            _ if raw_url.is_none() => raw_url = Some(arg),
            _ => {
                print_usage(&program);
                return Ok(());
            }
        }
    }

    if dump_styled {
        let Some(raw_url) = raw_url else {
            print_usage(&program);
            return Ok(());
        };
        return dump_styled_layout(&Url::parse(&raw_url)?);
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

/// Debug: load a page, then report how much of the styled tree is hidden
/// (display:none) vs visible, and how the layout engine sizes it. Distinguishes
/// "content is in the DOM but CSS/JS hides it" from "layout collapses it".
fn dump_styled_layout(url: &Url) -> Result<()> {
    use css::{Display, StyledNode};
    let page = load_page_for_cli(url)?;

    fn walk(
        node: &StyledNode,
        hidden: bool,
        depth: usize,
        stats: &mut (usize, usize, usize, usize),
    ) {
        match node {
            StyledNode::Text(t) => {
                let len = t.text.split_whitespace().collect::<Vec<_>>().join(" ").len();
                if hidden {
                    stats.3 += len;
                } else {
                    stats.2 += len;
                }
            }
            StyledNode::Element(e) => {
                stats.0 += 1;
                let now_hidden = hidden || matches!(e.style.display, Display::None);
                if matches!(e.style.display, Display::None) {
                    stats.1 += 1;
                }
                if depth <= 4 {
                    let cls: String = e
                        .attributes
                        .get("class")
                        .map(|c| c.chars().take(40).collect())
                        .unwrap_or_default();
                    let inline: String = e
                        .attributes
                        .get("style")
                        .map(|c| c.chars().take(70).collect())
                        .unwrap_or_default();
                    println!(
                        "{}<{} class=\"{}\" display={:?} opacity={} style=\"{}\">{}",
                        "  ".repeat(depth),
                        e.tag_name,
                        cls,
                        e.style.display,
                        e.style.opacity,
                        inline,
                        if matches!(e.style.display, Display::None) { "  [none]" } else if e.style.opacity == 0 { "  [OPACITY:0]" } else { "" },
                    );
                }
                for c in &e.children {
                    walk(c, now_hidden, depth + 1, stats);
                }
            }
        }
    }

    let mut stats = (0usize, 0usize, 0usize, 0usize);
    walk(&page.styled_document, false, 0, &mut stats);
    println!("\n=== summary ===");
    println!("elements            = {}", stats.0);
    println!("display:none nodes  = {}", stats.1);
    println!("visible text bytes  = {}", stats.2);
    println!("hidden  text bytes  = {}", stats.3);

    // For each display:none *root* (ancestor not already hidden), report how much
    // text it hides and why, so we can tell script/style/head from real content.
    fn subtree_text(node: &StyledNode) -> usize {
        match node {
            StyledNode::Text(t) => t.text.split_whitespace().collect::<Vec<_>>().join(" ").len(),
            StyledNode::Element(e) => e.children.iter().map(subtree_text).sum(),
        }
    }
    fn find_hidden_roots<'a>(node: &'a StyledNode, hidden: bool, out: &mut Vec<&'a css::StyledElement>) {
        if let StyledNode::Element(e) = node {
            let is_none = matches!(e.style.display, Display::None);
            if is_none && !hidden {
                out.push(e);
            }
            for c in &e.children {
                find_hidden_roots(c, hidden || is_none, out);
            }
        }
    }
    let mut roots = Vec::new();
    find_hidden_roots(&page.styled_document, false, &mut roots);
    let mut rooted: Vec<_> = roots
        .iter()
        .map(|e| (subtree_text(&StyledNode::Element((*e).clone())), *e))
        .collect();
    rooted.sort_by_key(|(n, _)| std::cmp::Reverse(*n));
    println!("\n=== top display:none roots (hidden text bytes : tag#id.class | style) ===");
    for (bytes, e) in rooted.iter().take(15) {
        let id = e.attributes.get("id").cloned().unwrap_or_default();
        let cls: String = e.attributes.get("class").map(|c| c.chars().take(40).collect()).unwrap_or_default();
        let style: String = e.attributes.get("style").map(|c| c.chars().take(60).collect()).unwrap_or_default();
        println!("  {:>8} : {}#{}.{} | style=\"{}\"", bytes, e.tag_name, id, cls, style);
    }

    // What does the visible (non-hidden) text actually say?
    fn collect_visible(node: &StyledNode, hidden: bool, out: &mut Vec<String>) {
        match node {
            StyledNode::Text(t) => {
                let s = t.text.split_whitespace().collect::<Vec<_>>().join(" ");
                if !hidden && !s.is_empty() {
                    out.push(s);
                }
            }
            StyledNode::Element(e) => {
                let h = hidden || matches!(e.style.display, Display::None);
                for c in &e.children {
                    collect_visible(c, h, out);
                }
            }
        }
    }
    let mut vis = Vec::new();
    collect_visible(&page.styled_document, false, &mut vis);
    println!("\n=== visible text ({} runs) ===", vis.len());
    println!("{}", vis.join(" | "));

    let mut fonts = font::FontContext::load();
    let layout = layout::layout_styled_document(&page.styled_document, &page.images, 1280, &mut fonts);
    println!("\n=== layout (viewport_width=1280) ===");
    println!("content_height      = {}", layout.content_height);
    println!("draw commands       = {}", layout.commands.len());
    for (i, cmd) in layout.commands.iter().enumerate().take(12) {
        let s = format!("{cmd:?}");
        println!("  cmd[{i}] = {}", s.chars().take(160).collect::<String>());
    }
    println!("element hitboxes    = {}", layout.element_hitboxes.len());
    let mut boxes = layout.element_hitboxes.clone();
    boxes.sort_by_key(|b| std::cmp::Reverse(u64::from(b.width) * u64::from(b.height)));
    println!("largest boxes (node: WxH @ x,y):");
    for b in boxes.iter().take(10) {
        println!("  node {} : {}x{} @ {},{}", b.node_id, b.width, b.height, b.x, b.y);
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
