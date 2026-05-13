use std::collections::BTreeMap;

use encoding_rs::Encoding;

use crate::css::{StyledNode, Stylesheet, build_styled_tree, parse_stylesheet};
use crate::error::Result;
use crate::html::{Element, Node, parse_document};
use crate::http::{HttpResponse, fetch};
use crate::render::render_document;
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
    pub rendered: String,
}

impl BrowserPage {
    pub fn status_text(&self) -> String {
        format!("{} {}", self.status_code, self.reason_phrase)
            .trim()
            .to_string()
    }

    pub fn body_text(&self) -> &str {
        let trimmed = self.rendered.trim();
        if trimmed.is_empty() {
            "[empty document]"
        } else {
            trimmed
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
    let source = load_document_source(url, 0)?;
    let stylesheet = collect_stylesheet(&source.document, &source.final_url);
    let rendered = render_document(&source.document);
    let styled_document = build_styled_tree(&source.document, &stylesheet);

    Ok(BrowserPage {
        url: source.final_url,
        status_code: source.status_code,
        reason_phrase: source.reason_phrase,
        content_type: source.content_type,
        title: source.title,
        styled_document,
        rendered,
    })
}

fn load_document_source(url: &Url, frame_depth: usize) -> Result<LoadedDocumentSource> {
    let response = fetch(url)?;
    let content_type = response.header("content-type").map(str::to_string);
    let text = decode_response_text(&response);
    let parsed_document = parse_document(&text);
    let original_title = document_title(&parsed_document);
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
    let frames = collect_frame_specs(document);
    if frames.is_empty() {
        return Ok(None);
    }

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

fn decode_response_text(response: &HttpResponse) -> String {
    let charset = response
        .header("content-type")
        .and_then(charset_from_content_type)
        .or_else(|| sniff_html_charset(&response.body));

    let Some(charset) = charset else {
        return String::from_utf8_lossy(&response.body).into_owned();
    };

    let Some(encoding) = Encoding::for_label(charset.as_bytes()) else {
        return String::from_utf8_lossy(&response.body).into_owned();
    };

    let (decoded, _, _) = encoding.decode(&response.body);
    decoded.into_owned()
}

fn charset_from_content_type(content_type: &str) -> Option<String> {
    content_type.split(';').find_map(|segment| {
        let (name, value) = segment.trim().split_once('=')?;
        if !name.trim().eq_ignore_ascii_case("charset") {
            return None;
        }

        let value = value.trim().trim_matches('"').trim_matches('\'');
        (!value.is_empty()).then(|| value.to_string())
    })
}

fn sniff_html_charset(body: &[u8]) -> Option<String> {
    let sample = body
        .iter()
        .take(4096)
        .map(|byte| if byte.is_ascii() { *byte as char } else { ' ' })
        .collect::<String>()
        .to_ascii_lowercase();

    if let Some(index) = sample.find("charset=") {
        let rest = &sample[index + "charset=".len()..];
        let charset = rest
            .trim_start_matches(['"', '\'', ' '])
            .chars()
            .take_while(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
            })
            .collect::<String>();
        if !charset.is_empty() {
            return Some(charset);
        }
    }

    None
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
        let css_text = decode_response_text(&response);
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
        BrowserPage, charset_from_content_type, collect_frame_specs, collect_stylesheet,
        decode_response_text, document_title, synthetic_document,
    };
    use crate::css::StyledNode;
    use crate::html::{Node, parse_document};
    use crate::http::HttpResponse;
    use crate::url::Url;
    use std::collections::HashMap;

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
                    font_size_px: 16,
                    font_family: crate::css::FontFamilyKind::Sans,
                    text_align: crate::css::TextAlign::Left,
                    font_weight: false,
                    underline: false,
                    white_space: crate::css::WhiteSpaceMode::Normal,
                },
            }),
            rendered: "   ".to_string(),
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
            rendered: "# Hello".to_string(),
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
    fn decodes_shift_jis_from_meta_charset() {
        let (encoded_title, _, _) = encoding_rs::SHIFT_JIS.encode("阿部寛");
        let mut body =
            b"<meta http-equiv=\"Content-Type\" content=\"text/html; charset=Shift_JIS\"><title>"
                .to_vec();
        body.extend_from_slice(&encoded_title);
        body.extend_from_slice(b"</title>");
        let response = HttpResponse {
            final_url: Url::parse("https://example.com").unwrap(),
            status_code: 200,
            reason_phrase: "OK".to_string(),
            headers: HashMap::new(),
            body,
        };

        let decoded = decode_response_text(&response);

        assert!(decoded.contains("阿部寛"));
    }

    #[test]
    fn extracts_charset_from_content_type() {
        assert_eq!(
            charset_from_content_type("text/html; charset=Shift_JIS"),
            Some("Shift_JIS".to_string())
        );
        assert_eq!(charset_from_content_type("text/html"), None);
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

    fn parse_styled_text(text: &str) -> StyledNode {
        crate::css::StyledNode::Text(crate::css::StyledText {
            text: text.to_string(),
            style: crate::css::ComputedStyle {
                display: crate::css::Display::Inline,
                color: crate::css::DEFAULT_TEXT_COLOR,
                background_color: None,
                margin: crate::css::EdgeSizes::default(),
                padding: crate::css::EdgeSizes::default(),
                font_size_px: 16,
                font_family: crate::css::FontFamilyKind::Sans,
                text_align: crate::css::TextAlign::Left,
                font_weight: false,
                underline: false,
                white_space: crate::css::WhiteSpaceMode::Normal,
            },
        })
    }
}
