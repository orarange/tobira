import re

with open('src/gui.rs', 'r', encoding='utf-8') as f:
    content = f.read()

# Make the script flexible by removing the strict >>>>>>> origin/codex/codex check
# and replacing it with regex matching any >>>>>>> line
c1_pattern = re.compile(r"<<<<<<< HEAD\nuse crate::layout::\{DrawCommand, LayerCommand, LayoutDocument, TextCommand, layout_styled_document\};\n=======\nuse crate::js::\{DomEventDispatchResult, DomEventRequest\};\nuse crate::layout::\{\n    FormControlCommand, FormControlKind, LayoutDocument, TextCommand, layout_styled_document,\n\};\n>>>>>>> [a-f0-9]+", re.MULTILINE)
r1 = """use crate::js::{DomEventDispatchResult, DomEventRequest};
use crate::layout::{
    DrawCommand, FormControlCommand, FormControlKind, LayerCommand, LayoutDocument, TextCommand, layout_styled_document,
};"""
content = c1_pattern.sub(r1, content)

c2_pattern = re.compile(r"<<<<<<< HEAD\n            &mut self\.scratch,\n=======\n            &self\.page_control_values,\n            self\.focused_page_input\.as_ref\(\),\n            self\.hovered_target,\n>>>>>>> [a-f0-9]+", re.MULTILINE)
r2 = """            &self.page_control_values,
            self.focused_page_input.as_ref(),
            self.hovered_target,
            &mut self.scratch,"""
content = c2_pattern.sub(r2, content)

c3_pattern = re.compile(r"<<<<<<< HEAD\n    scratch: &mut Vec<Vec<u32>>,\n\) \{\n    let page = if let DocumentContent::Loaded\(page\) = &document\.content \{\n        Some\(page\)\n    \} else \{\n        None\n    \};\n\n    render_commands\(\n        buffer,\n        width,\n        height,\n        offset_x,\n        offset_y,\n        viewport_height,\n        scroll_y,\n        &layout\.commands,\n        page,\n        fonts,\n        scratch,\n        0,\n    \);\n\}\n\nfn render_commands\(\n    buffer: &mut \[u32\],\n    width: u32,\n    height: u32,\n    offset_x: u32,\n    offset_y: u32,\n    viewport_height: u32,\n    scroll_y: u32,\n    commands: &\[DrawCommand\],\n    page: Option<&crate::browser::BrowserPage>,\n    fonts: &mut FontContext,\n    scratch: &mut Vec<Vec<u32>>,\n    depth: usize,\n=======\n    page_control_values: &BTreeMap<usize, String>,\n    focused_page_input: Option<&FocusedPageInput>,\n    hovered_target: HitTarget,\n>>>>>>> [a-f0-9]+", re.MULTILINE)
r3 = """    page_control_values: &BTreeMap<usize, String>,
    focused_page_input: Option<&FocusedPageInput>,
    hovered_target: HitTarget,
    scratch: &mut Vec<Vec<u32>>,
) {
    let page = if let DocumentContent::Loaded(page) = &document.content {
        Some(page)
    } else {
        None
    };

    render_commands(
        buffer,
        width,
        height,
        offset_x,
        offset_y,
        viewport_height,
        scroll_y,
        &layout.commands,
        page,
        fonts,
        scratch,
        0,
    );

    let viewport_bottom = scroll_y.saturating_add(viewport_height);
    for control in &layout.controls {
        let control_bottom = control.y.saturating_add(control.height);
        if control_bottom < scroll_y || control.y > viewport_bottom {
            continue;
        }

        paint_page_control(
            fonts,
            buffer,
            width,
            height,
            offset_x,
            offset_y,
            scroll_y,
            control,
            page_control_values,
            focused_page_input,
            hovered_target,
        );
    }
}

fn render_commands(
    buffer: &mut [u32],
    width: u32,
    height: u32,
    offset_x: u32,
    offset_y: u32,
    viewport_height: u32,
    scroll_y: u32,
    commands: &[DrawCommand],
    page: Option<&crate::browser::BrowserPage>,
    fonts: &mut FontContext,
    scratch: &mut Vec<Vec<u32>>,
    depth: usize,"""
content = c3_pattern.sub(r3, content)

c4_pattern = re.compile(r"<<<<<<< HEAD\n    // Return offscreen Vec to the pool so its capacity is reused next frame\.\n    // Trim if dramatically over-sized: if the buffer is more than 4× larger than what\n    // this frame needed \(and wastes >1 MB\), release the excess so a previously-huge\n    // layer \(e\.g\. a tall scrollable body\) doesn't permanently inflate RAM after navigation\.\n    const MAX_OVERAGE_PIXELS: usize = 256 \* 1024; // 1 MB = 256K u32 pixels\n    if offscreen\.capacity\(\) > needed \* 4 && offscreen\.capacity\(\) - needed > MAX_OVERAGE_PIXELS \{\n        offscreen\.shrink_to\(needed \* 2\);\n    \}\n    scratch\[depth\] = offscreen;\n=======\n    for control in &layout\.controls \{\n        let control_bottom = control\.y\.saturating_add\(control\.height\);\n        if control_bottom < scroll_y \|\| control\.y > viewport_bottom \{\n            continue;\n        \}\n\n        paint_page_control\(\n            fonts,\n            buffer,\n            width,\n            height,\n            offset_x,\n            offset_y,\n            scroll_y,\n            control,\n            page_control_values,\n            focused_page_input,\n            hovered_target,\n        \);\n    \}\n>>>>>>> [a-f0-9]+", re.MULTILINE)
r4 = """    // Return offscreen Vec to the pool so its capacity is reused next frame.
    // Trim if dramatically over-sized: if the buffer is more than 4× larger than what
    // this frame needed (and wastes >1 MB), release the excess so a previously-huge
    // layer (e.g. a tall scrollable body) doesn't permanently inflate RAM after navigation.
    const MAX_OVERAGE_PIXELS: usize = 256 * 1024; // 1 MB = 256K u32 pixels
    if offscreen.capacity() > needed * 4 && offscreen.capacity() - needed > MAX_OVERAGE_PIXELS {
        offscreen.shrink_to(needed * 2);
    }
    scratch[depth] = offscreen;"""
content = c4_pattern.sub(r4, content)

with open('src/gui.rs', 'w', encoding='utf-8') as f:
    f.write(content)
print("Resolved gui.rs")
