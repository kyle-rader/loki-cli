use std::{
    ffi::OsStr,
    process::{Command, Output},
};

const GIT: &str = "git";

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
