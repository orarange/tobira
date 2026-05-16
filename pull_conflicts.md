--- HANDOFF.md ---
Conflict 1:
<<<<<<< HEAD
- Codex must stay on branch `codex/codex` unless the user explicitly changes that rule.
- Codex should use a dedicated worktree for `codex/codex` instead of sharing the user's main checkout.
- Keep Codex changes isolated to `codex/codex`; Claude may work on its own branch and merge reconciliation happens later through GitHub Copilot or the user's preferred flow.
- Update the `Current Snapshot` section whenever the high-level state changes.
- Append a short entry to `Session Log` whenever meaningful work is handed off or resumed.
=======
- Update the `Current Snapshot` section when the high-level state changes.
- Append a short entry to `Session Log` whenever you hand off or resume meaningful work.
- **Worktree layout** — Claude and Codex each have a dedicated worktree. Always work inside your own:
  - `browser-claude/`  → branch `claude/modest-pascal-9bf652`  (Claude's workspace)
  - `browser-codex/`   → branch `codex/codex`                  (Codex's workspace)
  - `browser/`         → branch `master`                        (canonical history)
  Never edit files in another agent's worktree. Merge reconciliation goes through GitHub Copilot / user-chosen flow.
>>>>>>> 4b2c68b0348c3b566f3e53bb79c21deee107c0b7

--- src/css.rs ---
Conflict 1:
<<<<<<< HEAD

=======
>>>>>>> 4b2c68b0348c3b566f3e53bb79c21deee107c0b7

Conflict 2:
<<<<<<< HEAD
}
=======
}
>>>>>>> 4b2c68b0348c3b566f3e53bb79c21deee107c0b7

--- src/layout.rs ---
Conflict 1:
<<<<<<< HEAD
<<<<<<< ours
=======
>>>>>>> 4b2c68b0348c3b566f3e53bb79c21deee107c0b7

Conflict 2:
<<<<<<< HEAD
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
>>>>>>> theirs

