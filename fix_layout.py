import re

with open('src/layout.rs', 'r', encoding='utf-8') as f:
    content = f.read()

def replace_conflict(content, search_head, search_codex, replacement):
    parts = []
    lines = content.splitlines(True)
    in_conflict = False
    head_lines = []
    codex_lines = []
    current_part = []
    
    for line in lines:
        if line.startswith('<<<<<<< HEAD'):
            in_conflict = True
            parts.append("".join(current_part))
            current_part = []
            head_lines = []
            codex_lines = []
            stage = 'head'
        elif line.startswith('======='):
            if in_conflict:
                stage = 'codex'
        elif line.startswith('>>>>>>>'):
            if in_conflict:
                in_conflict = False
                head_str = "".join(head_lines)
                codex_str = "".join(codex_lines)
                
                if search_head.strip() in head_str and search_codex.strip() in codex_str:
                    parts.append(replacement)
                else:
                    parts.append("<<<<<<< HEAD\n")
                    parts.append(head_str)
                    parts.append("=======\n")
                    parts.append(codex_str)
                    parts.append(line)
        else:
            if in_conflict:
                if stage == 'head':
                    head_lines.append(line)
                else:
                    codex_lines.append(line)
            else:
                current_part.append(line)
                
    parts.append("".join(current_part))
    return "".join(parts)

# Conflict 1
c1_head = "fn extract_body_background(node: &StyledNode) -> Option<u32>"
c1_codex = "pub enum FormControlKind"
r1 = """/// Scan the document tree for a body/html element with a solid background color.
/// Used to fill canvas margins without double-applying opacity.
fn extract_body_background(node: &StyledNode) -> Option<u32> {
    if let StyledNode::Element(el) = node {
        if (el.tag_name == "body" || el.tag_name == "html") && el.style.opacity == 255 {
            if let Some(bg) = el.style.background_color {
                return Some(bg);
            }
        }
        for child in &el.children {
            if let StyledNode::Element(child_el) = child {
                if (child_el.tag_name == "body" || child_el.tag_name == "html")
                    && child_el.style.opacity == 255
                {
                    if let Some(bg) = child_el.style.background_color {
                        return Some(bg);
                    }
                }
            }
        }
        for child in &el.children {
            if let Some(bg) = extract_body_background(child) {
                return Some(bg);
            }
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormControlKind {
    TextInput,
    Button,
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormControlCommand {
    pub id: usize,
    pub node_id: Option<usize>,
    pub form_node_id: Option<usize>,
    pub kind: FormControlKind,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub name: Option<String>,
    pub value: String,
    pub label: String,
    pub placeholder: Option<String>,
    pub form_id: Option<usize>,
    pub form_action: Option<String>,
    pub form_method: String,
    pub activates_submit: bool,
    pub disabled: bool,
    pub masked: bool,
    pub font_size_px: u32,
    pub font_family: FontFamilyKind,
    pub text_color: Color,
    pub background_color: Color,
    pub border_color: Color,
}
"""
content = replace_conflict(content, c1_head, c1_codex, r1)

# Conflict 2
c2_head = "    },"
c2_codex = "        link_node_id: Option<usize>,\n    },\n    Control(FormControlSpec),"
r2 = """        link_node_id: Option<usize>,
    },
    Control(FormControlSpec),
"""
content = replace_conflict(content, c2_head, c2_codex, r2)

# Conflict 3
c3_head = "style: ComputedStyle,"
c3_codex = "link_node_id: Option<usize>,"
r3 = """    style: ComputedStyle,
    link_href: Option<String>,
    link_node_id: Option<usize>,
"""
content = replace_conflict(content, c3_head, c3_codex, r3)

# Conflict 4
c4_head = "layout_table_element(element, x, width, cursor_y, &mut sub_context, images, fonts);"
c4_codex = "layout_table_element("
r4 = """        if element.style.opacity < 255 {
            // Table with opacity: render into sub-context and wrap in a LayerCommand
            let mut sub_context = LayoutContext {
                background_color: context.background_color,
                ..LayoutContext::default()
            };
            sub_context.next_control_id = context.next_control_id;
            sub_context.next_form_id = context.next_form_id;

            let y_before = *cursor_y;
            layout_table_element(element, x, width, cursor_y, &mut sub_context, images, fonts, current_form.clone());
            let table_height = cursor_y.saturating_sub(y_before).max(1);
            rebase_commands(&mut sub_context.commands, x, y_before);
            context.commands.push(DrawCommand::Layer(LayerCommand {
                x,
                y: y_before,
                width: width.max(1),
                height: table_height,
                opacity: element.style.opacity,
                commands: sub_context.commands,
            }));
            context.links.extend(sub_context.links);
            context.controls.extend(sub_context.controls);
            context.next_control_id = sub_context.next_control_id;
            context.next_form_id = sub_context.next_form_id;
        } else {
            layout_table_element(element, x, width, cursor_y, context, images, fonts, current_form);
        }
"""
content = replace_conflict(content, c4_head, c4_codex, r4)

