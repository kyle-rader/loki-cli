use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use colored::Colorize;

use crate::git::{git_command_iter, git_command_lines, git_command_status_quiet};
use crate::vars::LOKI_NEW_PREFIX;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extracts the worktree name from a `<repo>_<name>` directory name.
/// Returns everything after the first `_`, or the full string if none.
pub fn infer_worktree_name(dir_name: &str) -> &str {
    match dir_name.find('_') {
        Some(ix) => &dir_name[ix + 1..],
        None => dir_name,
    }
}

/// Returns the main (first) worktree path via `git worktree list --porcelain`.
fn resolve_main_worktree() -> Result<String, String> {
    git_command_iter("list worktrees", vec!["worktree", "list", "--porcelain"])?
        .find_map(|line| line.strip_prefix("worktree ").map(|s| s.to_string()))
        .ok_or_else(|| String::from("Could not determine main worktree from git worktree list"))
}

/// Builds a sibling worktree path: `<parent>/<repo_name>_<name>`.
fn worktree_path(repo_root: &str, name: &str) -> Result<PathBuf, String> {
    let root = Path::new(repo_root);
    let repo_name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .ok_or_else(|| format!("Could not determine repo name from path: {repo_root}"))?;
    let parent = root
        .parent()
        .ok_or_else(|| format!("Could not determine parent directory of: {repo_root}"))?;
    Ok(parent.join(format!("{repo_name}_{name}")))
}

/// Finds a worktree whose directory ends with `_<name>` or equals `<name>`.
fn resolve_worktree_by_name(name: &str) -> Result<String, String> {
    let suffix = format!("_{name}");

    for line in git_command_iter("list worktrees", vec!["worktree", "list", "--porcelain"])? {
        if let Some(path) = line.strip_prefix("worktree ") {
            let dir = Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if dir.ends_with(&suffix) || dir == name {
                return Ok(path.to_string());
            }
        }
    }

    Err(format!("No worktree found matching '{name}'"))
}

/// Normalizes path separators to forward slashes for cross-platform comparison.
fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

/// Outputs `cd <path>` to stdout. When stdout is a terminal (not piped),
/// prints a platform-appropriate tip for piping.
fn emit_cd(path: &str) {
    println!("cd {path}");
    if std::io::stdout().is_terminal() {
        let hint = if cfg!(windows) { "| iex" } else { "through eval" };
        eprintln!(
            "\n{}",
            format!("Tip: pipe this command {hint} to switch automatically.").dimmed()
        );
    }
}

/// Checks if a ref matches an existing remote branch on origin.
/// Returns the full remote ref (e.g. `origin/branch-name`) if found.
fn find_remote_branch(name: &str) -> Option<String> {
    // Check common forms: bare name, origin/name, or full ref
    let candidates = if name.starts_with("origin/") {
        vec![name.to_string()]
    } else if name.starts_with("refs/") {
        vec![name.strip_prefix("refs/heads/").unwrap_or(name).to_string()]
    } else {
        vec![name.to_string()]
    };

    for candidate in &candidates {
        let lines = git_command_lines(
            "ls-remote",
            vec!["ls-remote", "--heads", "origin", candidate.as_str()],
        )
        .unwrap_or_default();
        if !lines.is_empty() {
            return Some(format!("origin/{candidate}"));
        }
    }
    None
}

/// A parsed worktree entry from porcelain output.
struct WorktreeEntry {
    path: String,
    name: String,
    branch: Option<String>,
}

impl WorktreeEntry {
    fn display_label(&self) -> String {
        match &self.branch {
            Some(b) => format!("{} [{b}]", self.name),
            None => self.name.clone(),
        }
    }
}

