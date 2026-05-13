use std::collections::BTreeMap;

use crate::css::{StyledNode, Stylesheet, build_styled_tree, parse_stylesheet};
use crate::error::Result;
use crate::html::{Element, Node, parse_document};
use crate::http::fetch;
use crate::image::{ImageStore, decode_image};
use crate::js::process_document_scripts;
use crate::render::render_document;
use crate::text::decode_text_response;
use crate::url::Url;

const MAX_FRAME_DEPTH: usize = 3;

#[derive(Debug, Clone)]
pub struct BrowserPage {
    pub url: Url,
    pub status_code: u16,
    pub reason_phrase: String,
    pub content_type: Option<String>,
    pub title: String,
    pub styled_document: StyledNode,
    pub images: ImageStore,
    pub rendered: Option<String>,
}

impl BrowserPage {
    pub fn status_text(&self) -> String {
        format!("{} {}", self.status_code, self.reason_phrase)
            .trim()
            .to_string()
    }

    pub fn body_text(&self) -> &str {
        match &self.rendered {
            Some(rendered) => {
                let trimmed = rendered.trim();
                if trimmed.is_empty() {
                    "[empty document]"
                } else {
                    trimmed
                }
            }
            None => "[render unavailable]",
        }
    }

    pub fn to_cli_output(&self) -> String {
        let mut output = String::new();
        output.push_str(&format!("URL: {}\n", self.url));
        output.push_str(&format!("Status: {}\n", self.status_text()));
        if let Some(content_type) = &self.content_type {
            output.push_str(&format!("Content-Type: {content_type}\n"));
        }
        output.push('\n');
        output.push_str(self.body_text());
        output.push('\n');
        output
    }
}

