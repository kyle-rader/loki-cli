pub mod git;
pub mod pruning;

use std::process::Output;

use clap::Parser;
use git::{git_branches, git_command_output, git_command_status};

#[derive(Parser)]
#[clap(version, about, author)]
enum Cli {
    /// Create a new branch from HEAD and push it to origin.
    #[clap(visible_alias = "n")]
    New {
        /// List of names to join with dashes to form a valid branch name.
        name: Vec<String>,
    },

    /// Push the current branch to origin with --set-upstream
    Push {
        /// Use --force-with-lease
        #[clap(short, long)]
        force: bool,
    },

    /// Pull with --prune deleting local branches pruned from the remote.
    Pull,
    /// Fetch with --prune deleting local branches pruned from the remote.
    Fetch,
}

fn main() -> Result<(), String> {
    let cli = Cli::parse();

    match &cli {
        Cli::New { name } => new_branch(name),
        Cli::Push { force } => push_branch(*force),
        Cli::Pull => pull_prune(),
        Cli::Fetch => fetch_prune(),
    }
}

fn new_branch(name: &Vec<String>) -> Result<(), String> {
    if name.len() == 0 {
        return Err(String::from("name cannot be empty."));
    }

    let name = name.join("-");

    git::git_commands_status(vec![
        (
            "create new branch",
            vec!["switch", "--create", name.as_str()],
        ),
        (
            "push to origin",
            vec!["push", "--set-upstream", "origin", name.as_str()],
        ),
    ])?;

    Ok(())
}

fn push_branch(force: bool) -> Result<(), String> {
    let Output { stdout, .. } = git_command_output(
        "get current branch",
        vec!["rev-parse", "--abbrev-ref", "HEAD"],
    )?;

    let current_branch = match String::from_utf8(stdout) {
        Ok(value) => String::from(value.trim()),
        Err(err) => return Err(format!("{}", err)),
    };

    if current_branch.to_ascii_lowercase() == "head" {
        return Err(String::from(
            "HEAD is currently detached, no branch to push!",
        ));
    }

    let mut args = vec!["push", "--set-upstream"];
    if force {
        args.push("--force-with-lease");
    }
    args.push("origin");
    args.push(current_branch.as_str());
    let args = args;

    git_command_status("push", args)?;

    Ok(())
}

fn pull_prune() -> Result<(), String> {
    let branches = git_branches()?;

    for b in branches.iter() {
        println!("branch: {}", b);
    }
    Ok(())
}

fn fetch_prune() -> Result<(), String> {
    Ok(())
}