# Conflict 5
c5_head = "sub_context.commands.push(DrawCommand::Rect(RectCommand {"
c5_codex = "context.rects.push(RectCommand {"
r5 = """            sub_context.commands.push(DrawCommand::Rect(RectCommand {"""
content = replace_conflict(content, c5_head, c5_codex, r5)

# Conflict 6
c6_head = "let cell_layouts = placements"
c6_codex = "let mut cell_layouts = Vec::with_capacity(placements.len());"
r6 = """    let mut cell_layouts = Vec::with_capacity(placements.len());
    let mut next_control_id = context.next_control_id;
    let mut next_form_id = context.next_form_id;
    for placement in &placements {
        let span_width = span_width(&column_widths, placement.column_index, placement.colspan)
            .saturating_add(spacing.saturating_mul(placement.colspan.saturating_sub(1) as u32));
        let inner_width = span_width.saturating_sub(padding.saturating_mul(2)).max(1);
        let cell_backdrop = placement.cell.style.background_color
            .unwrap_or(context.background_color);
        let layout = layout_table_cell(
            placement.cell,
            inner_width,
            images,
            fonts,
            cell_backdrop,
            current_form.clone(),
            next_control_id,
            next_form_id,
        );
        next_control_id = layout.next_control_id;
        next_form_id = layout.next_form_id;
        cell_layouts.push(layout);
    }
"""
content = replace_conflict(content, c6_head, c6_codex, r6)

# Conflict 7
c7_head = "background_color: Color,"
c7_codex = "current_form: Option<FormContext>,"
r7 = """    background_color: Color,
    current_form: Option<FormContext>,
    control_id_seed: usize,
    form_id_seed: usize,
) -> FragmentLayout {
    let mut context = LayoutContext {
        background_color,
        commands: Vec::new(),
        links: Vec::new(),
        controls: Vec::new(),
        next_control_id: control_id_seed,
        next_form_id: form_id_seed,
    };
"""
content = replace_conflict(content, c7_head, c7_codex, r7)

# Conflict 8
c8_head = "DrawCommand::Layer(layer) => DrawCommand::Layer(LayerCommand {"
c8_codex = "context\n        .links"
r8 = """        }),
        DrawCommand::Layer(layer) => DrawCommand::Layer(LayerCommand {
            x: layer.x.saturating_add(offset_x),
            y: layer.y.saturating_add(offset_y),
            width: layer.width,
            height: layer.height,
            opacity: layer.opacity,
            commands: layer.commands.clone(),
        }),
    }
    context
        .links
        .extend(fragment.links.iter().map(|link| LinkCommand {
            node_id: link.node_id,
            x: link.x.saturating_add(offset_x),
            y: link.y.saturating_add(offset_y),
            width: link.width,
            height: link.height,
            href: link.href.clone(),
        }));
    context
        .controls
        .extend(fragment.controls.iter().map(|control| FormControlCommand {
            id: control.id,
            node_id: control.node_id,
            form_node_id: control.form_node_id,
            kind: control.kind,
            x: control.x.saturating_add(offset_x),
            y: control.y.saturating_add(offset_y),
            width: control.width,
            height: control.height,
            name: control.name.clone(),
            value: control.value.clone(),
            label: control.label.clone(),
            placeholder: control.placeholder.clone(),
            form_id: control.form_id,
            form_action: control.form_action.clone(),
            form_method: control.form_method.clone(),
            activates_submit: control.activates_submit,
            disabled: control.disabled,
            masked: control.masked,
            font_size_px: control.font_size_px,
            font_family: control.font_family,
            text_color: control.text_color,
            background_color: control.background_color,
            border_color: control.border_color,
        }));
"""
content = replace_conflict(content, c8_head, c8_codex, r8)

# Conflict 9
c9_head = "context: &mut LayoutContext," # This one might be empty on HEAD
c9_codex = "link_node_id: Option<usize>,"
r9 = """    link_node_id: Option<usize>,
    current_form: Option<FormContext>,
    context: &mut LayoutContext,
"""
content = replace_conflict(content, c9_head, c9_codex, r9)

