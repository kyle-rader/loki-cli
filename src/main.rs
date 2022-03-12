use std::{ffi::OsStr, process::Command};

use clap::Parser;

#[derive(Parser)]
#[clap(version, about, author)]
enum Cli {
    /// Create a new branch and push it to origin.
    #[clap(visible_alias = "n")]
    New { name: Vec<String> },
}

fn main() -> Result<(), String> {
    let cli = Cli::parse();

    match &cli {
        Cli::New { name } => new_branch(name)?,
    }
    Ok(())
}

fn new_branch(name: &Vec<String>) -> Result<(), String> {
    if name.len() == 0 {
        return Err(String::from("name cannot be empty."));
    }

    let name = name.join("-");

    git(
        "create new branch",
        vec!["switch", "--create", name.as_str()],
    )?;

    git(
        "push to origin",
        vec!["push", "--set-upstream", "origin", name.as_str()],
    )?;

    Ok(())
}

fn git<I, S>(name: &str, args: I) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    if let Some(error) = Command::new("git").args(args).status().err() {
        return Err(format!("{} failed to run: {}", name, error));
    }
    Ok(())
}
