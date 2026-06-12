use std::process::Command;

/// Capture build-time provenance (git short hash, dirty flag, commit date) and
/// expose it to the crate via `env!` so the running binary can report exactly
/// which revision it was built from. This makes "is my patch actually in this
/// build?" answerable at a glance (window title / `--version`).
fn main() {
    let hash = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".to_string());

    // A non-empty `git status --porcelain` means uncommitted changes are baked
    // into this build; flag it so a dirty build is never mistaken for a commit.
    let dirty = git(&["status", "--porcelain"])
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let hash = if dirty { format!("{hash}-dirty") } else { hash };

    let commit_date =
        git(&["show", "-s", "--format=%cd", "--date=short", "HEAD"]).unwrap_or_default();

    println!("cargo:rustc-env=TOBIRA_GIT_HASH={hash}");
    println!("cargo:rustc-env=TOBIRA_COMMIT_DATE={commit_date}");

    // Rebuild the version string whenever HEAD (or the index, for the dirty
    // check) moves, so the embedded hash never goes stale.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
    if let Some(ref_path) = git(&["symbolic-ref", "-q", "HEAD"]) {
        let ref_path = ref_path.trim();
        if !ref_path.is_empty() {
            println!("cargo:rerun-if-changed=.git/{ref_path}");
        }
    }
}

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let trimmed = text.trim().to_string();
    if trimmed.is_empty() { None } else { Some(trimmed) }
}
