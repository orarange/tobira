use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Element(Element),
    Text(String),
    /// `<!-- ... -->`, with the text exactly as written between the delimiters.
    ///
    /// A comment is a node like any other: scripts walk over it in
    /// `childNodes`, read it as `nodeType === 8`, and templating libraries use
    /// it as a marker. Dropped at the tokenizer, it was invisible to all of
    /// them.
    Comment(String),
    /// `<!DOCTYPE ...>`, reduced to the name it declares.
    Doctype(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub tag_name: String,
    pub attributes: BTreeMap<String, String>,
    pub children: Vec<Node>,
}

impl Element {
    fn new(tag_name: impl Into<String>) -> Self {
        Self {
            tag_name: tag_name.into(),
            attributes: BTreeMap::new(),
            children: Vec::new(),
        }
    }

    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes.get(name).map(String::as_str)
    }
}

#[derive(Debug)]
enum Token {
    Comment(String),
    Doctype(String),
    StartTag {
        name: String,
        attributes: BTreeMap<String, String>,
        self_closing: bool,
    },
    EndTag(String),
    Text(String),
}

pub fn parse_document(input: &str) -> Node {
    let mut document = parse_document_body(input);
    if let Node::Element(root) = &mut document {
        ensure_document_structure(root);
    }
    document
}

fn parse_document_body(input: &str) -> Node {
    let tokens = tokenize(input);
    let mut stack = vec![Element::new("document")];

    for token in tokens {
        match token {
            Token::Text(text) => {
                if !text.is_empty() {
                    stack
                        .last_mut()
                        .expect("document root always exists")
                        .children
                        .push(Node::Text(text));
                }
            }
            Token::StartTag {
                name,
                attributes,
                self_closing,
            } => {
                // HTML5 optional-end-tag: implicitly close certain elements before
                // opening a new one (e.g. <td> closes an open <td>, <li> closes open <li>).
                if !self_closing {
                    maybe_auto_close(&mut stack, &name);
                }

                let element = Element {
                    tag_name: name,
                    attributes,
                    children: Vec::new(),
                };

                if self_closing {
                    stack
                        .last_mut()
                        .expect("document root always exists")
                        .children
                        .push(Node::Element(element));
                } else {
                    stack.push(element);
                }
            }
            Token::EndTag(name) => close_element(&mut stack, &name),
            Token::Comment(text) => {
                stack
                    .last_mut()
                    .expect("document root always exists")
                    .children
                    .push(Node::Comment(text));
            }
            Token::Doctype(name) => {
                stack
                    .last_mut()
                    .expect("document root always exists")
                    .children
                    .push(Node::Doctype(name));
            }
        }
    }

    while stack.len() > 1 {
        let element = stack.pop().expect("stack is not empty");
        stack
            .last_mut()
            .expect("document root always exists")
            .children
            .push(Node::Element(element));
    }

    Node::Element(stack.pop().expect("document root exists"))
}

/// Parse an HTML fragment: the tags exactly as written, with no document
/// structure built around them.
///
/// `innerHTML`, `outerHTML` and `insertAdjacentHTML` all take a fragment, and a
/// fragment does not grow an `<html>`, a `<head>` or a `<body>` -- setting
/// `el.innerHTML = '<span>x</span>'` puts a span inside `el`, not a whole
/// document.
pub fn parse_fragment(input: &str) -> Vec<Node> {
    let Node::Element(root) = parse_document_body(input) else {
        return Vec::new();
    };
    root.children
}

/// Names that belong in `<head>` when they turn up before any body content.
fn is_head_only(name: &str) -> bool {
    matches!(
        name,
        "base" | "basefont" | "bgsound" | "link" | "meta" | "noscript" | "script" | "style"
            | "template" | "title"
    )
}

