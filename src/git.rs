use std::{collections::HashSet, ffi::OsStr, process::Command};

const GIT: &str = "git";

/// Execute the git command returning an error if it fails. No redirection is done.
pub fn git_command_status<I, S>(name: &str, args: I) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    if let Some(error) = Command::new(GIT).args(args).status().err() {
        return Err(format!("{} failed to run: {}", name, error));
    }
    Ok(())
}

/// Execute the list of git commands in order, returning on the first failure. No redirection is done.
pub fn git_commands_status<C, I, S>(commands: C) -> Result<(), String>
where
    C: IntoIterator<Item = (&'static str, I)>,
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    commands
        .into_iter()
        .try_for_each(|(name, cmd)| git_command_status(name, cmd))
}

/// Execute a git command and return an iterator over its output lines (both stdout and stderr).
pub fn git_command_iter<I, S>(name: &str, args: I) -> Result<impl Iterator<Item = String>, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new(GIT)
        .args(args)
        .output()
        .map_err(|err| format!("{} failed: {}", name, err))?;

    let stderr = String::from_utf8(output.stderr).map_err(|e| format!("{e}"))?;
    let stdout = String::from_utf8(output.stdout).map_err(|e| format!("{e}"))?;

    // Combine stderr and stdout lines into a single iterator
    let lines = stderr
        .lines()
        .chain(stdout.lines())
        .map(String::from)
        .collect::<Vec<_>>()
        .into_iter();

    Ok(lines)
}

pub fn git_current_branch() -> Result<String, String> {
    let mut lines = git_command_iter(
        "get current branch",
        vec!["rev-parse", "--abbrev-ref", "HEAD"],
    )?;

    lines
        .next()
        .map(|line| line.trim().to_string())
        .ok_or_else(|| "No output from git rev-parse".to_string())
}

pub fn git_branches() -> Result<HashSet<String>, String> {
    let branches: HashSet<String> =
        git_command_iter("get branches", vec!["branch", "--format=%(refname:short)"])?.collect();
    Ok(branches)
}

pub fn git_command_lines<I, S>(name: &str, args: I) -> Result<Vec<String>, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Ok(git_command_iter(name, args)?.collect())
}
