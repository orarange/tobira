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
mod svg;
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
                if depth <= dump_depth() && !t.text.trim().is_empty() {
                    println!(
                        "{}#text {:?}",
                        "  ".repeat(depth),
                        t.text.chars().take(60).collect::<String>()
                    );
                }
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
                if depth <= dump_depth() {
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
                        "{}<{} class=\"{}\" display={:?} position={:?} bg={} color={:#08x} opacity={} style=\"{}\">{}",
                        "  ".repeat(depth),
                        e.tag_name,
                        cls,
                        e.style.display,
                        e.style.position,
                        // Separates "the rule never matched" from "it matched and
                        // we failed to paint it" -- the two look identical on
                        // screen, and guessing between them from the stylesheet
                        // has cost more than one wrong turn.
                        match e.style.background_color {
                            Some(c) => format!("{c:#08x}"),
                            None => "none".to_string(),
                        },
                        e.style.color,
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

    fn dump_depth() -> usize {
        std::env::var("TOBIRA_DUMP_DEPTH")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(4)
    }

    let mut stats = (0usize, 0usize, 0usize, 0usize);
    walk(&page.styled_document, false, 0, &mut stats);
    println!("\n=== summary ===");
    println!("elements            = {}", stats.0);
    println!("display:none nodes  = {}", stats.1);
    println!("visible text bytes  = {}", stats.2);
    println!("hidden  text bytes  = {}", stats.3);

    // Under TOBIRA_DEBUG_CSS this is the ranked list of declarations the engine
    // parsed out of the page's stylesheets and then threw away, which is the
    // worklist for closing the gap against a real browser.
    let unsupported = css::unsupported_property_report();
    if !unsupported.is_empty() {
        let total: u32 = unsupported.iter().map(|(_, count)| count).sum();
        println!(
            "\n=== unsupported declarations ({total} across {} properties) ===",
            unsupported.len()
        );
        for (property, count) in &unsupported {
            println!("{count:8} : {property}");
        }
    }

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

    // A layer prints as one line however much is inside it, which hides exactly
    // what you need when a sticky header or a clipped box looks wrong. Set
    // TOBIRA_DUMP_LAYERS=1 to walk into them; inner coordinates are relative to
    // the layer's own origin.
    if std::env::var_os("TOBIRA_DUMP_LAYERS").is_some() {
        fn walk(commands: &[layout::DrawCommand], depth: usize) {
            for cmd in commands {
                let indent = "  ".repeat(depth + 1);
                match cmd {
                    layout::DrawCommand::Layer(l) => {
                        println!("{indent}Layer {}x{} @ {},{}", l.width, l.height, l.x, l.y);
                        walk(&l.commands, depth + 1);
                    }
                    layout::DrawCommand::Sticky(s) => {
                        println!(
                            "{indent}Sticky normal_y={} -> Layer {}x{} @ {},{}",
                            s.normal_y, s.layer.width, s.layer.height, s.layer.x, s.layer.y
                        );
                        walk(&s.layer.commands, depth + 1);
                    }
                    other => {
                        let text = format!("{other:?}");
                        println!("{indent}{}", text.chars().take(120).collect::<String>());
                    }
                }
            }
        }
        println!("=== command tree ===");
        walk(&layout.commands, 0);
    }
    println!("element hitboxes    = {}", layout.element_hitboxes.len());

    // What kinds of command the page actually produced, and how many images
    // are formats the decoder cannot read.
    {
        use std::collections::BTreeMap;
        fn tally(cmds: &[layout::DrawCommand], counts: &mut BTreeMap<&'static str, u32>, svg: &mut u32, other: &mut u32) {
            for cmd in cmds {
                let name = match cmd {
                    layout::DrawCommand::Rect(_) => "rect",
                    layout::DrawCommand::Text(_) => "text",
                    layout::DrawCommand::Image(i) => {
                        let is_svg = i.src.contains("image/svg") || i.src.trim_end().ends_with(".svg");
                        if is_svg { *svg += 1 } else { *other += 1 }
                        "image"
                    }
                    layout::DrawCommand::Gradient(_) => "gradient",
                    layout::DrawCommand::Sticky(_) => "sticky",
                    layout::DrawCommand::Layer(l) => {
                        tally(&l.commands, counts, svg, other);
                        "layer"
                    }
                };
                *counts.entry(name).or_insert(0) += 1;
            }
        }
        let mut counts = BTreeMap::new();
        let (mut svg, mut other) = (0, 0);
        tally(&layout.commands, &mut counts, &mut svg, &mut other);
        let summary: Vec<String> = counts.iter().map(|(k, v)| format!("{k}={v}")).collect();
        println!("command mix         = {}", summary.join(" "));
        println!("images              = {svg} svg / {other} raster");
        if std::env::var_os("TOBIRA_DUMP_IMAGES").is_some() {
            fn walk(cmds: &[layout::DrawCommand]) {
                for cmd in cmds {
                    match cmd {
                        layout::DrawCommand::Image(i) => println!(
                            "  {:5},{:5} {:4}x{:<4} {}",
                            i.x, i.y, i.width, i.height,
                            i.src.chars().take(60).collect::<String>()
                        ),
                        layout::DrawCommand::Layer(l) => walk(&l.commands),
                        _ => {}
                    }
                }
            }
            println!("=== image commands ===");
            walk(&layout.commands);
        }
    }
    let mut boxes = layout.element_hitboxes.clone();
    boxes.sort_by_key(|b| std::cmp::Reverse(u64::from(b.width) * u64::from(b.height)));
    println!("largest boxes (node: WxH @ x,y):");
    for b in boxes.iter().take(10) {
        println!("  node {} : {}x{} @ {},{}", b.node_id, b.width, b.height, b.x, b.y);
    }

    // Two runs of text drawn over each other is the loudest rendering defect a
    // page can have and the hardest to spot by reading a command list, so count
    // it directly. Text boxes in a correct layout never intersect: the line
    // breaker gives each run its own strip, and boxes that share a strip are
    // laid out side by side. Any intersection is a positioning bug.
    let texts = layout.texts();
    if std::env::var_os("TOBIRA_DUMP_TEXT").is_some() {
        println!("
=== all text runs (x,y wxh size) ===");
        for t in &texts {
            println!(
                "  {:5},{:5} {:4}x{:<3} {:2}px : {:?}",
                t.x, t.y, t.width, t.line_height_px, t.font_size_px, t.text
            );
        }
    }
    let mut collisions: Vec<(usize, usize, u32)> = Vec::new();
    for (i, a) in texts.iter().enumerate() {
        for (j, b) in texts.iter().enumerate().skip(i + 1) {
            let overlap_w = (a.x + a.width).min(b.x + b.width).saturating_sub(a.x.max(b.x));
            let a_bottom = a.y + a.line_height_px;
            let b_bottom = b.y + b.line_height_px;
            let overlap_h = a_bottom.min(b_bottom).saturating_sub(a.y.max(b.y));
            if overlap_w > 0 && overlap_h > 0 {
                collisions.push((i, j, overlap_w * overlap_h));
            }
        }
    }
    collisions.sort_by_key(|(_, _, area)| std::cmp::Reverse(*area));
    println!(
        "\n=== overlapping text runs ({} of {} runs collide) ===",
        collisions.len(),
        texts.len()
    );
    for (i, j, area) in collisions.iter().take(15) {
        let (a, b) = (&texts[*i], &texts[*j]);
        let clip = |t: &str| t.chars().take(24).collect::<String>();
        println!(
            "  {area:7}px^2 : {:?} @ {},{} {}x{}  X  {:?} @ {},{} {}x{}",
            clip(&a.text), a.x, a.y, a.width, a.line_height_px,
            clip(&b.text), b.x, b.y, b.width, b.line_height_px,
        );
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
