pub mod git;
pub mod pruning;

use std::vec;

use clap::{
    builder::{styling::AnsiColor, Styles},
    Parser,
};
use git::{
    git_branches, git_command_lines, git_command_status, git_commands_status, git_current_branch,
};
use pruning::is_pruned_branch;

fn styles() -> clap::builder::Styles {
    Styles::styled()
        .header(AnsiColor::Yellow.on_default())
        .usage(AnsiColor::Green.on_default())
        .literal(AnsiColor::Magenta.on_default())
        .placeholder(AnsiColor::Cyan.on_default())
}

const LOKI_NEW_PREFIX: &str = "LOKI_NEW_PREFIX";
const NO_HOOKS: &str = "core.hooksPath=/dev/null";
const NO_HOOKS_ARGS: [&str; 2] = ["-c", NO_HOOKS];

#[derive(Debug, Parser)]
struct CommitOptions {
    #[clap(short, long)]
    all: bool,
    /// Optional message to include. Each MESSAGE will be joined on whitespace.
    message: Vec<String>,
}

#[derive(Parser)]
#[clap(version, about, author, color = clap::ColorChoice::Auto, styles = styles())]
enum Cli {
    /// Create a new branch from HEAD and push it to origin.
    /// Set a prefix for all new branch names with the env var LOKI_NEW_PREFIX
    #[clap(visible_alias = "n")]
    New {
        /// List of names to join with dashes to form a valid branch name.
        name: Vec<String>,
    },

    /// Push the current branch to origin with --set-upstream
    #[clap(visible_alias = "p")]
    Push {
        /// Use --force-with-lease
        #[clap(short, long)]
        force: bool,
    },

    /// Pull with --prune deleting local branches pruned from the remote.
    Pull,

    /// Fetch with --prune deleting local branches pruned from the remote.
    Fetch,

    /// Add, commit, and push using a timestamp based commit message.
    #[clap(visible_alias = "s")]
    Save(CommitOptions),

    /// Commit local changes
    #[clap(visible_alias = "c")]
    Commit(CommitOptions),

    /// Rebase the current branch onto the target branch after fetching.
    Rebase {
        /// The branch to rebase onto.
        #[clap(default_value = "main", env = "LOKI_REBASE_TARGET")]
        target: String,
    },

    /// Run any command without triggering any hooks
    #[clap(visible_alias = "x")]
    NoHooks {
        /// The command to run.
        command: Vec<String>,
    },
}

fn main() -> Result<(), String> {
    let cli = Cli::parse();

    match &cli {
        Cli::New { name } => new_branch(name),
        Cli::Push { force } => push_branch(*force),
        Cli::Pull => pull_prune(),
        Cli::Fetch => fetch_prune(),
        Cli::Save(CommitOptions { all, message }) => save(*all, message),
        Cli::Commit(CommitOptions { all, message }) => commit(*all, message),
        Cli::Rebase { target } => rebase(target),
        Cli::NoHooks { command } => no_hooks(command),
    }
}

fn without_hooks(command: &[impl AsRef<str>]) -> impl Iterator<Item = &str> {
    // create iter from no_hook_args and command
    NO_HOOKS_ARGS
        .iter()
        .copied()
        .chain(command.iter().map(|s| s.as_ref()))
}

fn no_hooks(command: &[impl AsRef<str>]) -> Result<(), String> {
    if command.is_empty() {
        return Err(String::from("command cannot be empty."));
    }

    let args = without_hooks(command);

    git_command_status("run command without hooks", args)?;

    Ok(())
}

fn rebase(target: &str) -> Result<(), String> {
    git_commands_status(vec![
        (
            "fetch target",
            vec![
                "-c",
                NO_HOOKS,
                "fetch",
                "origin",
                format!("{target}:{target}").as_str(),
            ],
        ),
        ("rebase", vec!["-c", NO_HOOKS, "rebase", target]),
    ])?;

    Ok(())
}

fn save(all: bool, message: &[String]) -> Result<(), String> {
    commit(all, message)?;
    push_branch(false)?;
    Ok(())
}

fn commit(all: bool, message: &[String]) -> Result<(), String> {
    let selector_option = if all { "--all" } else { "--update" };

    let message = if message.is_empty() {
        String::from("lk commit")
    } else {
        // leading space important for the format of the message below.
        message.join(" ")
    };

    git_commands_status(vec![
        ("add files", vec!["add", selector_option]),
        ("commit", vec!["commit", "--message", message.as_str()]),
    ])?;

    Ok(())
}

fn new_branch(name: &[String]) -> Result<(), String> {
    if name.is_empty() {
        return Err(String::from("name cannot be empty."));
    }

    let mut name = name.join("-");

    if let Ok(prefix) = std::env::var(LOKI_NEW_PREFIX) {
        eprintln!("Using prefix from env var {LOKI_NEW_PREFIX}={prefix}");
        name = format!("{prefix}{name}");
    }

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
                    "⚠️ Cannot delete pruned branch {pruned_branch} because HEAD is pointing to it."
                );
            } else if branches.contains(&pruned_branch) {
                if let Err(err) = git_command_status(
                    format!("💣 delete branch {pruned_branch}").as_str(),
                    vec!["branch", "-D", pruned_branch.as_str()],
                ) {
                    eprintln!("Failed to delete pruned branch {pruned_branch}: {err:?}")
                } else {
                    println!("💣      local  {pruned_branch} (pruned from remote)");
                }
            }
        }
    }

    Ok(())
}