# Conflict 10
c10_head = "" # Actually head is empty
c10_codex = "InlineFragment::Control(control) => {"
r10 = """            InlineFragment::Control(control) => {
                let (control_width, _) = measure_form_control(control, fonts);
                let effective_width = if first_line && line.is_empty() {
                    width.saturating_sub(text_indent)
                } else {
                    width
                };

                let pending_space_before_control = pending_space && !line.is_empty();
                if pending_space_before_control {
                    let space_width = char_width(&control.style, ' ', fonts);
                    if line.width.saturating_add(space_width) > effective_width {
                        emit_line_with_indent(
                            &mut line,
                            container_style,
                            x,
                            width,
                            cursor_y,
                            context,
                            fonts,
                            if first_line { text_indent } else { 0 },
                        );
                        first_line = false;
                    } else {
                        line.push_span(" ", &control.style, fonts, None, None);
                    }
                }

                let effective_width = if first_line && line.is_empty() {
                    width.saturating_sub(text_indent)
                } else {
                    width
                };
                if !line.is_empty() && line.width.saturating_add(control_width) > effective_width {
                    emit_line_with_indent(
                        &mut line,
                        container_style,
                        x,
                        width,
                        cursor_y,
                        context,
                        fonts,
                        if first_line { text_indent } else { 0 },
                    );
                    first_line = false;
                }
                line.push_control(control, fonts);
                pending_space = true;
            }
"""
content = replace_conflict(content, "", c10_codex, r10) # Using empty string for search_head if it's empty

# Conflict 11
c11_head = ""
c11_codex = "link_node_id,"
r11 = """                link_node_id,"""
content = replace_conflict(content, "", c11_codex, r11)

# Conflict 12
c12_head = ""
c12_codex = "InlineFragment::Control(control) => {"
r12 = """            InlineFragment::Control(control) => {
                let (control_width, _) = measure_form_control(control, fonts);
                if !line.is_empty() && line.width.saturating_add(control_width) > width {
                    emit_line(
                        &mut line,
                        container_style,
                        x,
                        width,
                        cursor_y,
                        context,
                        fonts,
                    );
                }
                line.push_control(control, fonts);
            }
"""
content = replace_conflict(content, "", c12_codex, r12)

# Conflict 13
c13_head = ""
c13_codex = "link_node_id,"
r13 = """                link_node_id,"""
content = replace_conflict(content, "", c13_codex, r13)

# Conflict 14
c14_head = ""
c14_codex = "*link_node_id,"
r14 = """                        *link_node_id,"""
content = replace_conflict(content, "", c14_codex, r14)

# Conflict 15
c15_head = "let span_opacity = span.style.effective_opacity;"
c15_codex = "if let Some(control) = &span.control {"
r15 = """        if let Some(control) = &span.control {
            let control_y = cursor_y.saturating_add(line_height.saturating_sub(span.height) / 2);
            let background_color = if control.disabled {
                0xE4E6EA
            } else if matches!(control.kind, FormControlKind::Button) {
                0xE7EBF2
            } else {
                0xFFFFFF
            };
            let border_color = if control.disabled { 0xA9AFB8 } else { 0x7F8B9C };
            context.controls.push(FormControlCommand {
                id: control.id,
                node_id: control.node_id,
                form_node_id: control.form_node_id,
                kind: control.kind,
                x: cursor_x,
                y: control_y,
                width: span.width.max(1),
                height: span.height.max(1),
                name: control.name.clone(),
                value: control.value.clone(),
                label: control.label.clone(),
                placeholder: control.placeholder.clone(),
                form_id: control.form_id,
                form_action: control.form_action.clone(),
                form_method: control.form_method.clone(),
                activates_submit: control.activates_submit,
                disabled: control.disabled,
                masked: control.masked,
                font_size_px: span.style.font_size_px,
                font_family: span.style.font_family,
                text_color: span.style.color,
                background_color,
                border_color,
            });

            cursor_x = cursor_x.saturating_add(span.width);
            continue;
        }

        let span_opacity = span.style.effective_opacity;
        // Note: apply_opacity here blends span colors against context.background_color,
        // which tracks the nearest solid block-level backdrop. For spans inside a block
        // with opacity < 1, the block emits a LayerCommand and effective_opacity is reset
        // to 255 (via compute_style stacking-context rule), so blending is correct.
        // For a bare inline <span style="opacity:0.5"> with no surrounding stacking-context
        // block, effective_opacity accumulates multiplicatively and blending is done against
        // the block-level backdrop — ignoring any inline content painted underneath.
        // This is an intentional approximation (see css.rs nested_inline_opacity test).
"""
content = replace_conflict(content, c15_head, c15_codex, r15)

