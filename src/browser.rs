use std::collections::BTreeMap;

use serde_json::Value;

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
    if is_youtube_watch_url(&response.final_url)
        && let Some(data) = extract_youtube_watch_data_from_html(&text, &response.final_url)
    {
        let title = data.title.clone();
        return Ok(LoadedDocumentSource {
            final_url: response.final_url,
            status_code: response.status_code,
            reason_phrase: response.reason_phrase,
            content_type,
            title,
            document: build_youtube_watch_document(&data),
        });
    }
    if is_google_host(&response.final_url) {
        let document = build_google_document_from_html(&text, &response.final_url);
        let title = document_title(&document).unwrap_or_else(|| "Google".to_string());
        return Ok(LoadedDocumentSource {
            final_url: response.final_url,
            status_code: response.status_code,
            reason_phrase: response.reason_phrase,
            content_type,
            title,
            document,
        });
    }
    if is_youtube_host(&response.final_url) && !is_youtube_watch_url(&response.final_url) {
        let document = build_youtube_generic_document_from_html(&text, &response.final_url);
        let title = document_title(&document).unwrap_or_else(|| "YouTube".to_string());
        return Ok(LoadedDocumentSource {
            final_url: response.final_url,
            status_code: response.status_code,
            reason_phrase: response.reason_phrase,
            content_type,
            title,
            document,
        });
    }
    let scripted = process_document_scripts(&text, &response.final_url);
    let mut parsed_document = parse_document(&scripted.html);
    if let Some(rewritten) =
        build_site_specific_document(&parsed_document, &text, &response.final_url)
    {
        parsed_document = rewritten;
    }
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct YouTubeRelatedVideo {
    title: String,
    channel: Option<String>,
    views: Option<String>,
    published: Option<String>,
    duration: Option<String>,
    url: Option<String>,
    thumbnail_url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct YouTubeCommentPreview {
    author: String,
    body: String,
    likes: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct YouTubeWatchData {
    title: String,
    channel: Option<String>,
    subscribers: Option<String>,
    description: Option<String>,
    view_count: Option<String>,
    duration: Option<String>,
    published: Option<String>,
    thumbnail_url: Option<String>,
    canonical_url: Option<String>,
    embed_url: Option<String>,
    comment_count: Option<String>,
    related_videos: Vec<YouTubeRelatedVideo>,
    featured_comments: Vec<YouTubeCommentPreview>,
}

fn build_site_specific_document(document: &Node, html: &str, url: &Url) -> Option<Node> {
    if is_youtube_watch_url(url) {
        let data = extract_youtube_watch_data(document, html, url)?;
        return Some(build_youtube_watch_document(&data));
    }

    if is_youtube_host(url) {
        return Some(build_youtube_generic_document(document, html, url));
    }

    None
}

fn is_youtube_host(url: &Url) -> bool {
    let host = url.host.to_ascii_lowercase();
    host == "youtube.com" || host.ends_with(".youtube.com")
}

fn is_google_host(url: &Url) -> bool {
    let host = url.host.to_ascii_lowercase();
    host == "google.com" || host.ends_with(".google.com")
}

fn is_youtube_watch_url(url: &Url) -> bool {
    is_youtube_host(url) && url.path.starts_with("/watch")
}

fn extract_youtube_watch_data_from_html(html: &str, url: &Url) -> Option<YouTubeWatchData> {
    let player_response = extract_assigned_json_object(
        html,
        &[
            "var ytInitialPlayerResponse =",
            "window['ytInitialPlayerResponse'] =",
            "window[\"ytInitialPlayerResponse\"] =",
        ],
    )
    .and_then(|json| serde_json::from_str::<Value>(&json).ok());
    let initial_data = extract_assigned_json_object(
        html,
        &[
            "var ytInitialData =",
            "window['ytInitialData'] =",
            "window[\"ytInitialData\"] =",
        ],
    )
    .and_then(|json| serde_json::from_str::<Value>(&json).ok());
    let ld_json = extract_first_ld_json_video_object(html);

    let title = player_response
        .as_ref()
        .and_then(|value| value.pointer("/videoDetails/title"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| extract_meta_content_from_html(html, "name", "title"))
        .or_else(|| extract_meta_content_from_html(html, "property", "og:title"))
        .or_else(|| extract_html_tag_text(html, "title"))?;

    let description = player_response
        .as_ref()
        .and_then(|value| value.pointer("/videoDetails/shortDescription"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| extract_meta_content_from_html(html, "name", "description"))
        .or_else(|| extract_meta_content_from_html(html, "property", "og:description"));
    let channel = player_response
        .as_ref()
        .and_then(|value| value.pointer("/videoDetails/author"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            initial_data
                .as_ref()
                .and_then(extract_owner_renderer)
                .and_then(|owner| json_text(owner.get("title")?))
        })
        .or_else(|| {
            ld_json
                .as_ref()
                .and_then(|value| value.pointer("/author/name"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    let subscribers = initial_data
        .as_ref()
        .and_then(extract_owner_renderer)
        .and_then(|owner| json_text(owner.get("subscriberCountText")?));
    let view_count = player_response
        .as_ref()
        .and_then(|value| value.pointer("/videoDetails/viewCount"))
        .and_then(Value::as_str)
        .map(format_numeric_count)
        .or_else(|| {
            ld_json
                .as_ref()
                .and_then(|value| value.pointer("/interactionStatistic/1/userInteractionCount"))
                .and_then(Value::as_str)
                .map(format_numeric_count)
        });
    let duration = player_response
        .as_ref()
        .and_then(|value| value.pointer("/videoDetails/lengthSeconds"))
        .and_then(Value::as_str)
        .and_then(format_duration_seconds)
        .or_else(|| {
            ld_json
                .as_ref()
                .and_then(|value| value.get("duration"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .and_then(parse_iso8601_duration)
        });
    let published = player_response
        .as_ref()
        .and_then(|value| {
            value
                .pointer("/microformat/playerMicroformatRenderer/publishDate")
                .and_then(Value::as_str)
        })
        .map(str::to_string)
        .or_else(|| {
            ld_json
                .as_ref()
                .and_then(|value| value.get("uploadDate"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    let thumbnail_url = player_response
        .as_ref()
        .and_then(extract_thumbnail_url)
        .or_else(|| extract_link_href_from_html(html, "rel", "image_src"))
        .or_else(|| extract_meta_content_from_html(html, "property", "og:image"));
    let canonical_url = extract_link_href_from_html(html, "rel", "canonical")
        .or_else(|| extract_meta_content_from_html(html, "property", "og:url"))
        .or_else(|| Some(url.to_string()));
    let embed_url = player_response
        .as_ref()
        .and_then(|value| {
            value
                .pointer("/microformat/playerMicroformatRenderer/embed/iframeUrl")
                .and_then(Value::as_str)
        })
        .map(str::to_string)
        .or_else(|| extract_meta_content_from_html(html, "property", "og:video:url"))
        .or_else(|| {
            player_response
                .as_ref()
                .and_then(|value| value.pointer("/videoDetails/videoId"))
                .and_then(Value::as_str)
                .map(|video_id| format!("https://www.youtube.com/embed/{video_id}"))
        });
    let comment_count = initial_data
        .as_ref()
        .and_then(extract_comments_panel_header)
        .and_then(|header| json_text(header.get("contextualInfo")?));
    let related_videos = initial_data
        .as_ref()
        .map(extract_related_videos)
        .unwrap_or_default();
    let featured_comments = ld_json
        .as_ref()
        .map(extract_featured_comments_from_ld_json)
        .unwrap_or_default();

    Some(YouTubeWatchData {
        title,
        channel,
        subscribers,
        description,
        view_count,
        duration,
        published,
        thumbnail_url,
        canonical_url,
        embed_url,
        comment_count,
        related_videos,
        featured_comments,
    })
}

fn extract_youtube_watch_data(document: &Node, html: &str, url: &Url) -> Option<YouTubeWatchData> {
    let html_data = extract_youtube_watch_data_from_html(html, url)?;
    let player_response = extract_assigned_json_object(
        html,
        &[
            "var ytInitialPlayerResponse =",
            "window['ytInitialPlayerResponse'] =",
            "window[\"ytInitialPlayerResponse\"] =",
        ],
    )
    .and_then(|json| serde_json::from_str::<Value>(&json).ok());

    let title = player_response
        .as_ref()
        .and_then(|value| value.pointer("/videoDetails/title"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| find_meta_content(document, "name", "title"))
        .or_else(|| find_meta_content(document, "property", "og:title"))
        .or_else(|| document_title(document))
        .unwrap_or(html_data.title);

    let description = player_response
        .as_ref()
        .and_then(|value| value.pointer("/videoDetails/shortDescription"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| find_meta_content(document, "name", "description"))
        .or_else(|| find_meta_content(document, "property", "og:description"))
        .or(html_data.description);
    let channel = player_response
        .as_ref()
        .and_then(|value| value.pointer("/videoDetails/author"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| find_link_content(document, "itemprop", "name"))
        .or(html_data.channel);
    let view_count = player_response
        .as_ref()
        .and_then(|value| value.pointer("/videoDetails/viewCount"))
        .and_then(Value::as_str)
        .map(format_numeric_count)
        .or_else(|| find_interaction_count(document, "WatchAction"))
        .or(html_data.view_count);
    let duration = player_response
        .as_ref()
        .and_then(|value| value.pointer("/videoDetails/lengthSeconds"))
        .and_then(Value::as_str)
        .and_then(format_duration_seconds)
        .or_else(|| {
            find_meta_content(document, "itemprop", "duration").and_then(parse_iso8601_duration)
        })
        .or(html_data.duration);
    let published = player_response
        .as_ref()
        .and_then(|value| {
            value
                .pointer("/microformat/playerMicroformatRenderer/publishDate")
                .and_then(Value::as_str)
        })
        .map(str::to_string)
        .or_else(|| find_meta_content(document, "itemprop", "datePublished"))
        .or(html_data.published);
    let thumbnail_url = player_response
        .as_ref()
        .and_then(extract_thumbnail_url)
        .or_else(|| find_link_href(document, "rel", "image_src"))
        .or_else(|| find_meta_content(document, "property", "og:image"))
        .or(html_data.thumbnail_url);
    let embed_url = player_response
        .as_ref()
        .and_then(|value| {
            value
                .pointer("/microformat/playerMicroformatRenderer/embed/iframeUrl")
                .and_then(Value::as_str)
        })
        .map(str::to_string)
        .or_else(|| find_meta_content(document, "property", "og:video:url"))
        .or_else(|| {
            player_response
                .as_ref()
                .and_then(|value| value.pointer("/videoDetails/videoId"))
                .and_then(Value::as_str)
                .map(|video_id| format!("https://www.youtube.com/embed/{video_id}"))
        });
    let canonical_url = find_link_href(document, "rel", "canonical")
        .or_else(|| find_meta_content(document, "property", "og:url"))
        .or(html_data.canonical_url)
        .or_else(|| Some(url.to_string()));

    Some(YouTubeWatchData {
        title,
        channel,
        subscribers: html_data.subscribers,
        description,
        view_count,
        duration,
        published,
        thumbnail_url,
        canonical_url,
        embed_url,
        comment_count: html_data.comment_count,
        related_videos: html_data.related_videos,
        featured_comments: html_data.featured_comments,
    })
}

fn build_youtube_watch_document(data: &YouTubeWatchData) -> Node {
    let mut left_column = vec![simple_text_element("h1", &data.title)];
    if let Some(thumbnail_url) = &data.thumbnail_url {
        left_column.push(Node::Element(Element {
            tag_name: "img".to_string(),
            attributes: BTreeMap::from([
                ("src".to_string(), thumbnail_url.clone()),
                ("data-scratch-src".to_string(), thumbnail_url.clone()),
                ("width".to_string(), "720".to_string()),
                ("alt".to_string(), data.title.clone()),
            ]),
            children: Vec::new(),
        }));
    }

    if let Some(channel) = data.channel.as_deref() {
        left_column.push(simple_text_element("h2", channel));
    }
    push_detail(&mut left_column, "Subscribers", data.subscribers.as_deref());
    push_detail(&mut left_column, "Views", data.view_count.as_deref());
    push_detail(&mut left_column, "Published", data.published.as_deref());
    push_detail(&mut left_column, "Length", data.duration.as_deref());
    push_detail(&mut left_column, "Comments", data.comment_count.as_deref());
    push_detail(&mut left_column, "Watch URL", data.canonical_url.as_deref());
    push_detail(&mut left_column, "Embed URL", data.embed_url.as_deref());
    left_column.push(hr_node());
    left_column.push(simple_text_element("h2", "Description"));

    let description = data
        .description
        .as_deref()
        .unwrap_or("No description was embedded in the page.");
    let mut pushed_description = false;
    for paragraph in description
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(12)
    {
        left_column.push(simple_text_element("p", paragraph));
        pushed_description = true;
    }
    if !pushed_description {
        left_column.push(simple_text_element("p", description.trim()));
    }
    if !data.featured_comments.is_empty() {
        left_column.push(hr_node());
        left_column.push(simple_text_element("h2", "Featured Comments"));
        for comment in data.featured_comments.iter().take(3) {
            left_column.push(simple_text_element("h3", &comment.author));
            if let Some(likes) = comment.likes.as_deref() {
                left_column.push(simple_text_element("p", &format!("Likes: {likes}")));
            }
            left_column.push(simple_text_element("p", &comment.body));
        }
    }

    let mut right_column = vec![simple_text_element("h2", "Up Next")];
    if data.related_videos.is_empty() {
        right_column.push(simple_text_element(
            "p",
            "Related videos were not embedded in this response.",
        ));
    } else {
        for related in data.related_videos.iter().take(8) {
            right_column.push(build_related_video_node(related));
        }
    }

    synthetic_document(
        &data.title,
        vec![Node::Element(Element {
            tag_name: "table".to_string(),
            attributes: BTreeMap::from([
                ("width".to_string(), "100%".to_string()),
                ("border".to_string(), "0".to_string()),
                ("cellpadding".to_string(), "0".to_string()),
                ("cellspacing".to_string(), "16".to_string()),
            ]),
            children: vec![Node::Element(Element {
                tag_name: "tr".to_string(),
                attributes: BTreeMap::new(),
                children: vec![
                    Node::Element(Element {
                        tag_name: "td".to_string(),
                        attributes: BTreeMap::from([
                            ("width".to_string(), "70%".to_string()),
                            ("valign".to_string(), "top".to_string()),
                        ]),
                        children: left_column,
                    }),
                    Node::Element(Element {
                        tag_name: "td".to_string(),
                        attributes: BTreeMap::from([
                            ("width".to_string(), "30%".to_string()),
                            ("valign".to_string(), "top".to_string()),
                        ]),
                        children: right_column,
                    }),
                ],
            })],
        })],
    )
}

fn build_youtube_generic_document(document: &Node, html: &str, url: &Url) -> Node {
    let title = document_title(document).unwrap_or_else(|| "YouTube".to_string());
    let description = find_meta_content(document, "name", "description")
        .or_else(|| find_meta_content(document, "property", "og:description"))
        .unwrap_or_else(|| {
            "This YouTube page relies on a large app shell. Scratch Browser currently renders specific watch URLs more accurately than the full home feed.".to_string()
        });

    let mut body_children = vec![
        simple_text_element("h1", &title),
        simple_text_element("p", &description),
        simple_text_element(
            "p",
            "Tip: open a specific https://www.youtube.com/watch?v=... URL for a richer video summary.",
        ),
    ];

    let watch_links = extract_html_attribute_values(html, "href=\"/watch?v=", '"', 10)
        .into_iter()
        .map(|path| {
            if path.starts_with("http://") || path.starts_with("https://") {
                path
            } else {
                format!("https://www.youtube.com{path}")
            }
        })
        .collect::<Vec<_>>();

    if !watch_links.is_empty() {
        body_children.push(hr_node());
        body_children.push(simple_text_element("h2", "Watch Links"));
        let items = watch_links
            .into_iter()
            .map(|href| {
                Node::Element(Element {
                    tag_name: "li".to_string(),
                    attributes: BTreeMap::new(),
                    children: vec![Node::Text(href)],
                })
            })
            .collect();
        body_children.push(Node::Element(Element {
            tag_name: "ul".to_string(),
            attributes: BTreeMap::new(),
            children: items,
        }));
    }

    body_children.push(hr_node());
    body_children.push(simple_text_element("p", &format!("URL: {}", url)));

    synthetic_document(&title, body_children)
}

fn build_youtube_generic_document_from_html(html: &str, url: &Url) -> Node {
    let title = extract_html_tag_text(html, "title").unwrap_or_else(|| "YouTube".to_string());
    let description = extract_meta_content_from_html(html, "name", "description")
        .or_else(|| extract_meta_content_from_html(html, "property", "og:description"))
        .unwrap_or_else(|| {
            "This YouTube page relies on a large app shell. Scratch Browser currently renders specific watch URLs more accurately than the full home feed.".to_string()
        });

    let mut body_children = vec![
        simple_text_element("h1", &title),
        simple_text_element("p", &description),
        simple_text_element(
            "p",
            "Tip: open a specific https://www.youtube.com/watch?v=... URL for a richer video summary.",
        ),
    ];

    let watch_links = extract_html_attribute_values(html, "href=\"/watch?v=", '"', 10)
        .into_iter()
        .map(|path| format!("https://www.youtube.com{path}"))
        .collect::<Vec<_>>();
    if !watch_links.is_empty() {
        body_children.push(hr_node());
        body_children.push(simple_text_element("h2", "Watch Links"));
        body_children.push(Node::Element(Element {
            tag_name: "ul".to_string(),
            attributes: BTreeMap::new(),
            children: watch_links
                .into_iter()
                .map(|href| {
                    Node::Element(Element {
                        tag_name: "li".to_string(),
                        attributes: BTreeMap::new(),
                        children: vec![Node::Text(href)],
                    })
                })
                .collect(),
        }));
    }

    body_children.push(hr_node());
    body_children.push(simple_text_element("p", &format!("URL: {}", url)));

    synthetic_document(&title, body_children)
}

fn build_google_document_from_html(html: &str, url: &Url) -> Node {
    let title = extract_html_tag_text(html, "title").unwrap_or_else(|| "Google".to_string());
    let description = extract_meta_content_from_html(html, "name", "description")
        .or_else(|| extract_meta_content_from_html(html, "property", "og:description"))
        .unwrap_or_else(|| {
            "This Google page uses a large interactive shell. Scratch Browser keeps it lightweight instead of trying to execute the full app.".to_string()
        });

    synthetic_document(
        &title,
        vec![
            simple_text_element("h1", &title),
            simple_text_element("p", &description),
            simple_text_element(
                "p",
                "Tip: direct content URLs usually render better than large search or portal shells.",
            ),
            hr_node(),
            simple_text_element("p", &format!("URL: {}", url)),
        ],
    )
}

fn build_related_video_node(video: &YouTubeRelatedVideo) -> Node {
    let mut detail_children = Vec::new();
    detail_children.push(simple_text_element("h3", &video.title));
    push_detail(&mut detail_children, "Channel", video.channel.as_deref());
    push_detail(&mut detail_children, "Views", video.views.as_deref());
    push_detail(&mut detail_children, "Published", video.published.as_deref());
    push_detail(&mut detail_children, "Length", video.duration.as_deref());
    push_detail(&mut detail_children, "URL", video.url.as_deref());

    let mut row_children = Vec::new();
    if let Some(thumbnail_url) = &video.thumbnail_url {
        row_children.push(Node::Element(Element {
            tag_name: "td".to_string(),
            attributes: BTreeMap::from([
                ("width".to_string(), "180".to_string()),
                ("valign".to_string(), "top".to_string()),
            ]),
            children: vec![Node::Element(Element {
                tag_name: "img".to_string(),
                attributes: BTreeMap::from([
                    ("src".to_string(), thumbnail_url.clone()),
                    ("data-scratch-src".to_string(), thumbnail_url.clone()),
                    ("width".to_string(), "168".to_string()),
                    ("alt".to_string(), video.title.clone()),
                ]),
                children: Vec::new(),
            })],
        }));
    }
    row_children.push(Node::Element(Element {
        tag_name: "td".to_string(),
        attributes: BTreeMap::from([("valign".to_string(), "top".to_string())]),
        children: detail_children,
    }));

    Node::Element(Element {
        tag_name: "div".to_string(),
        attributes: BTreeMap::new(),
        children: vec![
            Node::Element(Element {
                tag_name: "table".to_string(),
                attributes: BTreeMap::from([
                    ("width".to_string(), "100%".to_string()),
                    ("border".to_string(), "0".to_string()),
                    ("cellpadding".to_string(), "0".to_string()),
                    ("cellspacing".to_string(), "8".to_string()),
                ]),
                children: vec![Node::Element(Element {
                    tag_name: "tr".to_string(),
                    attributes: BTreeMap::new(),
                    children: row_children,
                })],
            }),
            hr_node(),
        ],
    })
}

fn simple_text_element(tag_name: &str, text: &str) -> Node {
    Node::Element(Element {
        tag_name: tag_name.to_string(),
        attributes: BTreeMap::new(),
        children: vec![Node::Text(text.to_string())],
    })
}

fn push_detail(output: &mut Vec<Node>, label: &str, value: Option<&str>) {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    output.push(simple_text_element("p", &format!("{label}: {value}")));
}

fn extract_assigned_json_object(html: &str, markers: &[&str]) -> Option<String> {
    for marker in markers {
        let Some(marker_index) = html.find(marker) else {
            continue;
        };
        let after_marker = marker_index + marker.len();
        let Some(open_offset) = html[after_marker..].find('{') else {
            continue;
        };
        let open_index = after_marker + open_offset;
        if let Some(close_index) = find_matching_json_brace(html, open_index) {
            return Some(html[open_index..=close_index].to_string());
        }
    }

    None
}

fn extract_html_tag_text(html: &str, tag_name: &str) -> Option<String> {
    let open_tag = format!("<{tag_name}>");
    let close_tag = format!("</{tag_name}>");
    let start = html.find(&open_tag)?;
    let content_start = start + open_tag.len();
    let end_offset = html[content_start..].find(&close_tag)?;
    let text = &html[content_start..content_start + end_offset];
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn extract_meta_content_from_html(html: &str, attribute: &str, expected: &str) -> Option<String> {
    let mut search_start = 0;
    let attribute_marker = format!("{attribute}=\"{expected}\"");
    while let Some(meta_offset) = html[search_start..].find("<meta") {
        let tag_start = search_start + meta_offset;
        let tag_end = tag_start + html[tag_start..].find('>')?;
        let tag = &html[tag_start..=tag_end];
        if tag.contains(&attribute_marker)
            && let Some(content_offset) = tag.find("content=\"")
        {
            let value_start = content_offset + "content=\"".len();
            let value_end = tag[value_start..].find('"')?;
            let content = &tag[value_start..value_start + value_end];
            if !content.trim().is_empty() {
                return Some(content.to_string());
            }
        }
        search_start = tag_end + 1;
    }

    None
}

fn extract_link_href_from_html(html: &str, attribute: &str, expected: &str) -> Option<String> {
    let mut search_start = 0;
    let attribute_marker = format!("{attribute}=\"{expected}\"");
    while let Some(link_offset) = html[search_start..].find("<link") {
        let tag_start = search_start + link_offset;
        let tag_end = tag_start + html[tag_start..].find('>')?;
        let tag = &html[tag_start..=tag_end];
        if tag.contains(&attribute_marker)
            && let Some(href_offset) = tag.find("href=\"")
        {
            let value_start = href_offset + "href=\"".len();
            let value_end = tag[value_start..].find('"')?;
            let href = &tag[value_start..value_start + value_end];
            if !href.trim().is_empty() {
                return Some(href.to_string());
            }
        }
        search_start = tag_end + 1;
    }

    None
}

fn extract_html_attribute_values(
    html: &str,
    marker: &str,
    closing_quote: char,
    limit: usize,
) -> Vec<String> {
    let mut values = Vec::new();
    let mut search_start = 0;

    while values.len() < limit {
        let Some(found) = html[search_start..].find(marker) else {
            break;
        };
        let value_start = search_start + found + "href=\"".len();
        let Some(end_offset) = html[value_start..].find(closing_quote) else {
            break;
        };
        let value = &html[value_start..value_start + end_offset];
        if !values.iter().any(|known| known == value) {
            values.push(value.to_string());
        }
        search_start = value_start + end_offset + 1;
    }

    values
}

fn find_matching_json_brace(input: &str, open_index: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut depth = 0_u32;
    let mut in_string = false;
    let mut escaped = false;

    for (index, byte) in bytes.iter().copied().enumerate().skip(open_index) {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match byte {
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match byte {
            b'"' => in_string = true,
            b'{' => depth = depth.saturating_add(1),
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }

    None
}

fn extract_thumbnail_url(value: &Value) -> Option<String> {
    value
        .pointer("/videoDetails/thumbnail/thumbnails")
        .and_then(Value::as_array)
        .and_then(|items| items.iter().rev().find_map(|item| item.get("url")))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn extract_owner_renderer(initial_data: &Value) -> Option<&Value> {
    initial_data
        .pointer("/contents/twoColumnWatchNextResults/results/results/contents")
        .and_then(Value::as_array)
        .and_then(|items| {
            items.iter().find_map(|item| {
                item.get("videoSecondaryInfoRenderer")
                    .and_then(|renderer| renderer.pointer("/owner/videoOwnerRenderer"))
            })
        })
}

fn extract_comments_panel_header(initial_data: &Value) -> Option<&Value> {
    initial_data
        .get("engagementPanels")
        .and_then(Value::as_array)
        .and_then(|panels| {
            panels.iter().find_map(|panel| {
                let section = panel.get("engagementPanelSectionListRenderer")?;
                let identifier = section.get("panelIdentifier")?.as_str()?;
                (identifier == "engagement-panel-comments-section")
                    .then(|| section.get("header"))
                    .flatten()
                    .and_then(|header| header.get("engagementPanelTitleHeaderRenderer"))
            })
        })
}

fn extract_related_videos(initial_data: &Value) -> Vec<YouTubeRelatedVideo> {
    let contents = initial_data
        .pointer("/contents/twoColumnWatchNextResults/secondaryResults/secondaryResults/results")
        .and_then(Value::as_array)
        .and_then(|results| {
            results.iter().find_map(|entry| {
                entry.get("itemSectionRenderer")
                    .and_then(|renderer| renderer.get("contents"))
                    .and_then(Value::as_array)
            })
        });

    let Some(contents) = contents else {
        return Vec::new();
    };

    contents
        .iter()
        .filter_map(|entry| {
            entry
                .get("lockupViewModel")
                .and_then(parse_related_lockup_video)
                .or_else(|| {
                    entry
                        .get("compactVideoRenderer")
                        .and_then(parse_related_compact_video)
                })
        })
        .collect()
}

fn parse_related_lockup_video(value: &Value) -> Option<YouTubeRelatedVideo> {
    let title = value.pointer("/metadata/lockupMetadataViewModel/title/content")?;
    let title = title.as_str()?.to_string();
    let rows = value
        .pointer("/metadata/lockupMetadataViewModel/metadata/contentMetadataViewModel/metadataRows")
        .and_then(Value::as_array);
    let channel = rows
        .and_then(|items| items.first())
        .and_then(|row| row.pointer("/metadataParts/0/text/content"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let views = rows
        .and_then(|items| items.get(1))
        .and_then(|row| row.pointer("/metadataParts/0/text/content"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let published = rows
        .and_then(|items| items.get(1))
        .and_then(|row| row.pointer("/metadataParts/1/text/content"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let duration = value
        .pointer("/contentImage/thumbnailViewModel/overlays")
        .and_then(Value::as_array)
        .and_then(|overlays| {
            overlays.iter().find_map(|overlay| {
                overlay
                    .pointer("/thumbnailBottomOverlayViewModel/badges/0/thumbnailBadgeViewModel/text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
        });
    let thumbnail_url = value
        .pointer("/contentImage/thumbnailViewModel/image/sources")
        .and_then(Value::as_array)
        .and_then(|sources| sources.last())
        .and_then(|source| source.get("url"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let url = value
        .pointer("/rendererContext/commandContext/onTap/innertubeCommand/commandMetadata/webCommandMetadata/url")
        .and_then(Value::as_str)
        .map(|path| format!("https://www.youtube.com{path}"));

    Some(YouTubeRelatedVideo {
        title,
        channel,
        views,
        published,
        duration,
        url,
        thumbnail_url,
    })
}

fn parse_related_compact_video(value: &Value) -> Option<YouTubeRelatedVideo> {
    let title = json_text(value.get("title")?)?;
    let channel = json_text(value.get("shortBylineText")?);
    let views = json_text(value.get("viewCountText")?);
    let published = json_text(value.get("publishedTimeText")?);
    let duration = value
        .pointer("/lengthText/simpleText")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| json_text(value.get("lengthText")?));
    let thumbnail_url = value
        .pointer("/thumbnail/thumbnails")
        .and_then(Value::as_array)
        .and_then(|sources| sources.last())
        .and_then(|source| source.get("url"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let url = value
        .pointer("/navigationEndpoint/commandMetadata/webCommandMetadata/url")
        .and_then(Value::as_str)
        .map(|path| format!("https://www.youtube.com{path}"));

    Some(YouTubeRelatedVideo {
        title,
        channel,
        views,
        published,
        duration,
        url,
        thumbnail_url,
    })
}

fn extract_first_ld_json_video_object(html: &str) -> Option<Value> {
    let mut search_start = 0;
    while let Some(script_offset) = html[search_start..].find("<script") {
        let tag_start = search_start + script_offset;
        let tag_end = tag_start + html[tag_start..].find('>')?;
        let open_tag = &html[tag_start..=tag_end];
        if open_tag.contains("application/ld+json") {
            let close_offset = html[tag_end + 1..].find("</script>")?;
            let json_text = &html[tag_end + 1..tag_end + 1 + close_offset];
            if let Ok(value) = serde_json::from_str::<Value>(json_text)
                && value
                    .get("@type")
                    .and_then(Value::as_str)
                    .map(|kind| kind == "VideoObject")
                    .unwrap_or(false)
            {
                return Some(value);
            }
        }
        search_start = tag_end + 1;
    }

    None
}

fn extract_featured_comments_from_ld_json(value: &Value) -> Vec<YouTubeCommentPreview> {
    let comments = value.get("comment");
    let Some(items) = comments else {
        return Vec::new();
    };

    let array = match items {
        Value::Array(array) => array,
        single => return extract_featured_comments_from_ld_json(&Value::Array(vec![single.clone()])),
    };

    array
        .iter()
        .filter_map(|item| {
            let author = item
                .pointer("/author/name")
                .and_then(Value::as_str)
                .or_else(|| item.pointer("/author/alternateName").and_then(Value::as_str))?;
            let body = item.get("text").and_then(Value::as_str)?;
            Some(YouTubeCommentPreview {
                author: author.to_string(),
                body: body.to_string(),
                likes: item
                    .get("upvoteCount")
                    .and_then(|value| value.as_i64().map(|number| number.to_string()).or_else(|| value.as_str().map(str::to_string))),
            })
        })
        .collect()
}

fn json_text(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    if let Some(text) = value.get("simpleText").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(text) = value.get("content").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    value
        .get("runs")
        .and_then(Value::as_array)
        .map(|runs| {
            runs.iter()
                .filter_map(|run| run.get("text").and_then(Value::as_str))
                .collect::<String>()
        })
        .filter(|text| !text.trim().is_empty())
}

fn find_meta_content(node: &Node, attribute: &str, expected: &str) -> Option<String> {
    match node {
        Node::Text(_) => None,
        Node::Element(element) => {
            if element.tag_name == "meta"
                && element
                    .attribute(attribute)
                    .map(|value| value.eq_ignore_ascii_case(expected))
                    .unwrap_or(false)
                && let Some(content) = element.attribute("content")
                && !content.trim().is_empty()
            {
                return Some(content.to_string());
            }

            for child in &element.children {
                if let Some(found) = find_meta_content(child, attribute, expected) {
                    return Some(found);
                }
            }

            None
        }
    }
}

fn find_link_href(node: &Node, attribute: &str, expected: &str) -> Option<String> {
    match node {
        Node::Text(_) => None,
        Node::Element(element) => {
            if element.tag_name == "link"
                && element
                    .attribute(attribute)
                    .map(|value| value.eq_ignore_ascii_case(expected))
                    .unwrap_or(false)
                && let Some(href) = element.attribute("href")
                && !href.trim().is_empty()
            {
                return Some(href.to_string());
            }

            for child in &element.children {
                if let Some(found) = find_link_href(child, attribute, expected) {
                    return Some(found);
                }
            }

            None
        }
    }
}

fn find_link_content(node: &Node, attribute: &str, expected: &str) -> Option<String> {
    match node {
        Node::Text(_) => None,
        Node::Element(element) => {
            if element.tag_name == "link"
                && element
                    .attribute(attribute)
                    .map(|value| value.eq_ignore_ascii_case(expected))
                    .unwrap_or(false)
                && let Some(content) = element.attribute("content")
                && !content.trim().is_empty()
            {
                return Some(content.to_string());
            }

            for child in &element.children {
                if let Some(found) = find_link_content(child, attribute, expected) {
                    return Some(found);
                }
            }

            None
        }
    }
}

fn find_interaction_count(node: &Node, expected_type_fragment: &str) -> Option<String> {
    match node {
        Node::Text(_) => None,
        Node::Element(element) => {
            if element.tag_name == "div"
                && element
                    .attribute("itemprop")
                    .map(|value| value.eq_ignore_ascii_case("interactionStatistic"))
                    .unwrap_or(false)
            {
                let mut interaction_type = None;
                let mut user_count = None;
                for child in &element.children {
                    if let Node::Element(meta) = child
                        && meta.tag_name == "meta"
                    {
                        match meta.attribute("itemprop") {
                            Some("interactionType") => {
                                interaction_type = meta.attribute("content");
                            }
                            Some("userInteractionCount") => {
                                user_count = meta.attribute("content");
                            }
                            _ => {}
                        }
                    }
                }

                if interaction_type
                    .map(|value| value.contains(expected_type_fragment))
                    .unwrap_or(false)
                    && let Some(count) = user_count
                {
                    return Some(format_numeric_count(count));
                }
            }

            for child in &element.children {
                if let Some(found) = find_interaction_count(child, expected_type_fragment) {
                    return Some(found);
                }
            }

            None
        }
    }
}

fn format_numeric_count(raw: &str) -> String {
    let digits: String = raw
        .chars()
        .filter(|character| character.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return raw.to_string();
    }

    let mut grouped = String::new();
    for (index, character) in digits.chars().rev().enumerate() {
        if index != 0 && index % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(character);
    }

    grouped.chars().rev().collect()
}

fn format_duration_seconds(raw: &str) -> Option<String> {
    let total_seconds = raw.parse::<u64>().ok()?;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    Some(if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    })
}

fn parse_iso8601_duration(raw: String) -> Option<String> {
    if !raw.starts_with("PT") {
        return None;
    }

    let mut remaining = raw.trim_start_matches("PT");
    let mut hours = 0_u64;
    let mut minutes = 0_u64;
    let mut seconds = 0_u64;

    if let Some((value, rest)) = remaining.split_once('H') {
        hours = value.parse().ok()?;
        remaining = rest;
    }
    if let Some((value, rest)) = remaining.split_once('M') {
        minutes = value.parse().ok()?;
        remaining = rest;
    }
    if let Some((value, _)) = remaining.split_once('S') {
        seconds = value.parse().ok()?;
    }

    Some(if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    })
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
        BrowserPage, build_site_specific_document, collect_frame_specs, collect_stylesheet,
        document_title, extract_body_children, synthetic_document,
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

    #[test]
    fn rewrites_youtube_watch_pages_into_visible_summary() {
        let html = r#"
            <html>
              <head>
                <title>Video - YouTube</title>
                <meta name="description" content="fallback description">
                <meta property="og:image" content="https://i.ytimg.com/vi/demo/hqdefault.jpg">
                <link rel="canonical" href="https://www.youtube.com/watch?v=demo123">
              </head>
              <body>
                <script>
                  var ytInitialPlayerResponse = {
                    "videoDetails": {
                      "title": "Demo Video",
                      "author": "Demo Channel",
                      "viewCount": "1234567",
                      "lengthSeconds": "214",
                      "shortDescription": "Line one\nLine two",
                      "thumbnail": {
                        "thumbnails": [
                          {"url": "https://i.ytimg.com/vi/demo/default.jpg"},
                          {"url": "https://i.ytimg.com/vi/demo/maxresdefault.jpg"}
                        ]
                      },
                      "videoId": "demo123"
                    },
                    "microformat": {
                      "playerMicroformatRenderer": {
                        "publishDate": "2026-05-13",
                        "embed": {
                          "iframeUrl": "https://www.youtube.com/embed/demo123"
                        }
                      }
                    }
                  };
                </script>
              </body>
            </html>
        "#;
        let document = parse_document(html);

        let rewritten = build_site_specific_document(
            &document,
            html,
            &Url::parse("https://www.youtube.com/watch?v=demo123").unwrap(),
        )
        .expect("youtube watch pages should be rewritten");
        let rendered = crate::render::render_document(&rewritten);

        assert!(rendered.contains("Demo Video"));
        assert!(rendered.contains("Demo Channel"));
        assert!(rendered.contains("1,234,567"));
        assert!(rendered.contains("3:34"));
        assert!(rendered.contains("2026-05-13"));
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
