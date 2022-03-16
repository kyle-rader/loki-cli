pub mod git;
pub mod pruning;

use clap::Parser;
use git::{git_branches, git_command_lines, git_command_status, git_current_branch};
use pruning::is_pruned_branch;

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
    let current_branch = git_current_branch()?;

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
    prune("pull")
}

fn fetch_prune() -> Result<(), String> {
    prune("fetch")
}

fn prune(cmd: &str) -> Result<(), String> {
    let current_branch = git_current_branch()?;
    let branches = git_branches()?;

    for line in git_command_lines("pull with pruning", vec![cmd, "--prune"])?.into_iter() {
        println!("{line}");
        if let Some(pruned_branch) = is_pruned_branch(line) {
            if pruned_branch.cmp(&current_branch).is_eq() {
                eprintln!(
                    "Cannot delete pruned branch {pruned_branch} because HEAD is pointing to it."
                );
            } else if branches.contains(&pruned_branch) {
                if let Err(err) = git_command_status(
                    format!("delete branch {pruned_branch}").as_str(),
                    vec!["branch", "-D", pruned_branch.as_str()],
                ) {
                    eprintln!("Failed to delete pruned branch {pruned_branch}: {err:?}")
                }
            }
        }
    }

    Ok(())
}