/// Parses `git worktree list --porcelain` into structured entries.
fn list_worktree_entries() -> Result<Vec<WorktreeEntry>, String> {
    let mut entries = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_branch: Option<String> = None;

    let mut flush = |path: String, branch: Option<String>| {
        let dir = Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let name = infer_worktree_name(&dir).to_string();
        entries.push(WorktreeEntry {
            path,
            name,
            branch,
        });
    };

    for line in git_command_iter("worktree list", vec!["worktree", "list", "--porcelain"])? {
        if let Some(path) = line.strip_prefix("worktree ") {
            current_path = Some(path.to_string());
        } else if let Some(branch) = line.strip_prefix("branch refs/heads/") {
            current_branch = Some(branch.to_string());
        } else if line.is_empty() {
            if let Some(path) = current_path.take() {
                flush(path, current_branch.take());
            }
            current_branch = None;
        }
    }
    if let Some(path) = current_path.take() {
        flush(path, current_branch.take());
    }

    Ok(entries)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Creates a worktree at `<parent>/<repo>_<name>`, then creates and pushes a
/// branch with optional prefix. If the base ref is an existing remote branch,
/// checks it out directly instead. Outputs `cd <path>` to stdout for piping.
pub fn worktree_add(name: &[String], base: &str, prefix: Option<&str>) -> Result<(), String> {
    if name.is_empty() {
        return Err(String::from("name cannot be empty."));
    }

    let mut name = name.join("-");
    let main_root = resolve_main_worktree()?;
    let wt_path = worktree_path(&main_root, &name)?;
    let wt_path_str = wt_path.to_string_lossy();

    if wt_path.exists() {
        return Err(format!("Worktree path already exists: {wt_path_str}"));
    }

    // Check if the base is an existing remote branch to check out directly
    if let Some(remote_ref) = find_remote_branch(base) {
        eprintln!(
            "Found existing branch {} — checking out into worktree",
            base.cyan()
        );
        git_command_status_quiet("fetch", vec!["fetch", "origin"])?;
        git_command_status_quiet(
            "worktree add",
            vec![
                "worktree",
                "add",
                "--track",
                "-b",
                base,
                wt_path_str.as_ref(),
                remote_ref.as_str(),
            ],
        )?;

        eprintln!("\n{}", "Worktree ready!".green().bold());
        emit_cd(&wt_path_str);
        return Ok(());
    }

    // New branch flow
    eprintln!("Creating worktree at {}", wt_path_str.green());
    git_command_status_quiet(
        "worktree add",
        vec!["worktree", "add", wt_path_str.as_ref(), base],
    )?;

    std::env::set_current_dir(&wt_path)
        .map_err(|e| format!("Failed to enter worktree directory: {e}"))?;

    if let Some(prefix) = prefix {
        eprintln!("Using branch prefix `{prefix}` (set via --prefix or {LOKI_NEW_PREFIX}).");
        name = format!("{prefix}{name}");
    }

    git_command_status_quiet("create branch", vec!["switch", "--create", name.as_str()])?;
    git_command_status_quiet(
        "push to origin",
        vec!["push", "--set-upstream", "origin", name.as_str()],
    )?;

    eprintln!("\n{}", "Worktree ready!".green().bold());
    emit_cd(&wt_path_str);

    Ok(())
}

/// Removes a worktree and deletes its local branch. If `name` is empty the
/// worktree name is inferred from the current directory. Outputs `cd <main>`
/// to stdout for piping.
pub fn worktree_remove(name: &[String], force: bool) -> Result<(), String> {
    let main_root = resolve_main_worktree()?;

    let name = if name.is_empty() {
        let cwd = std::env::current_dir()
            .map_err(|e| format!("Failed to get current directory: {e}"))?;
        let dir_name = cwd
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let inferred = infer_worktree_name(&dir_name).to_string();
        eprintln!("Inferred worktree name: {}", inferred.cyan());
        inferred
    } else {
        name.join("-")
    };

    let wt_path = worktree_path(&main_root, &name)?;

    // Fall back to plain name if the <repo>_<name> path doesn't exist
    let wt_path = if wt_path.exists() {
        wt_path
    } else {
        let parent = wt_path
            .parent()
            .ok_or_else(|| format!("Could not determine parent of: {}", wt_path.to_string_lossy()))?;
        let fallback = parent.join(&name);
        if !fallback.exists() {
            return Err(format!(
                "Worktree directory not found at {} or {}",
                wt_path.to_string_lossy(),
                fallback.to_string_lossy()
            ));
        }
        fallback
    };
    let wt_path_str = wt_path.to_string_lossy();

    // Don't allow removing the main worktree
    if normalize_path(&wt_path_str) == normalize_path(&main_root) {
        return Err(String::from(
            "You're in the main repo - only secondary worktrees can be removed.",
        ));
    }

    // Look up the actual branch checked out in this worktree before removing
    let actual_branch = list_worktree_entries()
        .ok()
        .and_then(|entries| {
            let normalized_target = normalize_path(&wt_path_str);
            entries
                .into_iter()
                .find(|e| normalize_path(&e.path) == normalized_target)
                .and_then(|e| e.branch)
        });

    // Move out of the worktree so the OS can delete it
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.starts_with(&wt_path) {
            eprintln!(
                "Leaving worktree directory, moving to {}",
                main_root.green()
            );
            std::env::set_current_dir(&main_root)
                .map_err(|e| format!("Failed to change to main worktree: {e}"))?;
        }
    }

    // Attempt worktree removal — retry with --force on dirty worktree errors
    let mut remove_args = vec!["worktree", "remove"];
    if force {
        remove_args.push("--force");
    }
    remove_args.push(wt_path_str.as_ref());

    if let Err(err) = git_command_status_quiet("worktree remove", remove_args) {
        if !force && (err.contains("modified or untracked") || err.contains("contains modified")) {
            return Err(format!(
                "Worktree has uncommitted changes. Run with --force to remove anyway:\n  lk w r --force {}",
                name
            ));
        }
        return Err(err);
    }
    eprintln!("Removed worktree {}", wt_path_str.red());

    // Prune stale worktree refs so branch deletion succeeds
    let _ = git_command_status_quiet("worktree prune", vec!["worktree", "prune"]);

    // Best-effort branch cleanup using the actual checked-out branch
    if let Some(branch) = actual_branch {
        match git_command_status_quiet("delete branch", vec!["branch", "-D", branch.as_str()]) {
            Ok(()) => eprintln!("Deleted branch {}", branch.red()),
            Err(_) => eprintln!(
                "Branch {} not found locally (may already be deleted)",
                branch.yellow()
            ),
        }
    }

    emit_cd(&main_root);
    Ok(())
}

