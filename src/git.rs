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