#[derive(Debug, Clone)]
struct LoadedDocumentSource {
    final_url: Url,
    status_code: u16,
    reason_phrase: String,
    content_type: Option<String>,
    title: String,
    document: Node,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FrameSpec {
    src: String,
    title: Option<String>,
}

pub fn load_page(url: &Url) -> Result<BrowserPage> {
    load_page_with_options(url, false)
}

pub fn load_page_for_cli(url: &Url) -> Result<BrowserPage> {
    load_page_with_options(url, true)
}

fn load_page_with_options(url: &Url, include_rendered_output: bool) -> Result<BrowserPage> {
    let source = load_document_source(url, 0)?;
    let stylesheet = collect_stylesheet(&source.document, &source.final_url);
    let images = collect_image_resources(&source.document);
    let rendered = include_rendered_output.then(|| render_document(&source.document));
    let styled_document = build_styled_tree(&source.document, &stylesheet);

    Ok(BrowserPage {
        url: source.final_url,
        status_code: source.status_code,
        reason_phrase: source.reason_phrase,
        content_type: source.content_type,
        title: source.title,
        styled_document,
        images,
        rendered,
    })
}

fn load_document_source(url: &Url, frame_depth: usize) -> Result<LoadedDocumentSource> {
    let response = fetch(url)?;
    let content_type = response.header("content-type").map(str::to_string);
    let text = decode_text_response(&response.body, response.header("content-type"));
    let scripted = process_document_scripts(&text, &response.final_url);
    let mut parsed_document = parse_document(&scripted.html);
    annotate_resource_urls(&mut parsed_document, &response.final_url);
    let original_title = scripted
        .title_override
        .or_else(|| document_title(&parsed_document));
    let document = if frame_depth < MAX_FRAME_DEPTH {
        expand_frames(&parsed_document, &response.final_url, frame_depth + 1)?
            .unwrap_or(parsed_document)
    } else {
        parsed_document
    };
    let title = original_title
        .or_else(|| document_title(&document))
        .or_else(|| first_heading(&document))
        .unwrap_or_else(|| "Scratch Browser".to_string());

    Ok(LoadedDocumentSource {
        final_url: response.final_url,
        status_code: response.status_code,
        reason_phrase: response.reason_phrase,
        content_type,
        title,
        document,
    })
}

fn expand_frames(document: &Node, base_url: &Url, frame_depth: usize) -> Result<Option<Node>> {
    if let Some(frameset) = first_frameset(document)
        && let Some(layout) = expand_frameset(document, frameset, base_url, frame_depth)?
    {
        return Ok(Some(layout));
    }

    let mut frames = collect_frame_specs(document);
    if frames.is_empty() {
        return Ok(None);
    }

    frames.sort_by_key(frame_display_priority);

    let mut body_children = Vec::new();
    let multiple_frames = frames.len() > 1;

    for frame in frames {
        let Ok(frame_url) = base_url.resolve(&frame.src) else {
            continue;
        };

        let Ok(frame_document) = load_document_source(&frame_url, frame_depth) else {
            continue;
        };

        if multiple_frames && let Some(section_title) = frame_section_title(&frame, &frame_document)
        {
            body_children.push(section_heading_node(&section_title));
        }

        body_children.push(frame_document.document);

        if multiple_frames {
            body_children.push(hr_node());
        }
    }

    if body_children.is_empty() {
        return Ok(None);
    }

    if multiple_frames
        && matches!(
            body_children.last(),
            Some(Node::Element(element)) if element.tag_name == "hr"
        )
    {
        body_children.pop();
    }

    let title = document_title(document).unwrap_or_else(|| "Scratch Browser".to_string());
    Ok(Some(synthetic_document(&title, body_children)))
}

fn first_frameset(node: &Node) -> Option<&Element> {
    match node {
        Node::Text(_) => None,
        Node::Element(element) => {
            if element.tag_name == "frameset" {
                return Some(element);
            }

            for child in &element.children {
                if let Some(found) = first_frameset(child) {
                    return Some(found);
                }
            }

            None
        }
    }
}

fn expand_frameset(
    document: &Node,
    frameset: &Element,
    base_url: &Url,
    frame_depth: usize,
) -> Result<Option<Node>> {
    let frames = frameset
        .children
        .iter()
        .filter_map(|child| match child {
            Node::Element(element) if element.tag_name == "frame" => Some(element),
            _ => None,
        })
        .collect::<Vec<_>>();
    if frames.is_empty() {
        return Ok(None);
    }

    let title = document_title(document).unwrap_or_else(|| "Scratch Browser".to_string());
    let cols = frameset
        .attribute("cols")
        .map(|value| parse_frame_tracks(value, frames.len()));
    let rows = frameset
        .attribute("rows")
        .map(|value| parse_frame_tracks(value, frames.len()));

    let mut frame_nodes = Vec::new();
    for frame in frames {
        let Some(src) = frame.attribute("src") else {
            continue;
        };
        let Ok(frame_url) = base_url.resolve(src) else {
            continue;
        };
        let Ok(frame_document) = load_document_source(&frame_url, frame_depth) else {
            continue;
        };
        frame_nodes.push(extract_body_children(&frame_document.document));
    }

    if frame_nodes.is_empty() {
        return Ok(None);
    }

    if let Some(cols) = cols
        && cols.len() == frame_nodes.len()
    {
        return Ok(Some(synthetic_frameset_columns_document(
            &title,
            &cols,
            frame_nodes,
        )));
    }

    if let Some(rows) = rows
        && rows.len() == frame_nodes.len()
    {
        return Ok(Some(synthetic_frameset_rows_document(
            &title,
            &rows,
            frame_nodes,
        )));
    }

    let mut body_children = Vec::new();
    for children in frame_nodes {
        body_children.extend(children);
        body_children.push(hr_node());
    }
    if matches!(
        body_children.last(),
        Some(Node::Element(element)) if element.tag_name == "hr"
    ) {
        body_children.pop();
    }

    Ok(Some(synthetic_document(&title, body_children)))
}

fn collect_frame_specs(document: &Node) -> Vec<FrameSpec> {
    let mut frames = Vec::new();
    collect_frame_specs_into(document, &mut frames);
    frames
}

fn collect_frame_specs_into(node: &Node, output: &mut Vec<FrameSpec>) {
    match node {
        Node::Text(_) => {}
        Node::Element(element) => {
            if matches!(element.tag_name.as_str(), "frame" | "iframe")
                && let Some(src) = element.attribute("src")
                && !src.trim().is_empty()
            {
                let title = element
                    .attribute("title")
                    .or_else(|| element.attribute("name"))
                    .map(str::to_string);
                output.push(FrameSpec {
                    src: src.to_string(),
                    title,
                });
            }

            for child in &element.children {
                collect_frame_specs_into(child, output);
            }
        }
    }
}

fn section_heading_node(title: &str) -> Node {
    Node::Element(Element {
        tag_name: "h2".to_string(),
        attributes: BTreeMap::new(),
        children: vec![Node::Text(title.to_string())],
    })
}

fn frame_section_title(frame: &FrameSpec, frame_document: &LoadedDocumentSource) -> Option<String> {
    if let Some(first_heading) = first_heading(&frame_document.document)
        && first_heading == frame_document.title
    {
        return None;
    }

    if !frame_document.title.trim().is_empty() && frame_document.title != "Scratch Browser" {
        return Some(frame_document.title.clone());
    }

    frame
        .title
        .clone()
        .or_else(|| Some(frame.src.clone()))
        .filter(|title| !title.trim().is_empty())
}

fn frame_display_priority(frame: &FrameSpec) -> u8 {
    let hint = format!(
        "{} {}",
        frame.title.as_deref().unwrap_or_default(),
        frame.src
    )
    .to_ascii_lowercase();

    if hint.contains("menu") || hint.contains("left") || hint.contains("nav") {
        1
    } else {
        0
    }
}

fn hr_node() -> Node {
    Node::Element(Element {
        tag_name: "hr".to_string(),
        attributes: BTreeMap::new(),
        children: Vec::new(),
    })
}

fn synthetic_document(title: &str, body_children: Vec<Node>) -> Node {
    Node::Element(Element {
        tag_name: "document".to_string(),
        attributes: BTreeMap::new(),
        children: vec![Node::Element(Element {
            tag_name: "html".to_string(),
            attributes: BTreeMap::new(),
            children: vec![
                Node::Element(Element {
                    tag_name: "head".to_string(),
                    attributes: BTreeMap::new(),
                    children: vec![Node::Element(Element {
                        tag_name: "title".to_string(),
                        attributes: BTreeMap::new(),
                        children: vec![Node::Text(title.to_string())],
                    })],
                }),
                Node::Element(Element {
                    tag_name: "body".to_string(),
                    attributes: BTreeMap::new(),
                    children: body_children,
                }),
            ],
        })],
    })
}

fn synthetic_frameset_columns_document(
    title: &str,
    tracks: &[FrameTrack],
    frame_nodes: Vec<Vec<Node>>,
) -> Node {
    let cells = frame_nodes
        .into_iter()
        .zip(tracks.iter())
        .map(|(children, track)| {
            Node::Element(Element {
                tag_name: "td".to_string(),
                attributes: table_cell_attributes(track),
                children,
            })
        })
        .collect();

    synthetic_document(
        title,
        vec![Node::Element(Element {
            tag_name: "table".to_string(),
            attributes: full_width_table_attributes(),
            children: vec![Node::Element(Element {
                tag_name: "tr".to_string(),
                attributes: BTreeMap::new(),
                children: cells,
            })],
        })],
    )
}

fn synthetic_frameset_rows_document(
    title: &str,
    tracks: &[FrameTrack],
    frame_nodes: Vec<Vec<Node>>,
) -> Node {
    let rows = frame_nodes
        .into_iter()
        .zip(tracks.iter())
        .map(|(children, track)| {
            Node::Element(Element {
                tag_name: "tr".to_string(),
                attributes: row_attributes(track),
                children: vec![Node::Element(Element {
                    tag_name: "td".to_string(),
                    attributes: BTreeMap::new(),
                    children,
                })],
            })
        })
        .collect();

    synthetic_document(
        title,
        vec![Node::Element(Element {
            tag_name: "table".to_string(),
            attributes: full_width_table_attributes(),
            children: rows,
        })],
    )
}

fn full_width_table_attributes() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("width".to_string(), "100%".to_string()),
        ("border".to_string(), "0".to_string()),
        ("cellpadding".to_string(), "0".to_string()),
        ("cellspacing".to_string(), "0".to_string()),
    ])
}