/// Outputs `cd <path>` for the named worktree, or the main worktree if no
/// name is given. Designed for `eval` / `Invoke-Expression` piping.
pub fn worktree_switch(name: &[String]) -> Result<(), String> {
    let target = if name.is_empty() {
        resolve_main_worktree()?
    } else {
        resolve_worktree_by_name(&name.join("-"))?
    };

    emit_cd(&target);
    Ok(())
}
/// Lists all worktrees, highlighting the current one and showing switch hints.
pub fn worktree_list() -> Result<(), String> {
    let cwd = std::env::current_dir()
        .ok()
        .map(|p| normalize_path(&p.to_string_lossy()));

    let entries = list_worktree_entries()?;

    for entry in &entries {
        let is_current = cwd.as_ref().is_some_and(|c| {
            let normalized = normalize_path(&entry.path);
            *c == normalized || c.starts_with(&format!("{normalized}/"))
        });

        let label = entry.display_label();
        if is_current {
            println!("{}", format!("* {label}").green().bold());
        } else {
            let hint = switch_hint(&entry.name).dimmed();
            println!("  {label}  {hint}");
        }
    }

    Ok(())
}

/// Returns the platform-appropriate command to switch to a worktree.
fn switch_hint(name: &str) -> String {
    if cfg!(windows) {
        format!("lk w s {name} | iex")
    } else {
        format!("eval \"$(lk w s {name})\"")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infer_name_with_underscore() {
        assert_eq!(infer_worktree_name("my-project_fix-auth"), "fix-auth");
    }

    #[test]
    fn infer_name_with_multiple_underscores() {
        assert_eq!(
            infer_worktree_name("my_project_fix-auth"),
            "project_fix-auth"
        );
    }

    #[test]
    fn infer_name_without_underscore() {
        assert_eq!(infer_worktree_name("standalone"), "standalone");
    }

    #[test]
    fn infer_name_empty() {
        assert_eq!(infer_worktree_name(""), "");
    }

    #[test]
    fn worktree_path_basic() {
        let root = Path::new("repos").join("my-project");
        let path = worktree_path(root.to_str().unwrap(), "fix-auth").unwrap();
        let expected = Path::new("repos").join("my-project_fix-auth");
        assert_eq!(path, expected);
    }

    #[test]
    fn worktree_path_errors_on_bare_root() {
        // A bare root like "/" or "C:\" has no file_name component
        let result = worktree_path("/", "fix-auth");
        assert!(result.is_err());
    }

    #[test]
    fn normalize_path_converts_backslashes() {
        assert_eq!(normalize_path(r"C:\repos\my-project"), "C:/repos/my-project");
    }

    #[test]
    fn normalize_path_preserves_forward_slashes() {
        assert_eq!(normalize_path("/home/user/repos"), "/home/user/repos");
    }
}
