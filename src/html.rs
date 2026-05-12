use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Element(Element),
    Text(String),
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
    StartTag {
        name: String,
        attributes: BTreeMap<String, String>,
        self_closing: bool,
    },
    EndTag(String),
    Text(String),
}

pub fn parse_document(input: &str) -> Node {
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

fn close_element(stack: &mut Vec<Element>, target: &str) {
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
            tokens.push(Token::Text(input[index..next].to_string()));
            index = next;
            continue;
        }

        if input[index..].starts_with("<!--") {
            if let Some(offset) = input[index + 4..].find("-->") {
                index += 4 + offset + 3;
            } else {
                break;
            }
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
            consume_until_tag_end(input, &mut index);
            continue;
        }

        index += 1;
        skip_whitespace(input, &mut index);

        let name_start = index;
        while index < bytes.len() && is_tag_name_char(bytes[index]) {
            index += 1;
        }

        if name_start == index {
            index += 1;
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

        tokens.push(Token::StartTag {
            name,
            attributes,
            self_closing,
        });
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
        let value = input[start..*index].to_string();
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
        input[start..*index].to_string()
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

#[cfg(test)]
mod tests {
    use super::{Node, parse_document};

    #[test]
    fn parses_text_and_nested_elements() {
        let document = parse_document("<h1>Hello</h1><p>Rust <a href=\"/docs\">docs</a></p>");
        let Node::Element(root) = document else {
            panic!("root should be an element");
        };

        assert_eq!(root.tag_name, "document");
        assert_eq!(root.children.len(), 2);
    }

    #[test]
    fn keeps_attributes() {
        let document = parse_document("<a href=\"/docs\" data-kind=\"nav\">docs</a>");
        let Node::Element(root) = document else {
            panic!("root should be an element");
        };

        let Node::Element(anchor) = &root.children[0] else {
            panic!("first child should be an element");
        };

        assert_eq!(anchor.attribute("href"), Some("/docs"));
        assert_eq!(anchor.attribute("data-kind"), Some("nav"));
    }
}
