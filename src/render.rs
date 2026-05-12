use crate::html::{Element, Node};

pub fn render_document(document: &Node) -> String {
    let mut renderer = Renderer::default();

    let title = find_first_text_by_tag(document, "title");
    let first_heading = find_first_heading(document);

    if let Some(title) = title {
        let duplicate_heading = first_heading
            .as_ref()
            .map(|heading| heading.eq_ignore_ascii_case(&title))
            .unwrap_or(false);

        if !title.is_empty() && !duplicate_heading {
            renderer.render_heading(1, &title);
        }
    }

    renderer.render_node(document);
    renderer.finish()
}

#[derive(Default)]
struct Renderer {
    output: String,
    at_line_start: bool,
    needs_space: bool,
}

impl Renderer {
    fn render_node(&mut self, node: &Node) {
        match node {
            Node::Text(text) => self.push_text(text),
            Node::Element(element) => self.render_element(element),
        }
    }

    fn render_children(&mut self, element: &Element) {
        for child in &element.children {
            self.render_node(child);
        }
    }

    fn render_element(&mut self, element: &Element) {
        match element.tag_name.as_str() {
            "document" | "html" | "body" | "main" | "section" | "article" | "div" | "header"
            | "footer" | "nav" => self.render_children(element),
            "head" | "title" | "script" | "style" | "noscript" => {}
            "h1" => self.render_heading(1, &collect_text(&Node::Element(element.clone()))),
            "h2" => self.render_heading(2, &collect_text(&Node::Element(element.clone()))),
            "h3" => self.render_heading(3, &collect_text(&Node::Element(element.clone()))),
            "h4" => self.render_heading(4, &collect_text(&Node::Element(element.clone()))),
            "h5" => self.render_heading(5, &collect_text(&Node::Element(element.clone()))),
            "h6" => self.render_heading(6, &collect_text(&Node::Element(element.clone()))),
            "p" => {
                self.blank_line();
                self.render_children(element);
                self.blank_line();
            }
            "ul" | "ol" => {
                self.blank_line();
                self.render_children(element);
                self.blank_line();
            }
            "li" => {
                self.start_list_item();
                self.render_children(element);
                self.newline();
            }
            "br" => self.newline(),
            "pre" => {
                self.blank_line();
                self.push_raw(&collect_raw_text(&Node::Element(element.clone())));
                self.blank_line();
            }
            "a" => self.render_link(element),
            "img" => {
                if let Some(alt) = element.attribute("alt") {
                    if !alt.trim().is_empty() {
                        self.push_text(&format!("[image: {alt}]"));
                    }
                }
            }
            _ => self.render_children(element),
        }
    }

    fn render_heading(&mut self, level: usize, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }

        self.blank_line();
        self.push_raw(&format!("{} {}", "#".repeat(level), trimmed));
        self.blank_line();
    }

    fn render_link(&mut self, element: &Element) {
        let text = collect_text(&Node::Element(element.clone()));
        let href = element.attribute("href").unwrap_or("");

        if text.trim().is_empty() {
            if !href.is_empty() {
                self.push_text(href);
            }
            return;
        }

        self.push_text(&text);
        if !href.is_empty() && href != text.trim() {
            self.push_text(&format!("({href})"));
        }
    }

    fn start_list_item(&mut self) {
        if !self.at_line_start {
            self.newline();
        }
        self.push_raw("- ");
    }

    fn push_text(&mut self, text: &str) {
        for word in text.split_whitespace() {
            if self.needs_space && !self.at_line_start {
                self.output.push(' ');
            }
            self.output.push_str(word);
            self.at_line_start = false;
            self.needs_space = true;
        }
    }

    fn push_raw(&mut self, text: &str) {
        self.output.push_str(text);
        self.at_line_start = text.ends_with('\n');
        self.needs_space = !text.ends_with([' ', '\n']);
    }

    fn newline(&mut self) {
        self.trim_trailing_spaces();
        if !self.output.ends_with('\n') {
            self.output.push('\n');
        }
        self.at_line_start = true;
        self.needs_space = false;
    }

    fn blank_line(&mut self) {
        self.trim_trailing_spaces();
        if self.output.is_empty() {
            return;
        }
        if self.output.ends_with("\n\n") {
            self.at_line_start = true;
            self.needs_space = false;
            return;
        }
        if self.output.ends_with('\n') {
            self.output.push('\n');
        } else {
            self.output.push_str("\n\n");
        }
        self.at_line_start = true;
        self.needs_space = false;
    }

    fn trim_trailing_spaces(&mut self) {
        while self.output.ends_with(' ') {
            self.output.pop();
        }
    }

    fn finish(mut self) -> String {
        self.trim_trailing_spaces();
        if !self.output.ends_with('\n') && !self.output.is_empty() {
            self.output.push('\n');
        }
        self.output
    }
}

fn find_first_text_by_tag(node: &Node, tag_name: &str) -> Option<String> {
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
                if let Some(found) = find_first_text_by_tag(child, tag_name) {
                    return Some(found);
                }
            }

            None
        }
    }
}

fn find_first_heading(node: &Node) -> Option<String> {
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
                if let Some(found) = find_first_heading(child) {
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

fn collect_text_into(node: &Node, buffer: &mut String) {
    match node {
        Node::Text(text) => {
            buffer.push_str(text);
            buffer.push(' ');
        }
        Node::Element(element) => {
            if matches!(element.tag_name.as_str(), "script" | "style" | "noscript") {
                return;
            }
            for child in &element.children {
                collect_text_into(child, buffer);
            }
        }
    }
}

fn collect_raw_text(node: &Node) -> String {
    let mut text = String::new();
    collect_raw_text_into(node, &mut text);
    text
}

fn collect_raw_text_into(node: &Node, buffer: &mut String) {
    match node {
        Node::Text(text) => buffer.push_str(text),
        Node::Element(element) => {
            for child in &element.children {
                collect_raw_text_into(child, buffer);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::render_document;
    use crate::html::parse_document;

    #[test]
    fn renders_headings_and_links() {
        let document = parse_document(
            "<html><head><title>Demo</title></head><body><h1>Hello</h1><p>Read <a href=\"/docs\">docs</a></p></body></html>",
        );

        let rendered = render_document(&document);

        assert!(rendered.contains("# Demo"));
        assert!(rendered.contains("# Hello"));
        assert!(rendered.contains("Read docs (/docs)"));
    }

    #[test]
    fn does_not_duplicate_title_when_heading_matches() {
        let document = parse_document(
            "<html><head><title>Example Domain</title></head><body><h1>Example Domain</h1></body></html>",
        );

        let rendered = render_document(&document);
        let heading_count = rendered.matches("# Example Domain").count();

        assert_eq!(heading_count, 1);
    }

    #[test]
    fn renders_lists() {
        let document = parse_document("<ul><li>one</li><li>two</li></ul>");
        let rendered = render_document(&document);

        assert!(rendered.contains("- one"));
        assert!(rendered.contains("- two"));
    }
}
