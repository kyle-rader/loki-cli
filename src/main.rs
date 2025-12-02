pub mod git;
pub mod pruning;

use std::{
    collections::{BTreeMap, HashMap},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use chrono::{DateTime, Duration as ChronoDuration, Months, NaiveDate, Utc};
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

#[derive(Debug, Parser)]
struct AuthorStatsOptions {
    /// Limit analysis to commits from the last N days.
    #[clap(long, conflicts_with_all = &["weeks", "months", "from"])]
    days: Option<u32>,

    /// Limit analysis to commits from the last N weeks.
    #[clap(long, conflicts_with_all = &["days", "months", "from"])]
    weeks: Option<u32>,

    /// Limit analysis to commits from the last N months.
    #[clap(long, conflicts_with_all = &["days", "weeks", "from"])]
    months: Option<u32>,

    /// Include commits starting from YYYY-MM-DD (overrides --days/--weeks/--months).
    #[clap(long, value_parser = parse_naive_date, conflicts_with_all = &["days", "weeks", "months"])]
    from: Option<NaiveDate>,

    /// Include commits through YYYY-MM-DD (defaults to today).
    #[clap(long, value_parser = parse_naive_date)]
    to: Option<NaiveDate>,
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
    /// Set a prefix for all new branch names with `--prefix` or `LOKI_NEW_PREFIX`.
    #[clap(visible_alias = "n")]
    New {
        /// Optional prefix to prepend to the generated branch name.
        #[clap(long, env = "LOKI_NEW_PREFIX")]
        prefix: Option<String>,

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

    /// Analyze first-parent commits by author over time.
    #[clap(name = "stats")]
    Stats(AuthorStatsOptions),
}

const LOKI_NEW_PREFIX: &str = "LOKI_NEW_PREFIX";
const AUTHOR_GRAPH_WIDTH: usize = 40;

fn main() -> Result<(), String> {
    let cli = Cli::parse();

    match &cli {
        Cli::New { name, prefix } => new_branch(name, prefix.as_deref()),
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
        Cli::Stats(options) => author_stats(options),
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
        println!("{author}: {count}");
    }

    Ok(())
}

struct TimeRange {
    start_ts: Option<i64>,
    end_ts: i64,
    start_label: String,
    end_label: String,
    end_is_latest: bool,
}

fn author_stats(options: &AuthorStatsOptions) -> Result<(), String> {
    let range = resolve_time_range(options)?;
    let log_lines = git::git_command_lines(
        "collect author stats",
        vec![
            "log",
            "--first-parent",
            "--pretty=format:%ct%x09%an%x09%ae",
            "HEAD",
        ],
    )?;

    let mut totals: HashMap<String, usize> = HashMap::new();
    let mut email_to_name: HashMap<String, String> = HashMap::new();
    let mut timeline: BTreeMap<NaiveDate, HashMap<String, usize>> = BTreeMap::new();

    for raw_line in log_lines {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let parts: Vec<&str> = trimmed.split('\t').collect();
        if parts.len() != 3 {
            return Err(format!(
                "Unexpected git log output (expected `<timestamp>\\t<name>\\t<email>`): `{trimmed}`"
            ));
        }
        let timestamp_part = parts[0];
        let name_part = parts[1];
        let email_part = parts[2];

        let timestamp = timestamp_part.parse::<i64>().map_err(|err| {
            format!("Failed to parse git log timestamp `{timestamp_part}`: {err}")
        })?;

        if timestamp > range.end_ts {
            continue;
        }
        if let Some(start_ts) = range.start_ts {
            if timestamp < start_ts {
                continue;
            }
        }

        let email = email_part.trim();
        let email = if email.is_empty() {
            String::from("Unknown")
        } else {
            email.to_string()
        };

        let name = name_part.trim();
        if !name.is_empty() {
            email_to_name
                .entry(email.clone())
                .or_insert_with(|| name.to_string());
        }

        let date = DateTime::from_timestamp(timestamp, 0)
            .ok_or_else(|| format!("Commit timestamp out of range: {timestamp}"))?
            .date_naive();

        *totals.entry(email.clone()).or_insert(0) += 1;
        timeline
            .entry(date)
            .or_default()
            .entry(email)
            .and_modify(|count| *count += 1)
            .or_insert(1);
    }

    if totals.is_empty() {
        println!(
            "No first-parent commits found between {} and {}.",
            range.start_label, range.end_label
        );
        return Ok(());
    }

    let mut author_counts: Vec<(String, usize)> = totals.into_iter().collect();
    author_counts.sort_by(|(email_a, count_a), (email_b, count_b)| {
        count_b.cmp(count_a).then_with(|| email_a.cmp(email_b))
    });

    let total_commits: usize = author_counts.iter().map(|(_, count)| *count).sum();

    let resolved_end_label = if range.end_is_latest {
        timeline
            .keys()
            .next_back()
            .map(|date| format!("{date} (latest commit)"))
            .unwrap_or_else(|| String::from("latest commit"))
    } else {
        range.end_label.clone()
    };

    println!(
        "First-parent commits between {} and {}: {} total ({} authors).",
        range.start_label,
        resolved_end_label,
        total_commits,
        author_counts.len()
    );
    println!("Commits by author:");
    for (email, count) in &author_counts {
        let display = if let Some(name) = email_to_name.get(email) {
            format!("{} <{}>", name, email)
        } else {
            email.clone()
        };
        println!("  {display}: {count}");
    }

    let author_counts_with_names: Vec<(String, usize)> = author_counts
        .iter()
        .map(|(email, count)| {
            let display = if let Some(name) = email_to_name.get(email) {
                format!("{} <{}>", name, email)
            } else {
                email.clone()
            };
            (display, *count)
        })
        .collect();
    print_author_graph(&author_counts_with_names);

    Ok(())
}

fn print_author_graph(author_counts: &[(String, usize)]) {
    if author_counts.is_empty() {
        return;
    }

    let max_author_len = author_counts
        .iter()
        .map(|(email, _)| email.len())
        .max()
        .unwrap_or(0);
    let max_count = author_counts
        .iter()
        .map(|(_, count)| *count)
        .max()
        .unwrap_or(0);

    if max_count == 0 {
        return;
    }

    println!("Commit distribution graph:");
    for (email, count) in author_counts {
        let mut bar_len = if max_count == 0 {
            0
        } else {
            ((*count as f64) / (max_count as f64) * AUTHOR_GRAPH_WIDTH as f64).round() as usize
        };
        if *count > 0 && bar_len == 0 {
            bar_len = 1;
        }
        bar_len = bar_len.min(AUTHOR_GRAPH_WIDTH);

        let bar = "#".repeat(bar_len);
        let padding_len = AUTHOR_GRAPH_WIDTH.saturating_sub(bar_len);
        let padding = " ".repeat(padding_len);

        println!(
            "{email:<width$} | {}{} ({count})",
            bar,
            padding,
            width = max_author_len
        );
    }
}

fn resolve_time_range(options: &AuthorStatsOptions) -> Result<TimeRange, String> {
    let now = Utc::now();
    let (reference_end_dt, end_label, end_is_latest, end_ts) = if let Some(to_date) = options.to {
        let end_naive = to_date
            .and_hms_opt(23, 59, 59)
            .ok_or_else(|| String::from("invalid --to date"))?;
        let dt = DateTime::<Utc>::from_naive_utc_and_offset(end_naive, Utc);
        (dt, to_date.to_string(), false, dt.timestamp())
    } else {
        (now, String::from("latest commit"), true, i64::MAX)
    };

    let mut start_label = String::from("initial commit");
    let mut start_dt: Option<DateTime<Utc>> = None;

    if let Some(from_date) = options.from {
        let from_naive = from_date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| String::from("invalid --from date"))?;
        start_dt = Some(DateTime::<Utc>::from_naive_utc_and_offset(from_naive, Utc));
        start_label = from_date.to_string();
    } else if let Some(days) = options.days {
        if days == 0 {
            return Err(String::from("--days must be greater than zero."));
        }
        let dt = reference_end_dt - ChronoDuration::days(days as i64);
        start_label = format!(
            "{} (last {} day{})",
            dt.format("%Y-%m-%d"),
            days,
            if days == 1 { "" } else { "s" }
        );
        start_dt = Some(dt);
    } else if let Some(weeks) = options.weeks {
        if weeks == 0 {
            return Err(String::from("--weeks must be greater than zero."));
        }
        let dt = reference_end_dt - ChronoDuration::weeks(weeks as i64);
        start_label = format!(
            "{} (last {} week{})",
            dt.format("%Y-%m-%d"),
            weeks,
            if weeks == 1 { "" } else { "s" }
        );
        start_dt = Some(dt);
    } else if let Some(months_count) = options.months {
        if months_count == 0 {
            return Err(String::from("--months must be greater than zero."));
        }
        let end_date = reference_end_dt.date_naive();
        let start_date = end_date
            .checked_sub_months(Months::new(months_count))
            .ok_or_else(|| String::from("month subtraction overflow"))?;
        let start_naive = start_date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| String::from("invalid computed month range"))?;
        let dt = DateTime::<Utc>::from_naive_utc_and_offset(start_naive, Utc);
        start_label = format!(
            "{} (last {} month{})",
            start_date,
            months_count,
            if months_count == 1 { "" } else { "s" }
        );
        start_dt = Some(dt);
    }

    if let Some(start_dt_value) = start_dt {
        if start_dt_value > reference_end_dt {
            return Err(String::from(
                "The computed start date occurs after the end date. Check your filters.",
            ));
        }
    }

    Ok(TimeRange {
        start_ts: start_dt.map(|dt| dt.timestamp()),
        end_ts,
        start_label,
        end_label,
        end_is_latest,
    })
}

fn parse_naive_date(value: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|err| format!("Invalid date `{value}` (expected YYYY-MM-DD): {err}"))
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

fn new_branch(name: &[String], prefix: Option<&str>) -> Result<(), String> {
    if name.is_empty() {
        return Err(String::from("name cannot be empty."));
    }

    let mut name = name.join("-");

    if let Some(prefix) = prefix {
        eprintln!("Using branch prefix `{prefix}` (set via --prefix or {LOKI_NEW_PREFIX}).");
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
