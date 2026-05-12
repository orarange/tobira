use crate::css::{StyledNode, Stylesheet, build_styled_tree, parse_stylesheet};
use crate::error::Result;
use crate::html::{Node, parse_document};
use crate::http::fetch;
use crate::render::render_document;
use crate::url::Url;

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

pub fn load_page(url: &Url) -> Result<BrowserPage> {
    let response = fetch(url)?;
    let content_type = response.header("content-type").map(str::to_string);
    let document = parse_document(&String::from_utf8_lossy(&response.body));
    let stylesheet = collect_stylesheet(&document, &response.final_url);
    let styled_document = build_styled_tree(&document, &stylesheet);
    let rendered = render_document(&document);
    let title = document_title(&document)
        .or_else(|| first_heading(&document))
        .unwrap_or_else(|| "Scratch Browser".to_string());

    Ok(BrowserPage {
        url: response.final_url,
        status_code: response.status_code,
        reason_phrase: response.reason_phrase,
        content_type,
        title,
        styled_document,
        rendered,
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
        let css_text = String::from_utf8_lossy(&response.body);
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
    use super::{BrowserPage, collect_stylesheet, document_title};
    use crate::css::StyledNode;
    use crate::html::parse_document;
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