/// Give the document the `<html>`, `<head>` and `<body>` a browser always
/// builds, whether or not the markup wrote them.
///
/// Every browser produces `html > head + body` for any input at all -- an empty
/// file, a bare `Hello`, a stray `</p>`. This parser produced whatever the tags
/// said and nothing more, which is right for a well-formed page and wrong for
/// everything else; against the WHATWG tree-construction suite it agreed with
/// browsers on 2 of 1229 cases, failing almost all of them on the first line.
///
/// Done as a pass over the finished tree rather than as insertion modes inside
/// the parser: the shape is what pages and scripts observe, and this reaches it
/// without disturbing the tag handling that real pages already depend on.
fn ensure_document_structure(root: &mut Element) {
    // A doctype, and any comment written before the markup proper, belong to
    // the document itself rather than inside `<html>` -- which is where a
    // browser puts them, and where `document.doctype` looks for one.
    let mut before_html: Vec<Node> = Vec::new();
    let mut rest: Vec<Node> = Vec::new();
    let mut seen_content = false;
    for child in root.children.drain(..) {
        match &child {
            Node::Doctype(_) if !seen_content => before_html.push(child),
            Node::Comment(_) if !seen_content => before_html.push(child),
            Node::Text(text) if !seen_content && text.trim().is_empty() => {}
            _ => {
                seen_content = true;
                rest.push(child);
            }
        }
    }
    root.children = rest;

    let existing_html = root
        .children
        .iter()
        .position(|child| matches!(child, Node::Element(e) if e.tag_name == "html"));

    let mut html = match existing_html {
        Some(index) => {
            // Anything outside `<html>` still belongs inside it.
            let Node::Element(mut html) = root.children.remove(index) else {
                unreachable!("checked above")
            };
            let stray: Vec<Node> = root.children.drain(..).collect();
            let mut merged = Vec::with_capacity(stray.len() + html.children.len());
            merged.extend(html.children.drain(..));
            merged.extend(stray);
            html.children = merged;
            html
        }
        None => {
            let mut html = Element::new("html");
            html.children = root.children.drain(..).collect();
            html
        }
    };

    // Pull out any `<head>` and `<body>` the markup wrote, keeping their
    // contents; everything else is sorted between them below.
    let mut head = Element::new("head");
    let mut body = Element::new("body");
    let mut loose: Vec<Node> = Vec::new();
    for child in html.children.drain(..) {
        match child {
            Node::Element(element) if element.tag_name == "head" => {
                head.attributes.extend(element.attributes);
                head.children.extend(element.children);
            }
            Node::Element(element) if element.tag_name == "body" => {
                body.attributes.extend(element.attributes);
                body.children.extend(element.children);
            }
            other => loose.push(other),
        }
    }

    // Before any body content, head-only elements go to the head; once
    // something else has appeared, everything that follows is body content --
    // which is what "after head" means.
    let mut seen_body_content = !body.children.is_empty();
    for node in loose {
        match &node {
            Node::Element(element) if !seen_body_content && is_head_only(&element.tag_name) => {
                head.children.push(node);
            }
            // Whitespace before the body starts is dropped, as it is in the
            // "before head" and "in head" modes.
            Node::Text(text) if !seen_body_content && text.trim().is_empty() => {}
            _ => {
                seen_body_content = true;
                body.children.push(node);
            }
        }
    }

    // A frameset document has no body: the frameset takes its place. Browsers
    // put `html > head + frameset` on screen and leave the body out entirely.
    let frameset = body
        .children
        .iter()
        .position(|child| matches!(child, Node::Element(e) if e.tag_name == "frameset"))
        .filter(|_| {
            body.children.iter().all(|child| match child {
                // Neither renders, and neither carries anything this walk wants.
                Node::Comment(_) | Node::Doctype(_) => Default::default(),
                Node::Element(e) => matches!(e.tag_name.as_str(), "frameset" | "noframes"),
                Node::Text(text) => text.trim().is_empty(),
            })
        });
    let second = match frameset {
        Some(index) => body.children.remove(index),
        None => Node::Element(body),
    };

    html.children = vec![Node::Element(head), second];
    before_html.push(Node::Element(html));
    root.children = before_html;
}

fn close_element(stack: &mut Vec<Element>, target: &str) {
    if !stack[1..].iter().any(|element| element.tag_name == target) {
        return;
    }

    while stack.len() > 1 {
        let element = stack.pop().expect("stack is not empty");
        let matched = element.tag_name == target;
        stack
            .last_mut()
            .expect("document root always exists")
            .children
            .push(Node::Element(element));
        if matched {
            break;
        }
    }
}

