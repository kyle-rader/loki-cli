use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use colored::Colorize;

use crate::git::{git_command_iter, git_command_status, git_commands_status};
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
/// branch with optional prefix. Outputs `cd <path>` to stdout for piping.
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

    eprintln!("Creating worktree at {}", wt_path_str.green());
    git_command_status(
        "worktree add",
        vec!["worktree", "add", wt_path_str.as_ref(), base],
    )?;

    // Set process cwd so branch creation runs inside the new worktree
    std::env::set_current_dir(&wt_path)
        .map_err(|e| format!("Failed to enter worktree directory: {e}"))?;

    if let Some(prefix) = prefix {
        eprintln!("Using branch prefix `{prefix}` (set via --prefix or {LOKI_NEW_PREFIX}).");
        name = format!("{prefix}{name}");
    }

    git_commands_status(vec![
        ("create branch", vec!["switch", "--create", name.as_str()]),
        (
            "push to origin",
            vec!["push", "--set-upstream", "origin", name.as_str()],
        ),
    ])?;

    eprintln!("\n{}", "Worktree ready!".green().bold());
    println!("cd {wt_path_str}");

    Ok(())
}

/// Removes a worktree and deletes its local branch. If `name` is empty the
/// worktree name is inferred from the current directory. Outputs `cd <main>`
/// to stdout for piping.
pub fn worktree_remove(name: &[String], force: bool, prefix: Option<&str>) -> Result<(), String> {
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

    if let Ok(cwd) = std::env::current_dir() {
        if cwd.starts_with(&wt_path) {
            eprintln!(
                "{} You are inside the worktree being removed.",
                "Warning:".yellow().bold(),
            );
        }
    }

    let mut remove_args = vec!["worktree", "remove"];
    if force {
        remove_args.push("--force");
    }
    remove_args.push(wt_path_str.as_ref());
    git_command_status("worktree remove", remove_args)?;
    eprintln!("Removed worktree {}", wt_path_str.red());

    // Best-effort branch cleanup — may already be gone
    let branch = match prefix {
        Some(p) => format!("{p}{name}"),
        None => name,
    };

    match git_command_status("delete branch", vec!["branch", "-D", branch.as_str()]) {
        Ok(()) => eprintln!("Deleted branch {}", branch.red()),
        Err(_) => eprintln!(
            "Branch {} not found locally (may already be deleted)",
            branch.yellow()
        ),
    }

    println!("cd {main_root}");
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

    println!("cd {target}");

    if std::io::stdout().is_terminal() {
        let example = if cfg!(windows) {
            "lk w s | iex"
        } else {
            "eval \"$(lk w s)\""
        };
        eprintln!("\n{}", format!("Tip: pipe to switch automatically: {example}").dimmed());
    }

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
