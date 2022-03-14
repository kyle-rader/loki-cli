pub mod pruning;

use std::{
    collections::HashSet,
    ffi::OsStr,
    io::{BufRead, BufReader},
    process::{Command, Output, Stdio},
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

pub fn git_command_process_lines<I, S>(
    name: &str,
    args: I,
    process_line: fn(&String),
) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = Command::new(GIT)
        .args(args)
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{e}"))?;

    if let Some(stdout) = cmd.stdout.as_mut() {
        let lines = BufReader::new(stdout).lines();
        for line in lines {
            match line {
                Ok(line) => {
                    process_line(&line);
                }
                Err(e) => eprintln!("{e}"),
            }
        }
    }

    cmd.wait()
        .map_err(|e| format!("{e}"))?
        .success()
        .then(|| ())
        .ok_or(format!("{name} failed to execute"))
}