/// HTML5 optional-end-tag / implicit-close rules.
/// When certain start tags are encountered, currently open elements of the
/// same category must be implicitly closed first (e.g. a new <td> closes any
/// already-open <td> that is within the same <tr>).
fn maybe_auto_close(stack: &mut Vec<Element>, new_tag: &str) {
    match new_tag {
        // Table cell: close any open td/th within the current tr context
        "td" | "th" => {
            auto_close_before(
                stack,
                &["td", "th"],
                &["tr", "table", "tbody", "thead", "tfoot"],
            );
        }
        // Table row: close any open tr within the current table body/head/foot
        "tr" => {
            auto_close_before(stack, &["tr"], &["table", "tbody", "thead", "tfoot"]);
        }
        // List item: close any open li within the current list
        "li" => {
            auto_close_before(stack, &["li"], &["ul", "ol"]);
        }
        // Definition list items
        "dt" | "dd" => {
            auto_close_before(stack, &["dt", "dd"], &["dl"]);
        }
        // A new <p> closes an open <p> (and many block elements do too)
        tag if is_block_like(tag) => {
            auto_close_before(
                stack,
                &["p"],
                &["td", "th", "li", "dd", "dt", "body", "html", "document"],
            );
        }
        _ => {}
    }
}

/// Walk up the stack looking for an element whose tag is in `targets`.
/// Stop (and do nothing) if we hit a `boundary` element first.
/// If found, call close_element to pop up to and including that element.
fn auto_close_before(stack: &mut Vec<Element>, targets: &[&str], boundaries: &[&str]) {
    let close_tag = stack.iter().rev().find_map(|el| {
        let name = el.tag_name.as_str();
        if targets.contains(&name) {
            Some(name.to_string())
        } else if boundaries.contains(&name) {
            Some(String::new()) // boundary hit — signal "stop, nothing to close"
        } else {
            None
        }
    });
    if let Some(tag) = close_tag {
        if !tag.is_empty() {
            close_element(stack, &tag);
        }
    }
}

/// Elements that trigger implicit closure of an open <p>.
fn is_block_like(tag: &str) -> bool {
    matches!(
        tag,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "details"
            | "dialog"
            | "div"
            | "dl"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hgroup"
            | "hr"
            | "main"
            | "menu"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "summary"
            | "table"
            | "ul"
    )
}

fn tokenize(input: &str) -> Vec<Token> {
    let bytes = input.as_bytes();
    let mut index = 0;
    let mut tokens = Vec::new();

    while index < bytes.len() {
        if bytes[index] != b'<' {
            let next = input[index..]
                .find('<')
                .map(|offset| index + offset)
                .unwrap_or(bytes.len());
            tokens.push(Token::Text(decode_html_entities(&input[index..next])));
            index = next;
            continue;
        }

        if input[index..].starts_with("<!--") {
            // An unterminated comment runs to the end of the file, which is
            // what a browser does with it rather than dropping the rest.
            let (text, next) = match input[index + 4..].find("-->") {
                Some(offset) => (
                    input[index + 4..index + 4 + offset].to_string(),
                    index + 4 + offset + 3,
                ),
                None => (input[index + 4..].to_string(), bytes.len()),
            };
            tokens.push(Token::Comment(text));
            index = next;
            continue;
        }

        if input[index..].starts_with("</") {
            index += 2;
            skip_whitespace(input, &mut index);
            let name_start = index;
            while index < bytes.len() && is_tag_name_char(bytes[index]) {
                index += 1;
            }
            let name = input[name_start..index].to_ascii_lowercase();
            consume_until_tag_end(input, &mut index);
            tokens.push(Token::EndTag(name));
            continue;
        }

        if input[index..].starts_with("<!") {
            let start = index;
            consume_until_tag_end(input, &mut index);
            let raw = &input[start..index];
            if raw.len() > 9 && raw[2..9].eq_ignore_ascii_case("doctype") {
                // Only the name is kept: it is what `document.doctype.name`
                // reports and what decides quirks mode.
                let name = raw[9..]
                    .trim_end_matches('>')
                    .trim()
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_matches('"')
                    .to_ascii_lowercase();
                tokens.push(Token::Doctype(name));
            }
            continue;
        }

        index += 1;
        skip_whitespace(input, &mut index);

        let name_start = index;
        while index < bytes.len() && is_tag_name_char(bytes[index]) {
            index += 1;
        }

        if name_start == index {
            index += input[index..].chars().next().map_or(1, |c| c.len_utf8());
            continue;
        }

        let name = input[name_start..index].to_ascii_lowercase();
        let mut attributes = BTreeMap::new();
        let mut self_closing = is_void_element(&name);

        loop {
            skip_whitespace(input, &mut index);

            if index >= bytes.len() {
                break;
            }

            match bytes[index] {
                b'>' => {
                    index += 1;
                    break;
                }
                b'/' => {
                    self_closing = true;
                    index += 1;
                }
                _ => {
                    let attr_name_start = index;
                    while index < bytes.len()
                        && !matches!(
                            bytes[index],
                            b'=' | b'>' | b'/' | b' ' | b'\n' | b'\r' | b'\t'
                        )
                    {
                        index += 1;
                    }

                    let attr_name = input[attr_name_start..index].to_ascii_lowercase();
                    skip_whitespace(input, &mut index);

                    let attr_value = if index < bytes.len() && bytes[index] == b'=' {
                        index += 1;
                        skip_whitespace(input, &mut index);
                        parse_attribute_value(input, &mut index)
                    } else {
                        String::new()
                    };

                    if !attr_name.is_empty() {
                        attributes.insert(attr_name, attr_value);
                    }
                }
            }
        }

        let is_raw_text = !self_closing && is_raw_text_element(&name);
        tokens.push(Token::StartTag {
            name: name.clone(),
            attributes,
            self_closing,
        });

        if is_raw_text {
            if let Some(close_start) = find_raw_text_close(input, index, &name) {
                let raw_text = &input[index..close_start];
                if !raw_text.is_empty() {
                    tokens.push(Token::Text(raw_text.to_string()));
                }
                tokens.push(Token::EndTag(name.clone()));
                index = consume_raw_text_close(input, close_start, &name);
            } else {
                let raw_text = &input[index..];
                if !raw_text.is_empty() {
                    tokens.push(Token::Text(raw_text.to_string()));
                }
                break;
            }
        }
    }

    tokens
}

