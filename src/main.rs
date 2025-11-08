pub mod git;
pub mod pruning;

use std::{
    collections::HashMap,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use clap::{
    builder::{styling::AnsiColor, Styles},
    Parser, Subcommand,
};
use git::{
    git_branches, git_command_iter, git_command_status, git_commands_status, git_current_branch,
};
use pruning::{highlight_branch_name, highlight_pruned_branch_line, is_pruned_branch};

fn styles() -> clap::builder::Styles {
    Styles::styled()
        .header(AnsiColor::Yellow.on_default())
        .usage(AnsiColor::Green.on_default())
        .literal(AnsiColor::Magenta.on_default())
        .placeholder(AnsiColor::Cyan.on_default())
}

const NO_HOOKS: &str = "core.hooksPath=/dev/null";

#[derive(Debug, Parser)]
struct CommitOptions {
    /// Stage and commit all changes with --all
    #[clap(short, long, default_value = "false")]
    all: bool,

    /// Stage and commit only tracked files with --update
    #[clap(short, long, default_value = "false")]
    update: bool,

    /// Optional message to include. Each MESSAGE will be joined on whitespace.
    message: Vec<String>,
}

#[derive(Debug, Subcommand)]
enum RepoSubcommand {
    /// Display repository statistics.
    Stats,
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
    ///
    /// Optionally stage files with --all or --update.
    #[clap(visible_alias = "s")]
    Save(CommitOptions),

    /// Commit local changes.
    ///
    /// Optionally stage files with --all or --update.
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

    /// Repository related commands.
    Repo {
        #[clap(subcommand)]
        command: RepoSubcommand,
    },
}

const LOKI_NEW_PREFIX: &str = "LOKI_NEW_PREFIX";

fn main() -> Result<(), String> {
    let cli = Cli::parse();

    match &cli {
        Cli::New { name } => new_branch(name),
        Cli::Push { force } => push_branch(*force),
        Cli::Pull => pull_prune(),
        Cli::Fetch => fetch_prune(),
        Cli::Save(commit_options) => save(commit_options),
        Cli::Commit(commit_options) => commit(commit_options),
        Cli::Rebase { target } => rebase(target),
        Cli::NoHooks { command } => no_hooks(command),
        Cli::Repo {
            command: RepoSubcommand::Stats,
        } => repo_stats(),
    }
}

fn no_hooks(command: &[impl AsRef<str>]) -> Result<(), String> {
    if command.is_empty() {
        return Err(String::from("command cannot be empty."));
    }

    let no_hook_args = [String::from("-c"), String::from(NO_HOOKS)];
    // create iter from no_hook_args and command
    let args = no_hook_args
        .iter()
        .map(|s| s.as_ref())
        .chain(command.iter().map(|s| s.as_ref()));

    git_command_status("run command without hooks", args)?;

    Ok(())
}

fn repo_stats() -> Result<(), String> {
    let author_lines =
        git::git_command_lines("collect commit authors", vec!["log", "--pretty=format:%an"])?;

    if author_lines.is_empty() {
        println!("No commits found.");
        return Ok(());
    }

    let mut author_counts: HashMap<String, usize> = HashMap::new();

    for raw_author in author_lines {
        let trimmed = raw_author.trim();
        let author = if trimmed.is_empty() {
            String::from("Unknown")
        } else {
            trimmed.to_string()
        };

        *author_counts.entry(author).or_insert(0) += 1;
    }

    let total_commits: usize = author_counts.values().sum();

    let mut author_counts: Vec<(String, usize)> = author_counts.into_iter().collect();
    author_counts.sort_by(|(author_a, count_a), (author_b, count_b)| {
        count_b.cmp(count_a).then_with(|| author_a.cmp(author_b))
    });

    let first_commit_hashes = git::git_command_lines(
        "find initial commits",
        vec!["rev-list", "--max-parents=0", "HEAD"],
    )?;

    let first_commit_hash = first_commit_hashes
        .first()
        .ok_or_else(|| String::from("Failed to determine the first commit."))?
        .trim()
        .to_string();

    let first_commit_timestamp = git::git_command_lines(
        "get first commit timestamp",
        vec!["show", "-s", "--format=%ct", first_commit_hash.as_str()],
    )?
    .first()
    .ok_or_else(|| String::from("Failed to read first commit timestamp."))?
    .trim()
    .parse::<u64>()
    .map_err(|err| format!("Failed to parse first commit timestamp: {err}"))?;

    let first_commit_date = git::git_command_lines(
        "get first commit date",
        vec!["show", "-s", "--format=%cs", first_commit_hash.as_str()],
    )?
    .first()
    .map(|date| date.trim().to_string())
    .unwrap_or_else(|| String::from("unknown"));

    let first_commit_time = UNIX_EPOCH + Duration::from_secs(first_commit_timestamp);

    let since_first_commit = SystemTime::now()
        .duration_since(first_commit_time)
        .unwrap_or_else(|_| Duration::from_secs(0));

    println!("Total commits: {total_commits}");
    println!(
        "Time since first commit: {} (since {})",
        format_duration(since_first_commit),
        first_commit_date
    );
    println!("Commits by author:");
    for (author, count) in author_counts {
        println!("  {author}: {count}");
    }

    Ok(())
}

fn format_duration(duration: Duration) -> String {
    let mut seconds = duration.as_secs();

    if seconds == 0 {
        return String::from("0s");
    }

    let years = seconds / 31_536_000;
    seconds %= 31_536_000;

    let days = seconds / 86_400;
    seconds %= 86_400;
    let hours = seconds / 3_600;
    seconds %= 3_600;
    let minutes = seconds / 60;

    let mut parts = Vec::new();
    if years > 0 {
        parts.push(format!("{years}y"));
    }
    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if minutes > 0 {
        parts.push(format!("{minutes}m"));
    }
    let remaining_seconds = duration.as_secs() % 60;
    if parts.is_empty() || remaining_seconds > 0 && parts.len() < 3 {
        parts.push(format!("{remaining_seconds}s"));
    }

    parts.join(" ")
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

fn save(options: &CommitOptions) -> Result<(), String> {
    commit(options)?;
    push_branch(false)?;
    Ok(())
}

fn commit(
    CommitOptions {
        all,
        update,
        message,
    }: &CommitOptions,
) -> Result<(), String> {
    let add_type = if *update {
        Some("--update")
    } else if *all {
        Some("--all")
    } else {
        None
    };

    let message = if message.is_empty() {
        String::from("lk commit")
    } else {
        message.join(" ")
    };

    let mut commands = vec![];

    if let Some(add_type) = add_type {
        commands.push(("add files", vec!["add", add_type]));
    }

    commands.push(("commit", vec!["commit", "--message", message.as_str()]));

    git_commands_status(commands)?;

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

    if current_branch.eq_ignore_ascii_case("head") {
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

    let mut pruned_branches = Vec::new();

    for line in git_command_iter("pull with pruning", vec![cmd, "--prune"])? {
        if let Some(pruned_branch) = is_pruned_branch(line.clone()) {
            println!("{}", highlight_pruned_branch_line(&line, &pruned_branch));
            if branches.contains(&pruned_branch) && pruned_branch != current_branch {
                pruned_branches.push(pruned_branch);
            }
        } else {
            println!("{line}");
        }
    }

    if pruned_branches.is_empty() {
        println!("No pruned branches found");
        return Ok(());
    }

    for pruned_branch in pruned_branches {
        let branch_delete_cmd = vec!["branch", "-D", pruned_branch.as_str()];
        let branch_delete = git_command_status(
            format!("💣 delete branch {pruned_branch}").as_str(),
            branch_delete_cmd,
        );
        if let Err(err) = branch_delete {
            eprintln!(
                "Failed to delete pruned branch {}: {err:?}",
                highlight_branch_name(&pruned_branch)
            )
        } else {
            println!(
                "💣 Deleted local branch {} (pruned from remote)",
                highlight_branch_name(&pruned_branch)
            );
        }
    }

    Ok(())
}
