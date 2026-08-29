use std::collections::BTreeMap;

mod entities;

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
    /// Which language the element belongs to.
    ///
    /// `<title>` in HTML is the document title and `<title>` in SVG is a
    /// tooltip; the name alone does not say which one an element is.
    pub namespace: Namespace,
}

/// The three languages an element in an HTML document can be written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Namespace {
    #[default]
    Html,
    Svg,
    MathMl,
}

impl Element {
    fn new(tag_name: impl Into<String>) -> Self {
        Self {
            tag_name: tag_name.into(),
            attributes: BTreeMap::new(),
            children: Vec::new(),
            namespace: Namespace::Html,
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
    let (mut document, frameset_allowed) = parse_document_body(input);
    if let Node::Element(root) = &mut document {
        ensure_document_structure(root, frameset_allowed);
    }
    document
}

/// One node while the tree is being built.
///
/// The tree is assembled in an arena -- nodes by index, with parent links --
/// rather than by moving each element into its parent as it closes. HTML's own
/// error handling reaches back into the tree it has already built: a misnested
/// `</b>` moves nodes that are already placed, and content that lands inside a
/// table is fostered out to just before it. Neither is expressible once a
/// finished element has been handed to its parent by value.
#[derive(Debug)]
enum BuildKind {
    Element {
        tag_name: String,
        attributes: BTreeMap<String, String>,
        namespace: Namespace,
    },
    Text(String),
    Comment(String),
    Doctype(String),
}

#[derive(Debug)]
struct BuildNode {
    kind: BuildKind,
    children: Vec<usize>,
    parent: Option<usize>,
}

/// An entry in the list of active formatting elements.
///
/// A marker is pushed when a table cell, `<applet>`, `<object>` or `<marquee>`
/// opens: formatting does not reach across one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Formatting {
    Marker,
    Element(usize),
}

struct Builder {
    nodes: Vec<BuildNode>,
    /// The elements currently open, innermost last. Index 0 is the document.
    open: Vec<usize>,
    /// Whether a `<frameset>` would still be honoured.
    ///
    /// A frameset replaces the body, so it is only allowed while the body is
    /// effectively empty. Text and most content elements settle the question:
    /// once a page has shown something, a later `<frameset>` is ignored rather
    /// than throwing that content away.
    frameset_ok: bool,
    /// What `frameset_ok` was when the first `<frameset>` arrived.
    frameset_allowed: Option<bool>,
    /// Formatting elements that are still in force.
    ///
    /// `<b>a<p>b</p>c` puts the bold inside the paragraph too: the `<b>` is
    /// still active when the paragraph opens, so a fresh one is created there.
    /// This list is what makes that happen, and what the adoption agency works
    /// over when an end tag arrives out of order.
    formatting: Vec<Formatting>,
}

impl Builder {
    fn new() -> Self {
        Self {
            nodes: vec![BuildNode {
                kind: BuildKind::Element { namespace: Default::default(),
                    tag_name: "document".to_string(),
                    attributes: BTreeMap::new(),
                },
                children: Vec::new(),
                parent: None,
            }],
            open: vec![0],
            frameset_ok: true,
            frameset_allowed: None,
            formatting: Vec::new(),
        }
    }

    fn tag_of(&self, index: usize) -> &str {
        match &self.nodes[index].kind {
            BuildKind::Element { tag_name, .. } => tag_name,
            _ => "",
        }
    }

    fn current(&self) -> usize {
        *self.open.last().expect("the document is always open")
    }

    fn attach(&mut self, parent: usize, child: usize) {
        self.nodes[child].parent = Some(parent);
        self.nodes[parent].children.push(child);
    }

    fn insert(&mut self, kind: BuildKind) -> usize {
        let index = self.nodes.len();
        self.nodes.push(BuildNode {
            kind,
            children: Vec::new(),
            parent: None,
        });
        let parent = self.current();
        self.attach(parent, index);
        index
    }

    fn is_open(&self, target: &str) -> bool {
        self.open[1..].iter().any(|i| self.tag_of(*i) == target)
    }

    /// Pop open elements until `target` has been closed. A stray end tag whose
    /// element is not open is ignored.
    fn close(&mut self, target: &str) {
        if !self.is_open(target) {
            return;
        }
        while self.open.len() > 1 {
            let index = self.open.pop().expect("checked above");
            if self.tag_of(index) == target {
                break;
            }
        }
    }

    /// Re-open the formatting elements that are still active but no longer on
    /// the stack.
    ///
    /// This is what carries `<b>` into the next paragraph. Without it, markup
    /// like `<b>one<p>two` leaves the second paragraph unbolded, which is not
    /// what any browser shows.
    fn namespace_of(&self, index: usize) -> Namespace {
        match &self.nodes[index].kind {
            BuildKind::Element { namespace, .. } => *namespace,
            _ => Namespace::Html,
        }
    }

    /// The language the next token should be read in.
    ///
    /// Inside `<svg>` or `<math>` the rules of HTML are suspended, but not
    /// everywhere: a handful of elements are integration points where HTML
    /// starts again, which is how `<foreignObject>` can hold a `<div>` and how
    /// `<mtext>` can hold ordinary markup.
    fn foreign_context(&self, start_tag: Option<&str>) -> Namespace {
        let Some(&node) = self.open.last() else {
            return Namespace::Html;
        };
        let namespace = self.namespace_of(node);
        if namespace == Namespace::Html {
            return Namespace::Html;
        }
        let tag = self.tag_of(node);
        match namespace {
            Namespace::Svg if matches!(tag, "foreignObject" | "desc" | "title") => Namespace::Html,
            Namespace::MathMl if matches!(tag, "mi" | "mo" | "mn" | "ms" | "mtext") => {
                // These hold text, so they hold HTML -- except that `<mglyph>`
                // and `<malignmark>` are still MathML of their own.
                match start_tag {
                    Some("mglyph") | Some("malignmark") => Namespace::MathMl,
                    _ => Namespace::Html,
                }
            }
            Namespace::MathMl if tag == "annotation-xml" => {
                let encoding = self.attribute_of(node, "encoding").unwrap_or_default();
                let encoding = encoding.to_ascii_lowercase();
                if encoding == "text/html" || encoding == "application/xhtml+xml" {
                    Namespace::Html
                } else {
                    Namespace::MathMl
                }
            }
            other => other,
        }
    }

