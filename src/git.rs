use std::{
    collections::HashSet,
    ffi::OsStr,
    process::{Command, Output},
};

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
        .map(|(name, cmd)| git_command_status(name, cmd))
        .collect()
}

pub fn git_command_output<I, S>(name: &str, args: I) -> Result<Output, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    match Command::new(GIT).args(args).output() {
        Ok(output) => Ok(output),
        Err(err) => Err(format!("{} failed: {}", name, err)),
    }
}

pub fn git_current_branch() -> Result<String, String> {
    let Output { stdout, .. } = git_command_output(
        "get current branch",
        vec!["rev-parse", "--abbrev-ref", "HEAD"],
    )?;

    match String::from_utf8(stdout) {
        Ok(value) => Ok(String::from(value.trim())),
        Err(err) => return Err(format!("{}", err)),
    }
}

pub fn git_branches() -> Result<HashSet<String>, String> {
    let branches: HashSet<String> =
        git_command_lines("get branches", vec!["branch", "--format=%(refname:short)"])?
            .into_iter()
            .collect();
    Ok(branches)
}

pub fn git_command_lines<I, S>(name: &str, args: I) -> Result<Vec<String>, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let Output { stdout, stderr, .. } = git_command_output(name, args)?;

    let stderr = String::from_utf8(stderr).map_err(|e| format!("{e}"))?;
    let stdout = String::from_utf8(stdout).map_err(|e| format!("{e}"))?;

    let mut lines: Vec<String> = Vec::new();
    for l in stderr.lines() {
        lines.push(String::from(l));
    }
    for l in stdout.lines() {
        lines.push(String::from(l));
    }

    Ok(lines)
}