# Conflict 16
c16_head = "fn uses_document_background_for_opacity_blending() {"
c16_codex = "fn emits_form_controls_for_inputs_and_buttons() {"
r16 = """    fn uses_document_background_for_opacity_blending() {
        let document = parse_document("<body><div>Hi</div></body>");
        let stylesheet =
            parse_stylesheet("body { background-color: #000000; } div { background-color: #ff0000; opacity: 0.5; }");
        let styled = build_styled_tree(&document, &stylesheet, 1280);
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, &ImageStore::default(), 320, &mut fonts);

        // With stacking contexts, the div with opacity: 0.5 becomes a LayerCommand.
        // Its background rect inside the layer uses the raw red (#ff0000), not a pre-blended value.
        // The compositor blends it at render time.
        let has_layer = layout.commands.iter().any(|cmd| {
            matches!(cmd, DrawCommand::Layer(layer) if layer.opacity == 128 || layer.opacity == 127)
        });
        assert!(
            has_layer,
            "div with opacity: 0.5 should produce a LayerCommand with ~50% opacity"
        );
        // The raw red rect should be inside the layer
        let has_raw_red = layout.commands.iter().any(|cmd| {
            if let DrawCommand::Layer(layer) = cmd {
                layer.commands.iter().any(|inner| {
                    matches!(inner, DrawCommand::Rect(r) if r.color == 0xFF0000)
                })
            } else {
                false
            }
        });
        assert!(
            has_raw_red,
            "raw red rect should be inside the LayerCommand"
        );
    }

    #[test]
    fn accumulates_parent_opacity_for_text() {
        let document = parse_document("<body><div><span>Hi</span></div></body>");
        let stylesheet = parse_stylesheet(
            "body { background-color: #000000; } div { opacity: 0.5; } span { opacity: 0.5; color: #ffffff; }",
        );
        let styled = build_styled_tree(&document, &stylesheet, 1280);
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, &ImageStore::default(), 320, &mut fonts);

        // With proper stacking contexts, div creates a LayerCommand with its own opacity.
        // The span inside has its own effective_opacity (reset at stacking context boundary).
        // The text color inside the layer is pre-blended with the span's own opacity (0.5)
        // against the layer's local backdrop (black #000000 from body background).
        // span.opacity=0.5=128, color=white=#ffffff blended against black => ~0x808080
        let has_layer = layout.commands.iter().any(|cmd| {
            matches!(cmd, DrawCommand::Layer(_))
        });
        assert!(has_layer, "div with opacity: 0.5 should produce a LayerCommand");

        // Text color inside layer should be pre-blended with span's own opacity against the
        // layer's local backdrop color. The layer's backdrop is black (body bg).
        // span effective_opacity = 128 (its own opacity, reset at stacking context boundary)
        // color = apply_opacity(0xFFFFFF, 0x000000, 128) = ~0x808080
        let texts = layout.texts();
        let text = texts.first().expect("text command should exist");
        // The text should be blended with span's 50% opacity against the layer backdrop (black)
        assert_eq!(text.color, 0x808080,
            "text inside stacking context should be pre-blended with span's own opacity against layer backdrop");
    }

    #[test]
    fn emits_form_controls_for_inputs_and_buttons() {
        let document = parse_document(
            "<form action=\"/search\"><input name=\"q\" value=\"rust\"><button type=\"submit\">Go</button></form>",
        );
        let styled = build_styled_tree(&document, &parse_stylesheet(""), 1280);
        let mut fonts = FontContext::load();
        let layout = layout_styled_document(&styled, &ImageStore::default(), 320, &mut fonts);

        assert_eq!(layout.controls.len(), 2);
        assert!(
            layout
                .controls
                .iter()
                .any(|control| control.kind == super::FormControlKind::TextInput
                    && control.name.as_deref() == Some("q"))
        );
        assert!(
            layout
                .controls
                .iter()
                .any(|control| control.kind == super::FormControlKind::Button
                    && control.label == "Go")
        );
    }
"""
content = replace_conflict(content, c16_head, c16_codex, r16)

with open('src/layout.rs', 'w', encoding='utf-8') as f:
    f.write(content)
print("Applied all replacements to layout.rs")