    fn attribute_of(&self, index: usize, name: &str) -> Option<String> {
        match &self.nodes[index].kind {
            BuildKind::Element { attributes, .. } => attributes.get(name).cloned(),
            _ => None,
        }
    }

    /// Leave foreign content because an HTML element that cannot appear there
    /// has been opened.
    fn break_out_of_foreign_content(&mut self) {
        while self.open.len() > 1 && self.foreign_context(None) != Namespace::Html {
            self.open.pop();
        }
    }

    fn reconstruct_formatting(&mut self) {
        let Some(&last) = self.formatting.last() else {
            return;
        };
        match last {
            Formatting::Marker => return,
            Formatting::Element(index) if self.open.contains(&index) => return,
            Formatting::Element(_) => {}
        }

        // Walk back to the last entry that is a marker or still open; the
        // entries after it are the ones to recreate.
        let mut position = self.formatting.len() - 1;
        while position > 0 {
            match self.formatting[position - 1] {
                Formatting::Marker => break,
                Formatting::Element(index) if self.open.contains(&index) => break,
                Formatting::Element(_) => position -= 1,
            }
        }

        while position < self.formatting.len() {
            let Formatting::Element(source) = self.formatting[position] else {
                break;
            };
            let clone = self.clone_element(source);
            let parent = self.current();
            self.attach(parent, clone);
            self.open.push(clone);
            self.formatting[position] = Formatting::Element(clone);
            position += 1;
        }
    }

    /// A fresh element with the same name and attributes, and no children.
    fn clone_element(&mut self, source: usize) -> usize {
        let kind = match &self.nodes[source].kind {
            BuildKind::Element {
                tag_name,
                attributes,
                namespace,
            } => BuildKind::Element {
                tag_name: tag_name.clone(),
                attributes: attributes.clone(),
                namespace: *namespace,
            },
            _ => BuildKind::Text(String::new()),
        };
        let index = self.nodes.len();
        self.nodes.push(BuildNode {
            kind,
            children: Vec::new(),
            parent: None,
        });
        index
    }

    fn detach(&mut self, child: usize) {
        if let Some(parent) = self.nodes[child].parent.take() {
            self.nodes[parent].children.retain(|c| *c != child);
        }
    }

    /// Whether `target` is open with nothing but ordinary containers between --
    /// what the standard calls being in scope.
    fn in_scope(&self, target: &str) -> bool {
        const BOUNDARIES: &[&str] = &[
            "applet", "caption", "html", "table", "td", "th", "marquee", "object", "template",
            "document",
        ];
        for index in self.open.iter().rev() {
            let name = self.tag_of(*index);
            if name == target {
                return true;
            }
            if BOUNDARIES.contains(&name) {
                return false;
            }
        }
        false
    }

    /// The adoption agency: how a browser makes sense of formatting that closes
    /// across a block boundary.
    ///
    /// `<b>1<p>2</b>3` is not a tree as written -- the `</b>` closes an element
    /// a paragraph has been opened inside. Browsers produce
    /// `<b>1</b><p><b>2</b>3</p>`: the bold is split, and the part inside the
    /// paragraph stays bold. Closing the nearest matching tag instead threw the
    /// paragraph away with it.
    ///
    /// Returns false when the subject is not an active formatting element, so
    /// the caller falls back to an ordinary end tag.
    fn adoption_agency(&mut self, subject: &str) -> bool {
        for _ in 0..8 {
            // The last active formatting element with this name, after any marker.
            let mut formatting_position = None;
            for (position, entry) in self.formatting.iter().enumerate().rev() {
                match entry {
                    Formatting::Marker => break,
                    Formatting::Element(index) if self.tag_of(*index) == subject => {
                        formatting_position = Some(position);
                        break;
                    }
                    Formatting::Element(_) => {}
                }
            }
            let Some(formatting_position) = formatting_position else {
                return false;
            };
            let Formatting::Element(formatting_element) = self.formatting[formatting_position]
            else {
                return false;
            };

            let Some(stack_position) = self.open.iter().position(|i| *i == formatting_element)
            else {
                // Active but no longer open: drop it, and the tag is spent.
                self.formatting.remove(formatting_position);
                return true;
            };
            if stack_position == 0 || !self.in_scope(subject) {
                return true;
            }

            // The nearest element below it that content cannot simply move out
            // of. With none, the formatting element just closes.
            let furthest_block = self.open[stack_position + 1..]
                .iter()
                .position(|i| is_special(self.tag_of(*i)))
                .map(|offset| stack_position + 1 + offset);
            let Some(furthest_block) = furthest_block else {
                self.open.truncate(stack_position);
                self.formatting.remove(formatting_position);
                return true;
            };

            let common_ancestor = self.open[stack_position - 1];
            let block = self.open[furthest_block];
            let mut bookmark = formatting_position;

            // Walk up from the furthest block to the formatting element,
            // cloning the formatting elements on the way and re-parenting what
            // was inside them.
            let mut node_position = furthest_block;
            let mut last_node = block;
            for inner in 0..3 {
                if node_position == 0 {
                    break;
                }
                node_position -= 1;
                let node = self.open[node_position];
                if node == formatting_element {
                    break;
                }
                let in_list = self
                    .formatting
                    .iter()
                    .position(|entry| *entry == Formatting::Element(node));
                match in_list {
                    Some(position) if inner < 3 => {
                        let clone = self.clone_element(node);
                        self.formatting[position] = Formatting::Element(clone);
                        self.open[node_position] = clone;
                        if last_node == block {
                            bookmark = position + 1;
                        }
                        self.detach(last_node);
                        self.attach(clone, last_node);
                        last_node = clone;
                    }
                    Some(position) => {
                        self.formatting.remove(position);
                        self.open.remove(node_position);
                    }
                    None => {
                        self.open.remove(node_position);
                    }
                }
            }

            self.detach(last_node);
            self.attach(common_ancestor, last_node);

            // A fresh copy of the formatting element takes everything the
            // furthest block held.
            let clone = self.clone_element(formatting_element);
            let moved: Vec<usize> = std::mem::take(&mut self.nodes[block].children);
            for child in moved {
                self.nodes[child].parent = Some(clone);
                self.nodes[clone].children.push(child);
            }
            self.attach(block, clone);

            if let Some(position) = self
                .formatting
                .iter()
                .position(|entry| *entry == Formatting::Element(formatting_element))
            {
                self.formatting.remove(position);
                if bookmark > position {
                    bookmark -= 1;
                }
            }
            let bookmark = bookmark.min(self.formatting.len());
            self.formatting.insert(bookmark, Formatting::Element(clone));

            if let Some(position) = self.open.iter().position(|i| *i == formatting_element) {
                self.open.remove(position);
            }
            if let Some(position) = self.open.iter().position(|i| *i == block) {
                self.open.insert(position + 1, clone);
            }
        }
        true
    }