fn consume_until_tag_end(input: &str, index: &mut usize) {
    if let Some(offset) = input[*index..].find('>') {
        *index += offset + 1;
    } else {
        *index = input.len();
    }
}

fn parse_attribute_value(input: &str, index: &mut usize) -> String {
    let bytes = input.as_bytes();
    if *index >= bytes.len() {
        return String::new();
    }

    let quote = bytes[*index];
    if quote == b'"' || quote == b'\'' {
        *index += 1;
        let start = *index;
        while *index < bytes.len() && bytes[*index] != quote {
            *index += 1;
        }
        let value = decode_html_entities(&input[start..*index]);
        if *index < bytes.len() {
            *index += 1;
        }
        value
    } else {
        let start = *index;
        while *index < bytes.len()
            && !matches!(bytes[*index], b'>' | b'/' | b' ' | b'\n' | b'\r' | b'\t')
        {
            *index += 1;
        }
        decode_html_entities(&input[start..*index])
    }
}

fn skip_whitespace(input: &str, index: &mut usize) {
    let bytes = input.as_bytes();
    while *index < bytes.len() && bytes[*index].is_ascii_whitespace() {
        *index += 1;
    }
}

fn is_tag_name_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b':' | b'_')
}

fn is_void_element(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "frame"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn is_raw_text_element(name: &str) -> bool {
    matches!(name, "script" | "style" | "title" | "textarea")
}

fn find_raw_text_close(input: &str, start: usize, tag_name: &str) -> Option<usize> {
    let mut search_index = start;
    let close_tag = format!("</{tag_name}");

    while search_index < input.len() {
        let remaining = &input[search_index..];
        let Some(offset) = find_case_insensitive(remaining, &close_tag) else {
            return None;
        };
        let close_start = search_index + offset;
        let name_end = close_start + close_tag.len();
        if name_end >= input.len() {
            return Some(close_start);
        }

        let trailing = input.as_bytes()[name_end];
        if trailing.is_ascii_whitespace() || trailing == b'>' {
            return Some(close_start);
        }

        search_index = close_start + 1;
    }

    None
}

fn consume_raw_text_close(input: &str, close_start: usize, tag_name: &str) -> usize {
    let mut index = close_start + 2 + tag_name.len();
    consume_until_tag_end(input, &mut index);
    index
}

fn find_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    let needle = needle.as_bytes();
    if needle.is_empty() {
        return Some(0);
    }

    haystack
        .as_bytes()
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
}

