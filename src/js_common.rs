/// Shared JS output types used by both js.rs (boa) and js_host.rs (new engine).
/// This module exists to avoid a circular dependency between js.rs and js_host.rs.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProcessedScriptHtml {
    pub html: String,
    pub title_override: Option<String>,
    pub console_logs: Vec<String>,
    pub navigation_target: Option<String>,
    pub soft_navigation_target: Option<String>,
    pub scroll_y: u32,
}