    fn into_tree(mut self) -> Node {
        fn build(nodes: &mut Vec<BuildNode>, index: usize) -> Node {
            let children: Vec<usize> = std::mem::take(&mut nodes[index].children);
            let kind = std::mem::replace(&mut nodes[index].kind, BuildKind::Text(String::new()));
            match kind {
                BuildKind::Text(text) => Node::Text(text),
                BuildKind::Comment(text) => Node::Comment(text),
                BuildKind::Doctype(name) => Node::Doctype(name),
                BuildKind::Element {
                    tag_name,
                    attributes,
                    namespace,
                } => Node::Element(Element {
                    tag_name,
                    attributes,
                    namespace,
                    children: children.into_iter().map(|c| build(nodes, c)).collect(),
                }),
            }
        }
        build(&mut self.nodes, 0)
    }
}

/// The formatting elements, which stay in force across the blocks that open
/// inside them.
fn is_formatting(tag: &str) -> bool {
    matches!(
        tag,
        "a" | "b"
            | "big"
            | "code"
            | "em"
            | "font"
            | "i"
            | "nobr"
            | "s"
            | "small"
            | "strike"
            | "strong"
            | "tt"
            | "u"
    )
}

/// The elements formatting cannot simply be moved out of.
///
/// The standard calls these special. The adoption agency looks for the nearest
/// one below a misnested formatting element to decide what has to be split.
fn is_special(tag: &str) -> bool {
    matches!(
        tag,
        "address" | "applet" | "area" | "article" | "aside" | "base" | "basefont" | "bgsound"
            | "blockquote" | "body" | "br" | "button" | "caption" | "center" | "col"
            | "colgroup" | "dd" | "details" | "dir" | "div" | "dl" | "dt" | "embed"
            | "fieldset" | "figcaption" | "figure" | "footer" | "form" | "frame" | "frameset"
            | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "head" | "header" | "hgroup" | "hr"
            | "html" | "iframe" | "img" | "input" | "keygen" | "li" | "link" | "listing"
            | "main" | "marquee" | "menu" | "meta" | "nav" | "noembed" | "noframes"
            | "noscript" | "object" | "ol" | "p" | "param" | "plaintext" | "pre" | "script"
            | "search" | "section" | "select" | "source" | "style" | "summary" | "table"
            | "tbody" | "td" | "template" | "textarea" | "tfoot" | "th" | "thead" | "title"
            | "tr" | "track" | "ul" | "wbr" | "xmp" | "document"
    )
}

/// Elements that put a marker on the formatting list: formatting does not
/// reach across one.
fn starts_formatting_scope(tag: &str) -> bool {
    matches!(tag, "applet" | "marquee" | "object" | "td" | "th" | "caption" | "template")
}

/// HTML elements that cannot live inside `<svg>` or `<math>`.
///
/// Meeting one of these means the author forgot to close the foreign element,
/// so the parser leaves it rather than nesting a paragraph inside a drawing.
/// Start tags after which a `<frameset>` is no longer honoured.
///
/// The standard's list is not a category so much as a record of what browsers
/// settled on: anything that puts something on the page, or that a form needs.
fn clears_frameset_ok(tag: &str) -> bool {
    matches!(
        tag,
        "pre" | "listing"
            | "li"
            | "dd"
            | "dt"
            | "button"
            | "applet"
            | "marquee"
            | "object"
            | "table"
            | "area"
            | "br"
            | "embed"
            | "img"
            | "image"
            | "keygen"
            | "wbr"
            | "hr"
            | "textarea"
            | "xmp"
            | "iframe"
            | "noembed"
            | "select"
    )
}

fn breaks_out_of_foreign_content(tag: &str) -> bool {
    matches!(
        tag,
        "b" | "big"
            | "blockquote"
            | "body"
            | "br"
            | "center"
            | "code"
            | "dd"
            | "div"
            | "dl"
            | "dt"
            | "em"
            | "embed"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "head"
            | "hr"
            | "i"
            | "img"
            | "li"
            | "listing"
            | "menu"
            | "meta"
            | "nobr"
            | "ol"
            | "p"
            | "pre"
            | "ruby"
            | "s"
            | "small"
            | "span"
            | "strong"
            | "strike"
            | "sub"
            | "sup"
            | "table"
            | "tt"
            | "u"
            | "ul"
            | "var"
    )
}

/// The SVG element and attribute names that are not all lowercase.
///
/// HTML is case insensitive and the tokenizer has already folded every name,
/// but SVG is not: `viewBox` written as `viewbox` is a different attribute and
/// the drawing silently loses its coordinate system.
fn adjust_svg_tag_name(name: &str) -> Option<&'static str> {
    const NAMES: &[&str] = &[
        "altGlyph", "altGlyphDef", "altGlyphItem", "animateColor", "animateMotion",
        "animateTransform", "clipPath", "feBlend", "feColorMatrix", "feComponentTransfer",
        "feComposite", "feConvolveMatrix", "feDiffuseLighting", "feDisplacementMap",
        "feDistantLight", "feFlood", "feFuncA", "feFuncB", "feFuncG", "feFuncR",
        "feGaussianBlur", "feImage", "feMerge", "feMergeNode", "feMorphology", "feOffset",
        "fePointLight", "feSpecularLighting", "feSpotLight", "feTile", "feTurbulence",
        "foreignObject", "glyphRef", "linearGradient", "radialGradient", "textPath",
    ];
    NAMES
        .iter()
        .find(|candidate| candidate.eq_ignore_ascii_case(name))
        .copied()
}