fn table_cell_attributes(track: &FrameTrack) -> BTreeMap<String, String> {
    let mut attributes = BTreeMap::from([("valign".to_string(), "top".to_string())]);
    match track {
        FrameTrack::Percent(value) => {
            attributes.insert("width".to_string(), format!("{value}%"));
        }
        FrameTrack::Pixels(value) if *value > 0 => {
            attributes.insert("width".to_string(), value.to_string());
        }
        FrameTrack::Flex(_) | FrameTrack::Pixels(_) => {}
    }
    attributes
}

fn row_attributes(track: &FrameTrack) -> BTreeMap<String, String> {
    match track {
        FrameTrack::Percent(value) => BTreeMap::from([("height".to_string(), format!("{value}%"))]),
        FrameTrack::Pixels(value) if *value > 0 => {
            BTreeMap::from([("height".to_string(), value.to_string())])
        }
        FrameTrack::Flex(_) | FrameTrack::Pixels(_) => BTreeMap::new(),
    }
}

fn extract_body_children(node: &Node) -> Vec<Node> {
    match node {
        Node::Text(_) => Vec::new(),
        Node::Element(element) => {
            if element.tag_name == "body" {
                return element.children.clone();
            }

            for child in &element.children {
                let extracted = extract_body_children(child);
                if !extracted.is_empty() {
                    return extracted;
                }
            }

            Vec::new()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameTrack {
    Percent(u32),
    Pixels(u32),
    Flex(u32),
}

fn parse_frame_tracks(input: &str, count: usize) -> Vec<FrameTrack> {
    let tokens = input
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        return vec![FrameTrack::Flex(1); count];
    }

    let all_numeric = tokens
        .iter()
        .all(|value| value.chars().all(|character| character.is_ascii_digit()));
    let numeric_sum = tokens
        .iter()
        .filter_map(|value| value.parse::<u32>().ok())
        .sum::<u32>();

    tokens
        .into_iter()
        .map(|token| {
            if let Some(percent) = token.strip_suffix('%')
                && let Ok(value) = percent.parse::<u32>()
            {
                return FrameTrack::Percent(value);
            }

            if let Some(flex) = token.strip_suffix('*') {
                let weight = flex.parse::<u32>().unwrap_or(1).max(1);
                return FrameTrack::Flex(weight);
            }

            if all_numeric && numeric_sum == 100 {
                return FrameTrack::Percent(token.parse::<u32>().unwrap_or(0));
            }

            FrameTrack::Pixels(token.parse::<u32>().unwrap_or(0))
        })
        .collect()
}

fn annotate_resource_urls(document: &mut Node, base_url: &Url) {
    match document {
        Node::Text(_) => {}
        Node::Element(element) => {
            if element.tag_name == "img"
                && let Some(src) = element.attribute("src")
                && let Ok(url) = base_url.resolve(src)
            {
                element
                    .attributes
                    .insert("data-scratch-src".to_string(), url.to_string());
            }

            if element.tag_name == "body"
                && let Some(background) = element.attribute("background")
                && let Ok(url) = base_url.resolve(background)
            {
                element
                    .attributes
                    .insert("data-scratch-background".to_string(), url.to_string());
            }

            for child in &mut element.children {
                annotate_resource_urls(child, base_url);
            }
        }
    }
}

fn collect_image_resources(document: &Node) -> ImageStore {
    let mut sources = Vec::new();
    collect_image_sources_into(document, &mut sources);

    let mut images = ImageStore::default();
    for source in sources {
        let Ok(url) = Url::parse(&source) else {
            continue;
        };
        let Ok(response) = fetch(&url) else {
            continue;
        };
        let Ok(image) = decode_image(&response.body) else {
            continue;
        };
        images.insert(source, image);
    }

    images
}

fn collect_image_sources_into(node: &Node, output: &mut Vec<String>) {
    match node {
        Node::Text(_) => {}
        Node::Element(element) => {
            if element.tag_name == "img"
                && let Some(src) = element.attribute("data-scratch-src")
                && !output.iter().any(|known| known == src)
            {
                output.push(src.to_string());
            }

            for child in &element.children {
                collect_image_sources_into(child, output);
            }
        }
    }
}

fn collect_stylesheet(document: &Node, base_url: &Url) -> Stylesheet {
    let mut stylesheet = Stylesheet::default();

    for style_text in collect_style_blocks(document) {
        stylesheet.extend(parse_stylesheet(&style_text));
    }

    for href in collect_stylesheet_links(document) {
        let Ok(url) = base_url.resolve(&href) else {
            continue;
        };
        let Ok(response) = fetch(&url) else {
            continue;
        };
        let css_text = decode_text_response(&response.body, response.header("content-type"));
        stylesheet.extend(parse_stylesheet(&css_text));
    }

    stylesheet
}

fn collect_style_blocks(document: &Node) -> Vec<String> {
    let mut blocks = Vec::new();
    collect_style_blocks_into(document, &mut blocks);
    blocks
}

fn collect_style_blocks_into(node: &Node, output: &mut Vec<String>) {
    match node {
        Node::Text(_) => {}
        Node::Element(element) => {
            if element.tag_name == "style" {
                let content = collect_raw_text(node);
                if !content.trim().is_empty() {
                    output.push(content);
                }
                return;
            }

            for child in &element.children {
                collect_style_blocks_into(child, output);
            }
        }
    }
}

fn collect_stylesheet_links(document: &Node) -> Vec<String> {
    let mut links = Vec::new();
    collect_stylesheet_links_into(document, &mut links);
    links
}

fn collect_stylesheet_links_into(node: &Node, output: &mut Vec<String>) {
    match node {
        Node::Text(_) => {}
        Node::Element(element) => {
            if element.tag_name == "link"
                && element
                    .attribute("rel")
                    .map(|value| {
                        value
                            .split_whitespace()
                            .any(|token| token.eq_ignore_ascii_case("stylesheet"))
                    })
                    .unwrap_or(false)
            {
                if let Some(href) = element.attribute("href") {
                    output.push(href.to_string());
                }
            }

            for child in &element.children {
                collect_stylesheet_links_into(child, output);
            }
        }
    }
}

fn document_title(node: &Node) -> Option<String> {
    first_text_by_tag(node, "title")
}

fn first_heading(node: &Node) -> Option<String> {
    match node {
        Node::Text(_) => None,
        Node::Element(element) => {
            if matches!(
                element.tag_name.as_str(),
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            ) {
                let text = collect_text(node);
                if !text.trim().is_empty() {
                    return Some(text.trim().to_string());
                }
            }

            for child in &element.children {
                if let Some(found) = first_heading(child) {
                    return Some(found);
                }
            }

            None
        }
    }
}

fn first_text_by_tag(node: &Node, tag_name: &str) -> Option<String> {
    match node {
        Node::Text(_) => None,
        Node::Element(element) => {
            if element.tag_name == tag_name {
                let text = collect_text(node);
                if !text.trim().is_empty() {
                    return Some(text.trim().to_string());
                }
            }

            for child in &element.children {
                if let Some(found) = first_text_by_tag(child, tag_name) {
                    return Some(found);
                }
            }

            None
        }
    }
}

fn collect_text(node: &Node) -> String {
    let mut text = String::new();
    collect_text_into(node, &mut text);
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn collect_text_into(node: &Node, output: &mut String) {
    match node {
        Node::Text(text) => {
            output.push_str(text);
            output.push(' ');
        }
        Node::Element(element) => {
            if matches!(element.tag_name.as_str(), "script" | "style" | "noscript") {
                return;
            }

            for child in &element.children {
                collect_text_into(child, output);
            }
        }
    }
}

fn collect_raw_text(node: &Node) -> String {
    let mut text = String::new();
    collect_raw_text_into(node, &mut text);
    text
}

fn collect_raw_text_into(node: &Node, output: &mut String) {
    match node {
        Node::Text(text) => output.push_str(text),
        Node::Element(element) => {
            for child in &element.children {
                collect_raw_text_into(child, output);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BrowserPage, collect_frame_specs, collect_stylesheet, document_title,
        extract_body_children, synthetic_document,
    };
    use crate::css::StyledNode;
    use crate::html::{Node, parse_document};
    use crate::url::Url;

    #[test]
    fn falls_back_when_document_is_empty() {
        let page = BrowserPage {
            url: Url::parse("http://example.com").unwrap(),
            status_code: 200,
            reason_phrase: "OK".to_string(),
            content_type: Some("text/html".to_string()),
            title: "Example".to_string(),
            styled_document: StyledNode::Text(crate::css::StyledText {
                text: String::new(),
                style: crate::css::ComputedStyle {
                    display: crate::css::Display::Inline,
                    color: crate::css::DEFAULT_TEXT_COLOR,
                    background_color: None,
                    margin: crate::css::EdgeSizes::default(),
                    padding: crate::css::EdgeSizes::default(),
                    width: None,
                    height: None,
                    font_size_px: 16,
                    font_family: crate::css::FontFamilyKind::Sans,
                    text_align: crate::css::TextAlign::Left,
                    vertical_align: crate::css::VerticalAlign::Top,
                    font_weight: false,
                    underline: false,
                    white_space: crate::css::WhiteSpaceMode::Normal,
                },
            }),
            images: crate::image::ImageStore::default(),
            rendered: Some("   ".to_string()),
        };

        assert_eq!(page.body_text(), "[empty document]");
    }

    #[test]
    fn formats_cli_output() {
        let page = BrowserPage {
            url: Url::parse("http://example.com").unwrap(),
            status_code: 200,
            reason_phrase: "OK".to_string(),
            content_type: Some("text/html".to_string()),
            title: "Hello".to_string(),
            styled_document: parse_styled_text("Hello"),
            images: crate::image::ImageStore::default(),
            rendered: Some("# Hello".to_string()),
        };

        let output = page.to_cli_output();

        assert!(output.contains("URL: http://example.com/"));
        assert!(output.contains("Status: 200 OK"));
        assert!(output.contains("# Hello"));
    }

    #[test]
    fn extracts_embedded_stylesheet_and_title() {
        let document = parse_document(
            "<html><head><title>Demo</title><style>p { color: #ff0000; }</style></head><body><p>Hello</p></body></html>",
        );
        let stylesheet = collect_stylesheet(&document, &Url::parse("http://example.com").unwrap());

        assert_eq!(stylesheet.rules.len(), 1);
        assert_eq!(document_title(&document), Some("Demo".to_string()));
    }

    #[test]
    fn collects_frame_sources() {
        let document = parse_document(
            "<frameset cols=\"18,82\"><frame src=\"menu.htm\" name=\"left\"><frame src=\"top.htm\" name=\"right\"></frameset>",
        );

        let frames = collect_frame_specs(&document);

        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].src, "menu.htm");
        assert_eq!(frames[1].title.as_deref(), Some("right"));
    }

    #[test]
    fn synthetic_document_preserves_title_and_body_content() {
        let document = synthetic_document("Demo", vec![Node::Text("hello".to_string())]);
        let rendered = crate::render::render_document(&document);

        assert!(rendered.contains("# Demo"));
        assert!(rendered.contains("hello"));
    }

    #[test]
    fn extracts_only_body_children_from_document() {
        let document = parse_document(
            "<html><head><title>Demo</title></head><body><p>Hello</p></body></html>",
        );

        let children = extract_body_children(&document);

        assert_eq!(children.len(), 1);
        let Node::Element(paragraph) = &children[0] else {
            panic!("body child should be an element");
        };
        assert_eq!(paragraph.tag_name, "p");
    }

    fn parse_styled_text(text: &str) -> StyledNode {
        crate::css::StyledNode::Text(crate::css::StyledText {
            text: text.to_string(),
            style: crate::css::ComputedStyle {
                display: crate::css::Display::Inline,
                color: crate::css::DEFAULT_TEXT_COLOR,
                background_color: None,
                margin: crate::css::EdgeSizes::default(),
                padding: crate::css::EdgeSizes::default(),
                width: None,
                height: None,
                font_size_px: 16,
                font_family: crate::css::FontFamilyKind::Sans,
                text_align: crate::css::TextAlign::Left,
                vertical_align: crate::css::VerticalAlign::Top,
                font_weight: false,
                underline: false,
                white_space: crate::css::WhiteSpaceMode::Normal,
            },
        })
    }
}