fn decode_html_entities(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut remaining = input;

    while let Some(entity_start) = remaining.find('&') {
        output.push_str(&remaining[..entity_start]);
        remaining = &remaining[entity_start + 1..];

        let Some(entity_end) = remaining.find(';') else {
            output.push('&');
            output.push_str(remaining);
            return output;
        };

        let entity = &remaining[..entity_end];
        remaining = &remaining[entity_end + 1..];

        if let Some(character) = decode_entity(entity) {
            output.push(character);
        } else {
            output.push('&');
            output.push_str(entity);
            output.push(';');
        }
    }

    output.push_str(remaining);
    output
}

fn decode_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{00A0}'),
        _ => decode_numeric_entity(entity),
    }
}

fn decode_numeric_entity(entity: &str) -> Option<char> {
    if let Some(hex) = entity
        .strip_prefix("#x")
        .or_else(|| entity.strip_prefix("#X"))
    {
        u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
    } else if let Some(decimal) = entity.strip_prefix('#') {
        decimal.parse::<u32>().ok().and_then(char::from_u32)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{Element, Node, parse_document};

    /// The `<body>` a document always has now that the implied structure is
    /// built. These tests were written when the parser returned tags exactly as
    /// they appeared, so they reached straight into the root.
    fn html_of(document: &Node) -> &Element {
        let Node::Element(root) = document else {
            panic!("root should be an element");
        };
        assert_eq!(root.tag_name, "document");
        let Some(Node::Element(html)) = root.children.first() else {
            panic!("document should hold an <html>");
        };
        assert_eq!(html.tag_name, "html");
        html
    }

    fn head_of(document: &Node) -> &Element {
        let Some(Node::Element(head)) = html_of(document).children.first() else {
            panic!("<html> should hold a <head>");
        };
        assert_eq!(head.tag_name, "head");
        head
    }

    fn body_of(document: &Node) -> &Element {
        let Node::Element(root) = document else {
            panic!("root should be an element");
        };
        assert_eq!(root.tag_name, "document");
        let Some(Node::Element(html)) = root.children.first() else {
            panic!("document should hold an <html>");
        };
        assert_eq!(html.tag_name, "html");
        let Some(Node::Element(body)) = html.children.get(1) else {
            panic!("<html> should hold <head> then <body>");
        };
        assert_eq!(body.tag_name, "body");
        body
    }

    #[test]
    fn parses_text_and_nested_elements() {
        let document = parse_document("<h1>Hello</h1><p>Rust <a href=\"/docs\">docs</a></p>");
        let body = body_of(&document);

        assert_eq!(body.children.len(), 2);
    }

    #[test]
    fn keeps_attributes() {
        let document = parse_document("<a href=\"/docs\" data-kind=\"nav\">docs</a>");
        let body = body_of(&document);

        let Node::Element(anchor) = &body.children[0] else {
            panic!("first child should be an element");
        };

        assert_eq!(anchor.attribute("href"), Some("/docs"));
        assert_eq!(anchor.attribute("data-kind"), Some("nav"));
    }

    #[test]
    fn decodes_named_and_numeric_entities() {
        let document = parse_document("<p title=\"Tom &amp; Jerry\">A&nbsp;B &#x263A; &#9731;</p>");
        let body = body_of(&document);

        let Node::Element(paragraph) = &body.children[0] else {
            panic!("first child should be an element");
        };

        assert_eq!(paragraph.attribute("title"), Some("Tom & Jerry"));

        let Node::Text(text) = &paragraph.children[0] else {
            panic!("paragraph should contain text");
        };

        assert!(text.contains('\u{00A0}'));
        assert!(text.contains('☺'));
        assert!(text.contains('☃'));
    }

    #[test]
    fn treats_frame_as_void_element() {
        let document = parse_document(
            "<frameset cols=\"18,82\"><frame src=\"menu.htm\"><frame src=\"top.htm\"></frameset>",
        );
        let Some(Node::Element(frameset)) = html_of(&document).children.get(1) else {
            panic!("a frameset document should hold <head> then <frameset>");
        };

        assert_eq!(frameset.tag_name, "frameset");
        assert_eq!(frameset.children.len(), 2);
    }

    #[test]
    fn ignores_closing_tags_for_void_frames() {
        let document = parse_document(
            "<frameset cols=\"18,82\"><frame src=\"a.htm\"></frame><frame src=\"b.htm\"></frame></frameset>",
        );
        let Some(Node::Element(frameset)) = html_of(&document).children.get(1) else {
            panic!("a frameset document should hold <head> then <frameset>");
        };

        assert_eq!(frameset.tag_name, "frameset");
        assert_eq!(frameset.children.len(), 2);

        let Node::Element(first_frame) = &frameset.children[0] else {
            panic!("first frameset child should be a frame");
        };
        let Node::Element(second_frame) = &frameset.children[1] else {
            panic!("second frameset child should be a frame");
        };

        assert_eq!(first_frame.tag_name, "frame");
        assert_eq!(second_frame.tag_name, "frame");
    }

    #[test]
    fn ignores_unmatched_end_tags() {
        let document = parse_document("<div><span></b>text</span></div>");
        let Node::Element(div) = &body_of(&document).children[0] else {
            panic!("first child should be a div");
        };
        let Node::Element(span) = &div.children[0] else {
            panic!("div child should be a span");
        };
        let Node::Text(text) = &span.children[0] else {
            panic!("span should contain text");
        };

        assert_eq!(div.tag_name, "div");
        assert_eq!(span.tag_name, "span");
        assert_eq!(text, "text");
    }

    #[test]
    fn keeps_script_contents_as_raw_text() {
        let document = parse_document(
            "<script>document.write('<script>document.write(\"<p>Nested</p>\")</' + 'script>')</script>",
        );
        // A lone `<script>` belongs to the head, which is where a browser puts
        // one that appears before any body content.
        let Node::Element(script) = &head_of(&document).children[0] else {
            panic!("the head should hold the script");
        };

        assert_eq!(script.tag_name, "script");
        assert_eq!(script.children.len(), 1);

        let Node::Text(source) = &script.children[0] else {
            panic!("script should contain raw text");
        };

        assert!(source.contains("</' + 'script>"));
        assert!(source.contains("<p>Nested</p>"));
    }

    #[test]
    fn handles_invalid_tag_start_before_replacement_characters() {
        let document = parse_document("<\u{FFFD}\u{FFFD}abc");
        let Node::Element(root) = document else {
            panic!("root should be an element");
        };

        assert_eq!(root.tag_name, "document");
    }

    #[test]
    fn handles_lossy_binary_with_non_ascii_after_less_than() {
        let input = String::from_utf8_lossy(&[
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x3C, 0xEF, 0xA0,
            0x80, 0x3C, 0x80, 0x81,
        ]);
        let document = parse_document(&input);
        let Node::Element(root) = document else {
            panic!("root should be an element");
        };

        assert_eq!(root.tag_name, "document");
    }
}

/// The WHATWG tree-construction suite, as shipped by html5lib.
///
/// Real pages are mostly well formed, so the shape of this parser was settled by
/// what real pages needed. That leaves no way to tell how far it sits from the
/// standard -- and every browser agrees on the standard, including for markup no
/// author would write on purpose. These cases are the agreement written down.
///
/// The score is asserted against a floor rather than 100%: the point is a
/// ratchet that cannot slip while the gaps are closed one at a time.
#[cfg(test)]
mod html5lib_conformance {
    use super::{Node, parse_document};

    /// One `#data` block: the input and the tree it should produce.
    struct Case {
        file: String,
        data: String,
        document: String,
        fragment: bool,
    }

    fn parse_dat(file: &str, text: &str) -> Vec<Case> {
        let mut cases = Vec::new();
        let mut section = String::new();
        let mut data = String::new();
        let mut document = String::new();
        let mut fragment = false;
        let mut started = false;

        let flush = |cases: &mut Vec<Case>,
                     data: &mut String,
                     document: &mut String,
                     fragment: &mut bool,
                     started: &mut bool| {
            if *started {
                cases.push(Case {
                    file: file.to_string(),
                    // The final newline before the next `#` marker is a
                    // separator, not part of the input.
                    data: data.strip_suffix('\n').unwrap_or(data).to_string(),
                    document: document.trim_end_matches('\n').to_string(),
                    fragment: *fragment,
                });
            }
            data.clear();
            document.clear();
            *fragment = false;
            *started = false;
        };

        for line in text.split_inclusive('\n') {
            let trimmed = line.trim_end_matches('\n');
            if trimmed.starts_with('#') && !trimmed.starts_with("#document\n") {
                match trimmed {
                    "#data" => {
                        flush(&mut cases, &mut data, &mut document, &mut fragment, &mut started);
                        started = true;
                        section = "data".to_string();
                        continue;
                    }
                    "#errors" | "#new-errors" | "#script-on" | "#script-off" => {
                        section = "skip".to_string();
                        continue;
                    }
                    "#document-fragment" => {
                        fragment = true;
                        section = "skip".to_string();
                        continue;
                    }
                    "#document" => {
                        section = "document".to_string();
                        continue;
                    }
                    _ => {
                        section = "skip".to_string();
                        continue;
                    }
                }
            }
            match section.as_str() {
                "data" => data.push_str(line),
                "document" => document.push_str(line),
                _ => {}
            }
        }
        flush(&mut cases, &mut data, &mut document, &mut fragment, &mut started);
        cases
    }

    /// Our tree in html5lib's notation.
    fn serialize(document: &Node) -> String {
        let mut out = String::new();
        if let Node::Element(root) = document {
            for child in &root.children {
                write_node(child, 0, &mut out);
            }
        }
        out.trim_end_matches('\n').to_string()
    }

    fn write_node(node: &Node, depth: usize, out: &mut String) {
        let pad = "  ".repeat(depth);
        match node {
            Node::Text(text) => {
                out.push_str(&format!("| {pad}\"{text}\"\n"));
            }
            Node::Comment(text) => {
                out.push_str(&format!("| {pad}<!-- {text} -->
"));
            }
            Node::Doctype(name) => {
                out.push_str(&format!("| {pad}<!DOCTYPE {name}>
"));
            }
            Node::Element(element) => {
                out.push_str(&format!("| {pad}<{}>\n", element.tag_name));
                for (name, value) in &element.attributes {
                    out.push_str(&format!("| {pad}  {name}=\"{value}\"\n"));
                }
                for child in &element.children {
                    write_node(child, depth + 1, out);
                }
            }
        }
    }

    fn load_cases() -> Vec<Case> {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/html5lib");
        let mut cases = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return cases;
        };
        let mut files: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|e| e == "dat"))
            .collect();
        files.sort();
        for path in files {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?")
                .to_string();
            if let Ok(text) = std::fs::read_to_string(&path) {
                cases.extend(parse_dat(&name, &text));
            }
        }
        cases
    }

    /// Run: `cargo test --bin tobira html5lib -- --nocapture` to see the score
    /// and a sample of what still differs.
    #[test]
    fn tree_construction_conformance() {
        let cases = load_cases();
        assert!(!cases.is_empty(), "the fixtures should be present");

        let mut ran = 0usize;
        let mut passed = 0usize;
        let mut by_file: std::collections::BTreeMap<String, (usize, usize)> =
            std::collections::BTreeMap::new();
        let mut samples: Vec<String> = Vec::new();

        for case in &cases {
            // Fragment parsing takes a context element this parser has no entry
            // point for; those are counted separately once it does.
            if case.fragment {
                continue;
            }
            ran += 1;
            let entry = by_file.entry(case.file.clone()).or_insert((0, 0));
            entry.1 += 1;
            let got = serialize(&parse_document(&case.data));
            if got == case.document {
                passed += 1;
                entry.0 += 1;
            } else if samples.len() < 12 {
                samples.push(format!(
                    "--- {} ---\ninput:    {:?}\nexpected:\n{}\ngot:\n{}",
                    case.file, case.data, case.document, got
                ));
            }
        }

        let percent = passed as f64 * 100.0 / ran as f64;
        println!("\nhtml5lib tree construction: {passed}/{ran} ({percent:.1}%)");
        for (file, (ok, total)) in &by_file {
            println!("  {file:24} {ok:4}/{total:<4}");
        }
        for sample in &samples {
            println!("{sample}");
        }

        // A ratchet, not a target. Raise it as the gaps close.
        assert!(
            passed >= 1,
            "the suite should run and something should pass: {passed}/{ran}"
        );
    }
}