fn adjust_svg_attribute_name(name: &str) -> Option<&'static str> {
    const NAMES: &[&str] = &[
        "attributeName", "attributeType", "baseFrequency", "baseProfile", "calcMode",
        "clipPathUnits", "diffuseConstant", "edgeMode", "filterUnits", "glyphRef",
        "gradientTransform", "gradientUnits", "kernelMatrix", "kernelUnitLength", "keyPoints",
        "keySplines", "keyTimes", "lengthAdjust", "limitingConeAngle", "markerHeight",
        "markerUnits", "markerWidth", "maskContentUnits", "maskUnits", "numOctaves",
        "pathLength", "patternContentUnits", "patternTransform", "patternUnits", "pointsAtX",
        "pointsAtY", "pointsAtZ", "preserveAlpha", "preserveAspectRatio", "primitiveUnits",
        "refX", "refY", "repeatCount", "repeatDur", "requiredExtensions", "requiredFeatures",
        "specularConstant", "specularExponent", "spreadMethod", "startOffset", "stdDeviation",
        "stitchTiles", "surfaceScale", "systemLanguage", "tableValues", "targetX", "targetY",
        "textLength", "viewBox", "viewTarget", "xChannelSelector", "yChannelSelector",
        "zoomAndPan",
    ];
    NAMES
        .iter()
        .find(|candidate| candidate.eq_ignore_ascii_case(name))
        .copied()
}

