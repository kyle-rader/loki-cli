use std::{ffi::OsStr, process::Command};

pub fn git_command<I, S>(name: &str, args: I) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    if let Some(error) = Command::new("git").args(args).status().err() {
        return Err(format!("{} failed to run: {}", name, error));
    }
    Ok(())
}

pub fn git_commands<C, I, S>(commands: C) -> Result<(), String>
where
    C: IntoIterator<Item = (&'static str, I)>,
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    commands
        .into_iter()
        .map(|(name, cmd)| git_command(name, cmd))
        .collect()
}