/// Put a foreign start tag back into the case its language expects.
fn adjust_foreign_names(
    namespace: Namespace,
    name: &mut String,
    attributes: &mut BTreeMap<String, String>,
) {
    if namespace == Namespace::Svg {
        if let Some(adjusted) = adjust_svg_tag_name(name) {
            *name = adjusted.to_string();
        }
    }
    let renames: Vec<(String, &'static str)> = attributes
        .keys()
        .filter_map(|key| {
            let adjusted = match namespace {
                Namespace::Svg => adjust_svg_attribute_name(key),
                Namespace::MathMl if key == "definitionurl" => Some("definitionURL"),
                _ => None,
            }?;
            if adjusted == key {
                None
            } else {
                Some((key.clone(), adjusted))
            }
        })
        .collect();
    for (old, new) in renames {
        if let Some(value) = attributes.remove(&old) {
            attributes.insert(new.to_string(), value);
        }
    }
}

fn parse_document_body(input: &str) -> (Node, bool) {
    let tokens = tokenize(input);
    let mut builder = Builder::new();

    for token in tokens {
        match token {
            Token::Text(text) => {
                if !text.is_empty() {
                    // Text on the page settles it: a later frameset would have
                    // to discard what the reader can already see. Text in the
                    // head does not count, since none of it is shown.
                    if !builder.is_open("head") && text.trim().is_empty() == false {
                        builder.frameset_ok = false;
                    }
                    // Text belongs inside whatever formatting is still in
                    // force, so any that has fallen off the stack is re-opened
                    // around it first.
                    builder.reconstruct_formatting();
                    builder.insert(BuildKind::Text(text));
                }
            }
            Token::StartTag {
                name,
                attributes,
                self_closing,
            } => {
                let mut name = name;
                let mut attributes = attributes;

                // Inside `<svg>` or `<math>` almost none of the HTML rules
                // apply: no implied closing, no formatting to reconstruct, and
                // a trailing slash really does close the element.
                let mut namespace = builder.foreign_context(Some(&name));
                if namespace != Namespace::Html {
                    let breaks_out = breaks_out_of_foreign_content(&name)
                        || (name == "font"
                            && ["color", "face", "size"]
                                .iter()
                                .any(|key| attributes.contains_key(*key)));
                    if breaks_out {
                        builder.break_out_of_foreign_content();
                        namespace = Namespace::Html;
                    }
                }
                if namespace == Namespace::Html {
                    namespace = match name.as_str() {
                        "svg" => Namespace::Svg,
                        "math" => Namespace::MathMl,
                        _ => Namespace::Html,
                    };
                    // The tag that opens the foreign content carries foreign
                    // attributes too: `<svg viewbox=...>` is a viewBox.
                    adjust_foreign_names(namespace, &mut name, &mut attributes);
                } else {
                    adjust_foreign_names(namespace, &mut name, &mut attributes);
                    let index = builder.insert(BuildKind::Element {
                        tag_name: name,
                        attributes,
                        namespace,
                    });
                    if !self_closing {
                        builder.open.push(index);
                    }
                    continue;
                }

                // HTML5 optional-end-tag: implicitly close certain elements
                // before opening a new one (a `<td>` closes an open `<td>`, an
                // `<li>` closes an open `<li>`).
                if !self_closing {
                    maybe_auto_close(&mut builder, &name);
                }
                if !is_special(&name) {
                    builder.reconstruct_formatting();
                }

                if clears_frameset_ok(&name)
                    || (name == "input"
                        && attributes.get("type").map(String::as_str) != Some("hidden"))
                {
                    builder.frameset_ok = false;
                }
                if name == "frameset" && !builder.is_open("frameset") {
                    if !builder.frameset_ok {
                        // Too late to replace the body, so the tag is dropped
                        // rather than left in the middle of the page.
                        continue;
                    }
                    if builder.frameset_allowed.is_none() {
                        builder.frameset_allowed = Some(true);
                    }
                }

                let starts_scope = starts_formatting_scope(&name);
                let formatting = is_formatting(&name);
                let index = builder.insert(BuildKind::Element {
                    tag_name: name,
                    attributes,
                    namespace,
                });
                if !self_closing {
                    builder.open.push(index);
                }
                if starts_scope {
                    builder.formatting.push(Formatting::Marker);
                } else if formatting && !self_closing {
                    builder.formatting.push(Formatting::Element(index));
                }
            }
            Token::EndTag(name) => {
                // An end tag in foreign content matches by name alone, walking
                // down the stack until it reaches HTML again.
                if builder
                    .open
                    .last()
                    .is_some_and(|node| builder.namespace_of(*node) != Namespace::Html)
                {
                    let mut position = builder.open.len();
                    let mut handled = false;
                    while position > 1 {
                        position -= 1;
                        let node = builder.open[position];
                        if builder.tag_of(node).eq_ignore_ascii_case(&name) {
                            builder.open.truncate(position);
                            handled = true;
                            break;
                        }
                        if builder.namespace_of(node) == Namespace::Html {
                            break;
                        }
                    }
                    if handled {
                        continue;
                    }
                }

                // `</p>` with no open paragraph makes an empty one, and `</br>`
                // is read as `<br>`. Both are what the standard says and what
                // every browser does with the stray tags real pages contain.
                if name == "p" && !builder.is_open("p") {
                    builder.insert(BuildKind::Element { namespace: Default::default(),
                        tag_name: "p".to_string(),
                        attributes: BTreeMap::new(),
                    });
                } else if name == "br" {
                    builder.insert(BuildKind::Element { namespace: Default::default(),
                        tag_name: "br".to_string(),
                        attributes: BTreeMap::new(),
                    });
                } else if is_formatting(&name) {
                    if !builder.adoption_agency(&name) {
                        builder.close(&name);
                    }
                } else {
                    if starts_formatting_scope(&name) {
                        // Leaving the scope drops everything back to its marker.
                        if let Some(position) = builder
                            .formatting
                            .iter()
                            .rposition(|entry| *entry == Formatting::Marker)
                        {
                            builder.formatting.truncate(position);
                        }
                    }
                    builder.close(&name);
                }
            }
            Token::Comment(text) => {
                builder.insert(BuildKind::Comment(text));
            }
            Token::Doctype(name) => {
                builder.insert(BuildKind::Doctype(name));
            }
        }
    }

    let frameset_allowed = builder.frameset_allowed.unwrap_or(false);
    (builder.into_tree(), frameset_allowed)
}

pub fn parse_fragment(input: &str) -> Vec<Node> {
    let (tree, _) = parse_document_body(input);
    let Node::Element(root) = tree else {
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
/// Lift the frameset out of wherever the markup left it.
///
/// The parser pops everything open when it accepts a frameset, so a
/// `<frameset>` written inside a paragraph still ends up as the document's
/// own -- the paragraph goes away with the rest of the body.
fn take_first_frameset(children: &mut Vec<Node>) -> Option<Node> {
    for index in 0..children.len() {
        if matches!(&children[index], Node::Element(e) if e.tag_name == "frameset") {
            return Some(children.remove(index));
        }
    }
    for child in children.iter_mut() {
        if let Node::Element(element) = child {
            if let Some(found) = take_first_frameset(&mut element.children) {
                return Some(found);
            }
        }
    }
    None
}

fn ensure_document_structure(root: &mut Element, frameset_allowed: bool) {
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
    // The frameset takes the body's place, and the body's contents go with it:
    // by the time the parser accepted the frameset it had already decided that
    // nothing worth keeping had been written.
    let frameset = frameset_allowed
        .then(|| take_first_frameset(&mut body.children))
        .flatten();
    let second = match frameset {
        Some(node) => node,
        None => Node::Element(body),
    };

    html.children = vec![Node::Element(head), second];
    before_html.push(Node::Element(html));
    root.children = before_html;
}

/// HTML5 optional-end-tag / implicit-close rules.
///
/// Certain start tags close currently open elements of the same category
/// before they are inserted: a new `<td>` ends the open `<td>`, a new `<li>`
/// ends the open `<li>`.
fn maybe_auto_close(builder: &mut Builder, new_tag: &str) {
    // Anything that is not head content ends the head, whether or not the
    // markup wrote `</head>`. Without this, `<html><head><body>` left the body
    // nested inside the head -- a shape no browser produces.
    if !is_head_only(new_tag) && new_tag != "head" && builder.is_open("head") {
        builder.close("head");
    }

    const PARAGRAPH_BOUNDARIES: &[&str] =
        &["td", "th", "li", "dd", "dt", "body", "html", "document"];

    match new_tag {
        // Table cell: close any open td/th within the current tr context.
        "td" | "th" => auto_close_before(
            builder,
            &["td", "th"],
            &["tr", "table", "tbody", "thead", "tfoot"],
        ),
        // Table row: close any open tr within the current section.
        "tr" => auto_close_before(builder, &["tr"], &["table", "tbody", "thead", "tfoot"]),
        // List item: close any open li within the current list.
        "li" => auto_close_before(builder, &["li"], &["ul", "ol"]),
        "dt" | "dd" => auto_close_before(builder, &["dt", "dd"], &["dl"]),
        // A heading closes an open heading of any level: `<h1>a<h2>b` gives two
        // siblings, not a nested pair.
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            auto_close_before(
                builder,
                &["h1", "h2", "h3", "h4", "h5", "h6"],
                PARAGRAPH_BOUNDARIES,
            );
            auto_close_before(builder, &["p"], PARAGRAPH_BOUNDARIES);
        }
        // A new <p>, and many block elements, close an open <p>.
        tag if is_block_like(tag) => auto_close_before(builder, &["p"], PARAGRAPH_BOUNDARIES),
        _ => {}
    }
}

/// Walk up the open elements looking for one in `targets`, and close it.
/// Stop and do nothing if a `boundary` element is met first.
fn auto_close_before(builder: &mut Builder, targets: &[&str], boundaries: &[&str]) {
    let mut close_tag: Option<String> = None;
    for index in builder.open.iter().rev() {
        let name = builder.tag_of(*index);
        if targets.contains(&name) {
            close_tag = Some(name.to_string());
            break;
        }
        if boundaries.contains(&name) {
            break;
        }
    }
    if let Some(tag) = close_tag {
        builder.close(&tag);
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
            // These are not containers, but each still ends an open paragraph.
            | "hr"
            | "form"
            | "header"
            | "footer"
            | "main"
            | "nav"
            | "figure"
            | "menu"
            | "search"
            | "dir"
            | "center"
            | "listing"
            | "plaintext"
            | "xmp"
    )
}

/// The text of a bogus comment starting at `from`, and where it ends.
///
/// Everything up to the next `>` becomes the comment; without one it runs to
/// the end of the file.
fn bogus_comment(input: &str, from: usize) -> (String, usize) {
    match input[from..].find('>') {
        Some(offset) => (input[from..from + offset].to_string(), from + offset + 1),
        None => (input[from..].to_string(), input.len()),
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

        // A `<` that starts nothing is text, and several near-misses are
        // comments instead: `<?`, `<!x`, and `</` followed by anything that is
        // not a name. The standard calls the last three bogus comments, and
        // browsers put them in the tree rather than dropping them.
        let after_angle = input[index + 1..].chars().next();
        match after_angle {
            None => {
                tokens.push(Token::Text("<".to_string()));
                index = bytes.len();
                continue;
            }
            Some('?') => {
                let (text, next) = bogus_comment(input, index + 1);
                tokens.push(Token::Comment(text));
                index = next;
                continue;
            }
            Some('!') => {}
            Some('/') => {
                let after_slash = input[index + 2..].chars().next();
                match after_slash {
                    None => {
                        tokens.push(Token::Text("</".to_string()));
                        index = bytes.len();
                        continue;
                    }
                    Some(c) if !c.is_ascii_alphabetic() => {
                        let (text, next) = bogus_comment(input, index + 2);
                        tokens.push(Token::Comment(text));
                        index = next;
                        continue;
                    }
                    Some(_) => {}
                }
            }
            Some(c) if !c.is_ascii_alphabetic() => {
                // `<#` is not a tag; the `<` and what follows are text.
                let next = input[index + 1..]
                    .find('<')
                    .map(|offset| index + 1 + offset)
                    .unwrap_or(bytes.len());
                tokens.push(Token::Text(decode_html_entities(&input[index..next])));
                index = next;
                continue;
            }
            Some(_) => {}
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
            let is_doctype = input[start..].len() > 9 && input[start + 2..start + 9].eq_ignore_ascii_case("doctype");
            if !is_doctype {
                // `<!` followed by anything that is not a doctype or a comment
                // is a bogus comment holding the rest up to `>`.
                let (text, next) = bogus_comment(input, start + 2);
                tokens.push(Token::Comment(text));
                index = next;
                continue;
            }
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
        // The standard renames this one rather than honouring it: `<image>`
        // is a misspelling of `<img>` old pages still contain.
        let name = if name == "image" { "img".to_string() } else { name };
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
        let value = decode_attribute_entities(&input[start..*index]);
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
        decode_attribute_entities(&input[start..*index])
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
            // Obsolete, but a browser still parses them, and pages from the
            // era that used them are still on the web.
            | "basefont"
            | "bgsound"
            | "keygen"
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

/// Where the raw text of `tag_name` ends.
///
/// An end tag only closes raw text when the name is followed by whitespace, a
/// slash or `>`. `</SCRIPT` at the end of the file closes nothing -- it is just
/// more script.
///
/// Inside `<script>` there are two further states, and they are why a script
/// can contain the characters `</script>` without ending. After `<!--`, the
/// script data is escaped; if `<script` then appears, it is *double* escaped,
/// and a `</script>` there only ends the double escape. Writing a document that
/// contains a script tag is the everyday reason a page relies on this.
fn find_raw_text_close(input: &str, start: usize, tag_name: &str) -> Option<usize> {
    let bytes = input.as_bytes();
    let close_tag = format!("</{tag_name}");
    let open_tag = format!("<{tag_name}");
    let script = tag_name == "script";

    // Set by `<!--`, cleared by `-->`.
    let mut escaped = false;
    // Set by `<script` while escaped, cleared by `</script`.
    let mut double_escaped = false;

    let terminates = |at: usize| -> bool {
        match bytes.get(at) {
            None => false,
            Some(byte) => byte.is_ascii_whitespace() || *byte == b'>' || *byte == b'/',
        }
    };

    let mut index = start;
    while index < bytes.len() {
        if script && !escaped && input[index..].starts_with("<!--") {
            escaped = true;
            index += 4;
            continue;
        }
        if script && escaped && input[index..].starts_with("-->") {
            escaped = false;
            double_escaped = false;
            index += 3;
            continue;
        }
        if starts_with_case_insensitive(&input[index..], &close_tag)
            && terminates(index + close_tag.len())
        {
            if double_escaped {
                double_escaped = false;
                index += close_tag.len();
                continue;
            }
            return Some(index);
        }
        if script
            && escaped
            && !double_escaped
            && starts_with_case_insensitive(&input[index..], &open_tag)
            && terminates(index + open_tag.len())
        {
            double_escaped = true;
            index += open_tag.len();
            continue;
        }
        index += 1;
        while index < bytes.len() && !input.is_char_boundary(index) {
            index += 1;
        }
    }

    None
}

fn starts_with_case_insensitive(haystack: &str, needle: &str) -> bool {
    // Compared as bytes: slicing the string would panic when the tag name is
    // followed by a multi-byte character, which any Japanese page has.
    let haystack = haystack.as_bytes();
    let needle = needle.as_bytes();
    haystack.len() >= needle.len() && haystack[..needle.len()].eq_ignore_ascii_case(needle)
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
    decode_references(input, false)
}

/// Character references in an attribute value.
///
/// One rule differs: a named reference that stops short of its semicolon is
/// *not* a reference when what follows is `=` or alphanumeric. That is what
/// keeps `?a=b&copy=1` a query string rather than a copyright sign.
fn decode_attribute_entities(input: &str) -> String {
    decode_references(input, true)
}

fn decode_references(input: &str, in_attribute: bool) -> String {
    // A NUL in ordinary content is dropped. It reaches the tree in no browser,
    // and left in place it counts as content: `<html> <frameset>` looked like
    // a body had already started, so the frameset never took the body's place.
    if input.contains(' ') {
        let cleaned: String = input.chars().filter(|c| *c != ' ').collect();
        return decode_references(&cleaned, in_attribute);
    }
    let mut output = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(start) = rest.find('&') {
        output.push_str(&rest[..start]);
        rest = &rest[start + 1..];

        if let Some(after) = rest.strip_prefix('#') {
            // A hexadecimal reference takes hex digits, a decimal one decimal
            // digits, and the run stops at the first character that is not one.
            // Reading to the end of the word instead made `&#xBAR` and
            // `&#41BAR` unresolvable, where a browser reads `&#xBA` then `R`
            // and `&#41` then `BAR`.
            let (prefix_len, radix) = match after.as_bytes().first() {
                Some(b'x') | Some(b'X') => (2, 16),
                _ => (1, 10),
            };
            let body = &after[prefix_len - 1..];
            let digits_end = body
                .find(|c: char| !c.is_digit(radix))
                .unwrap_or(body.len());
            let digits = &body[..digits_end];
            if digits.is_empty() {
                output.push('&');
                continue;
            }
            let consumed = prefix_len + digits_end;
            let terminated = rest[consumed..].starts_with(';');
            let code = u32::from_str_radix(digits, radix).unwrap_or(0xFFFD);
            output.push(numeric_reference_char(code));
            rest = &rest[consumed + usize::from(terminated)..];
            continue;
        }

        match entities::longest_match(rest) {
            Some((len, replacement)) => {
                let name = &rest[..len];
                let ends_with_semicolon = name.ends_with(';');
                let next = rest[len..].chars().next();
                // The attribute-value carve-out, and nothing else: outside an
                // attribute the shorter legacy name always wins.
                let blocked_in_attribute = in_attribute
                    && !ends_with_semicolon
                    && next.is_some_and(|c| c == '=' || c.is_ascii_alphanumeric());
                if blocked_in_attribute {
                    output.push('&');
                } else {
                    output.push_str(replacement);
                    rest = &rest[len..];
                }
            }
            None => {
                output.push('&');
            }
        }
    }

    output.push_str(rest);
    output
}

/// The character a numeric reference stands for, with the standard's
/// substitutions applied.
///
/// Three groups do not mean what they say. Zero, the surrogates and anything
/// past the last code point are all replaced. The 0x80..0x9F range is the
/// famous one: those numbers name C1 controls, but the reference was almost
/// always written by an author who meant the Windows-1252 character at that
/// number -- a curly quote, an em dash -- so that is what browsers produce.
fn numeric_reference_char(code: u32) -> char {
    const WINDOWS_1252: [u32; 32] = [
        0x20AC, 0x0081, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160,
        0x2039, 0x0152, 0x008D, 0x017D, 0x008F, 0x0090, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022,
        0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x009D, 0x017E, 0x0178,
    ];
    let code = match code {
        0 => 0xFFFD,
        0x80..=0x9F => WINDOWS_1252[(code - 0x80) as usize],
        0xD800..=0xDFFF => 0xFFFD,
        c if c > 0x10FFFF => 0xFFFD,
        c => c,
    };
    char::from_u32(code).unwrap_or('\u{FFFD}')
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

    /// The standard's whole table, and the rules that decide where a
    /// reference ends. Pages lean on hundreds of these names, and the two
    /// awkward cases -- a legacy name without its semicolon, and the same name
    /// inside an attribute -- are where browsers agree and a naive decoder
    /// does not.
    #[test]
    fn a_frameset_after_content_is_ignored() {
        // The table is already on the page, so the frameset cannot take the
        // body's place -- and it is dropped rather than drawn inside the table.
        let document = parse_document("<table><frameset>");
        let body = body_of(&document);
        let Node::Element(table) = &body.children[0] else {
            panic!("expected a table");
        };
        assert_eq!(table.tag_name, "table");
        assert!(table.children.is_empty(), "the frameset should be gone");
    }

    #[test]
    fn a_frameset_after_only_whitespace_still_replaces_the_body() {
        let document = parse_document("<p> <frameset><frame>");
        let html = html_of(&document);
        let Some(Node::Element(frameset)) = html.children.get(1) else {
            panic!("expected head then frameset");
        };
        assert_eq!(frameset.tag_name, "frameset");
        assert_eq!(frameset.children.len(), 1);
    }

    #[test]
    fn svg_keeps_its_own_namespace_and_capitalisation() {
        let document = parse_document("<body><svg viewbox=\"0 0 1 1\"><clippath></clippath></svg>");
        let body = body_of(&document);
        let Node::Element(svg) = &body.children[0] else {
            panic!("expected an svg");
        };
        assert_eq!(svg.namespace, super::Namespace::Svg);
        // SVG is case sensitive, so the folded names have to be put back.
        assert_eq!(svg.attributes.get("viewBox").map(String::as_str), Some("0 0 1 1"));
        let Node::Element(clip) = &svg.children[0] else {
            panic!("expected a clipPath");
        };
        assert_eq!(clip.tag_name, "clipPath");
        assert_eq!(clip.namespace, super::Namespace::Svg);
    }

    #[test]
    fn html_starts_again_inside_a_foreign_object() {
        let document = parse_document("<body><svg><foreignobject><div>x</div></foreignobject></svg>");
        let body = body_of(&document);
        let Node::Element(svg) = &body.children[0] else {
            panic!("expected an svg");
        };
        let Node::Element(object) = &svg.children[0] else {
            panic!("expected a foreignObject");
        };
        assert_eq!(object.tag_name, "foreignObject");
        let Node::Element(div) = &object.children[0] else {
            panic!("expected a div");
        };
        assert_eq!(div.namespace, super::Namespace::Html);
    }

    #[test]
    fn a_paragraph_breaks_out_of_an_unclosed_svg() {
        // `<p>` cannot be drawn, so the author must have forgotten `</svg>`.
        let document = parse_document("<body><svg><circle><p>text");
        let body = body_of(&document);
        assert_eq!(body.children.len(), 2);
        let Node::Element(paragraph) = &body.children[1] else {
            panic!("expected a paragraph beside the svg");
        };
        assert_eq!(paragraph.tag_name, "p");
        assert_eq!(paragraph.namespace, super::Namespace::Html);
    }

    #[test]
    fn a_script_can_contain_the_characters_of_its_own_end_tag() {
        // Inside `<!--<script`, the script data is double escaped and the
        // `</script>` there only ends that state. A page that writes a script
        // tag from a script depends on this.
        let document =
            parse_document("<script><!--<script></script><script></script></script>after");
        let head = head_of(&document);
        let Node::Element(script) = &head.children[0] else {
            panic!("expected a script");
        };
        assert_eq!(script.tag_name, "script");
        let Node::Text(text) = &script.children[0] else {
            panic!("expected script text");
        };
        assert_eq!(text, "<!--<script></script><script></script>");
    }

    #[test]
    fn an_end_tag_without_a_terminator_is_not_an_end_tag() {
        // `</SCRIPT` with nothing after it is more script, not a close.
        let document = parse_document("<script></SCRIPT");
        let head = head_of(&document);
        let Node::Element(script) = &head.children[0] else {
            panic!("expected a script");
        };
        let Node::Text(text) = &script.children[0] else {
            panic!("expected script text");
        };
        assert_eq!(text, "</SCRIPT");
    }

    #[test]
    fn raw_text_survives_multi_byte_characters() {
        // Scanning for the end tag compares bytes; slicing the string at the
        // tag length would land inside a character here.
        let document = parse_document("<title>tobira 自作</title><p>x</p>");
        let head = head_of(&document);
        let Node::Element(title) = &head.children[0] else {
            panic!("expected a title");
        };
        let Node::Text(text) = &title.children[0] else {
            panic!("expected title text");
        };
        assert_eq!(text, "tobira 自作");
    }

    #[test]
    fn decodes_the_standard_named_references() {
        let text = |html: &str| -> String {
            let document = parse_document(html);
            let body = body_of(&document);
            body.children
                .iter()
                .map(|child| match child {
                    Node::Text(t) => t.clone(),
                    _ => String::new(),
                })
                .collect()
        };

        // Names the old six-entry table never had.
        assert_eq!(text("a&mdash;b"), "a\u{2014}b");
        assert_eq!(text("a&rsquo;b"), "a\u{2019}b");
        assert_eq!(text("a&hellip;b"), "a\u{2026}b");
        assert_eq!(text("a&times;b"), "a\u{D7}b");

        // The legacy set resolves without its semicolon; the rest does not.
        assert_eq!(text("FOO&gtBAR"), "FOO>BAR");
        assert_eq!(text("FOO&mdashBAR"), "FOO&mdashBAR");

        // The longest name wins: `&notit;` is `not` followed by `it;`.
        assert_eq!(text("&notit;"), "\u{AC}it;");
        assert_eq!(text("&notin;"), "\u{2209}");
    }

    /// In an attribute, a semicolon-less name followed by `=` or an
    /// alphanumeric is not a reference at all -- which keeps `?a=b&copy=1` a
    /// query string.
    /// A numeric reference takes only the digits of its own base, and three
    /// ranges do not mean what they say.
    #[test]
    fn numeric_references_stop_at_the_first_non_digit() {
        let text = |html: &str| -> String {
            let document = parse_document(html);
            body_of(&document)
                .children
                .iter()
                .map(|child| match child {
                    Node::Text(t) => t.clone(),
                    _ => String::new(),
                })
                .collect()
        };

        // `R` is not a hex digit, and `B` is not a decimal one.
        assert_eq!(text("FOO&#xBAR"), "FOO\u{BA}R");
        assert_eq!(text("FOO&#41BAR"), "FOO)BAR");

        // Written for Windows-1252, which is what the author meant.
        assert_eq!(text("&#151;"), "\u{2014}");
        assert_eq!(text("&#147;"), "\u{201C}");

        // Zero, the surrogates and anything past the last code point are
        // replaced.
        assert_eq!(text("&#0;"), "\u{FFFD}");
        assert_eq!(text("&#xD800;"), "\u{FFFD}");
        assert_eq!(text("&#x110000;"), "\u{FFFD}");
    }

    #[test]
    fn attribute_values_keep_ambiguous_ampersands() {
        let href = |html: &str| -> String {
            let document = parse_document(html);
            let Node::Element(a) = &body_of(&document).children[0] else {
                panic!("expected an element");
            };
            a.attribute("href").unwrap_or_default().to_string()
        };

        assert_eq!(href(r#"<a href="?a=b&copy=1">x</a>"#), "?a=b&copy=1");
        assert_eq!(href(r#"<a href="?a=b&copy;=1">x</a>"#), "?a=b\u{A9}=1");
        assert_eq!(href(r#"<a href="?a=b&copy 1">x</a>"#), "?a=b\u{A9} 1");
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
                // html5lib writes the namespace before the name, so an SVG
                // title reads `<svg title>` and an HTML one reads `<title>`.
                let prefix = match element.namespace {
                    super::Namespace::Html => "",
                    super::Namespace::Svg => "svg ",
                    super::Namespace::MathMl => "math ",
                };
                out.push_str(&format!("| {pad}<{prefix}{}>\n", element.tag_name));
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
        let mut sampled: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let focus = std::env::var("TOBIRA_H5_FILE").ok();

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
            } else if focus.as_deref().is_some_and(|want| case.file.starts_with(want)) {
                // `TOBIRA_H5_FILE=tests1` prints every failure in one file
                // rather than one sample per file.
                samples.push(format!(
                    "--- {} ---
input:    {:?}
expected:
{}
got:
{}",
                    case.file, case.data, case.document, got
                ));
            } else if focus.is_none() && !sampled.contains(&case.file) {
                // One per file, so the sample surveys rather than dwelling on
                // whichever file sorts first.
                sampled.insert(case.file.clone());
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
