pub mod git;
pub mod pruning;

use std::{
    collections::HashMap,
    io::{BufRead, Write},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use chrono::{DateTime, Duration as ChronoDuration, Months, NaiveDate, Utc};
use clap::{
    builder::{styling::AnsiColor, Styles},
    Parser, Subcommand,
};
use colored::Colorize;
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

#[derive(Debug, Default, Parser)]
struct RepoStatsOptions {
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

    /// Limit the output to the top N contributors.
    #[clap(long)]
    top: Option<usize>,

    /// Only include commits authored by these names (repeatable, case-insensitive fuzzy match).
    #[clap(long = "name", value_name = "NAME")]
    names: Vec<String>,

    /// Only include commits authored by these emails (repeatable, case-insensitive fuzzy match).
    #[clap(long = "email", value_name = "EMAIL")]
    emails: Vec<String>,
}


#[derive(Debug, Subcommand)]
enum RepoSubcommand {
    /// Analyze first-parent commits by author over time.
    #[clap(name = "stats")]
    Stats(RepoStatsOptions),
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

        /// Start an interactive rebase.
        #[clap(short, long)]
        interactive: bool,
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
        Cli::New { name, prefix } => new_branch(name, prefix.as_deref()),
        Cli::Push { force } => push_branch(*force),
        Cli::Pull => pull_prune(),
        Cli::Fetch => fetch_prune(),
        Cli::Save(commit_options) => save(commit_options),
        Cli::Commit(commit_options) => commit(commit_options),
        Cli::Rebase { target, interactive } => rebase(target, *interactive),
        Cli::NoHooks { command } => no_hooks(command),
        Cli::Repo {
            command: RepoSubcommand::Stats(options),
        } => repo_stats(options),
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

struct TimeRange {
    start_ts: Option<i64>,
    end_ts: i64,
    start_label: String,
    end_label: String,
    end_is_latest: bool,
}

fn repo_stats(options: &RepoStatsOptions) -> Result<(), String> {
    let progress = start_delayed_progress_meter("Computing repo stats...", Duration::from_secs(1));

    let range = resolve_time_range(options)?;
    if let Some(top) = options.top {
        if top == 0 {
            return Err(String::from("--top must be greater than zero."));
        }
    }

    let mut totals: HashMap<String, usize> = HashMap::new();
    let mut email_to_name: HashMap<String, String> = HashMap::new();
    let mut email_aliases: HashMap<String, String> = HashMap::new();
    let mut name_to_email: HashMap<String, String> = HashMap::new();
    let mut latest_commit_date_in_range: Option<NaiveDate> = None;

    let name_filters_lower: Vec<String> = options.names.iter().map(|s| s.to_lowercase()).collect();
    let email_filters_lower: Vec<String> =
        options.emails.iter().map(|s| s.to_lowercase()).collect();

    let mut git_args: Vec<String> = vec![
        "log".to_string(),
        "--first-parent".to_string(),
        "--pretty=format:%ct%x09%an%x09%ae".to_string(),
    ];
    if let Some(start_ts) = range.start_ts {
        git_args.push(format!("--since=@{start_ts}"));
    }
    if !range.end_is_latest {
        git_args.push(format!("--until=@{}", range.end_ts));
    }
    git_args.push("HEAD".to_string());

    let mut child = Command::new("git")
        .args(git_args)
        .stdout(Stdio::piped())
        // Avoid buffering/stalling on stderr while still surfacing errors.
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|err| format!("collect author stats failed to start: {err}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| String::from("collect author stats failed to capture stdout"))?;
    let reader = std::io::BufReader::new(stdout);

    for raw_line in reader.lines() {
        let raw_line = raw_line
            .map_err(|err| format!("Failed to read git log output: {err}"))?;
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let mut parts = trimmed.splitn(3, '\t');
        let (timestamp_part, name_part, email_part) =
            match (parts.next(), parts.next(), parts.next()) {
                (Some(ts), Some(name), Some(email)) => (ts, name, email),
                _ => {
                    return Err(format!(
                        "Unexpected git log output (expected `<timestamp>\\t<name>\\t<email>`): `{trimmed}`"
                    ));
                }
            };
        if timestamp_part.is_empty() {
            return Err(format!(
                "Unexpected git log output (expected `<timestamp>\\t<name>\\t<email>`): `{trimmed}`"
            ));
        }

        let timestamp = timestamp_part.parse::<i64>().map_err(|err| {
            format!("Failed to parse git log timestamp `{timestamp_part}`: {err}")
        })?;

        let email = email_part.trim();
        let email = if email.is_empty() { "Unknown" } else { email };

        let name = name_part.trim();
        let canonical_email =
            canonicalize_author(email, name, &mut email_aliases, &mut name_to_email);

        if !matches_author_filters_lowered(
            name,
            canonical_email.as_str(),
            &name_filters_lower,
            &email_filters_lower,
        ) {
            continue;
        }

        if !name.is_empty() {
            email_to_name
                .entry(canonical_email.clone())
                .or_insert_with(|| name.to_string());
        }

        let date = DateTime::from_timestamp(timestamp, 0)
            .ok_or_else(|| format!("Commit timestamp out of range: {timestamp}"))?
            .date_naive();
        if latest_commit_date_in_range.is_none() {
            // `git log` is reverse-chronological, so the first matching commit is the latest.
            latest_commit_date_in_range = Some(date);
        }

        *totals.entry(canonical_email.clone()).or_insert(0) += 1;
    }

    let status = child
        .wait()
        .map_err(|err| format!("collect author stats failed to wait: {err}"))?;
    if !status.success() {
        return Err(format!(
            "collect author stats failed with exit code: {}",
            status.code().unwrap_or(-1)
        ));
    }

    progress.finish();

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
    let unique_authors = author_counts.len();
    let display_author_counts: Vec<(String, usize)> = if let Some(top_n) = options.top {
        author_counts.iter().take(top_n).cloned().collect()
    } else {
        author_counts.clone()
    };

    let resolved_end_label = if range.end_is_latest {
        latest_commit_date_in_range
            .map(|date| format!("{date} (latest commit)"))
            .unwrap_or_else(|| String::from("latest commit"))
    } else {
        range.end_label.clone()
    };

    // Dashboard-style stats list
    println!("Repository Statistics");
    println!("  Range: {} to {}", range.start_label, resolved_end_label);
    println!("  Total commits: {}", total_commits.to_string().green());
    println!("  Authors: {}", unique_authors.to_string().green());

    let display_author_counts_with_names: Vec<(String, usize)> = display_author_counts
        .into_iter()
        .map(|(email, count)| {
            let display = if let Some(name) = email_to_name.get(&email) {
                format!("{} <{}>", name, email)
            } else {
                email
            };
            (display, count)
        })
        .collect();
    print_author_graph(&display_author_counts_with_names);

    Ok(())
}

fn canonicalize_author(
    email: &str,
    name: &str,
    email_aliases: &mut HashMap<String, String>,
    name_to_email: &mut HashMap<String, String>,
) -> String {
    if let Some(existing_email) = email_aliases.get(email) {
        return existing_email.clone();
    }

    if !name.is_empty() {
        if let Some(existing_email) = name_to_email.get(name) {
            email_aliases.insert(email.to_string(), existing_email.clone());
            return existing_email.clone();
        } else {
            name_to_email.insert(name.to_string(), email.to_string());
        }
    }

    let canonical = email.to_string();
    email_aliases
        .entry(canonical.clone())
        .or_insert_with(|| canonical.clone());
    canonical
}

fn matches_author_filters_lowered(
    name: &str,
    email: &str,
    name_filters_lower: &[String],
    email_filters_lower: &[String],
) -> bool {
    if !name_filters_lower.is_empty() {
        if name.is_empty() {
            return false;
        }
        let name_lower = name.to_lowercase();
        if !name_filters_lower
            .iter()
            .any(|filter| name_lower.contains(filter))
        {
            return false;
        }
    }

    if !email_filters_lower.is_empty() {
        if email.is_empty() {
            return false;
        }
        let email_lower = email.to_lowercase();
        if !email_filters_lower
            .iter()
            .any(|filter| email_lower.contains(filter))
        {
            return false;
        }
    }

    true
}

fn print_author_graph(author_counts: &[(String, usize)]) {
    if author_counts.is_empty() {
        return;
    }

    println!("Commits by author:");
    for (author_display, count) in author_counts {
        // Color the count green
        let count_str = count.to_string().green();

        // Colorize email addresses (extract email from "Name <email>" format or use as-is)
        let colored_author = if let Some(start) = author_display.find('<') {
            if let Some(end) = author_display.find('>') {
                let name = &author_display[..start].trim();
                let email = &author_display[start + 1..end];
                format!("{} <{}>", name, email.yellow().to_string())
            } else {
                author_display.yellow().to_string()
            }
        } else {
            // If no angle brackets, assume the whole thing is an email
            author_display.yellow().to_string()
        };

        println!("({count_str}) {colored_author}");
    }
}

fn resolve_time_range(options: &RepoStatsOptions) -> Result<TimeRange, String> {
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

fn matches_author_filters(name: &str, email: &str, options: &RepoStatsOptions) -> bool {
    if !options.names.is_empty()
        && (name.is_empty()
            || !options
                .names
                .iter()
                .any(|filter| name.to_lowercase().contains(&filter.to_lowercase())))
    {
        return false;
    }

    if !options.emails.is_empty()
        && (email.is_empty()
            || !options
                .emails
                .iter()
                .any(|filter| email.to_lowercase().contains(&filter.to_lowercase())))
    {
        return false;
    }

    true
}

fn parse_naive_date(value: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|err| format!("Invalid date `{value}` (expected YYYY-MM-DD): {err}"))
}

struct ProgressMeter {
    done: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl ProgressMeter {
    fn finish(mut self) {
        self.done.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        // Clear the line in case we printed progress.
        let mut stderr = std::io::stderr();
        let _ = write!(stderr, "\r{}\r", " ".repeat(80));
        let _ = stderr.flush();
    }
}

impl Drop for ProgressMeter {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        let mut stderr = std::io::stderr();
        let _ = write!(stderr, "\r{}\r", " ".repeat(80));
        let _ = stderr.flush();
    }
}

fn start_delayed_progress_meter(message: &'static str, delay: Duration) -> ProgressMeter {
    let done = Arc::new(AtomicBool::new(false));
    let done_clone = Arc::clone(&done);
    let handle = std::thread::spawn(move || {
        std::thread::sleep(delay);
        if done_clone.load(Ordering::Relaxed) {
            return;
        }

        let spinner = ['|', '/', '-', '\\'];
        let mut i = 0usize;
        while !done_clone.load(Ordering::Relaxed) {
            let mut stderr = std::io::stderr();
            let _ = write!(stderr, "\r{message} {}", spinner[i % spinner.len()]);
            let _ = stderr.flush();
            i = i.wrapping_add(1);
            std::thread::sleep(Duration::from_millis(120));
        }
    });

    ProgressMeter {
        done,
        handle: Some(handle),
    }
}

fn rebase(target: &str, interactive: bool) -> Result<(), String> {
    let fetch_target = format!("{target}:{target}");
    git_command_status(
        "fetch target",
        vec![
            "-c",
            NO_HOOKS,
            "fetch",
            "origin",
            fetch_target.as_str(),
        ],
    )?;

    let mut rebase_args = vec!["-c", NO_HOOKS, "rebase"];
    if interactive {
        rebase_args.push("-i");
    }
    rebase_args.push(target);

    git_command_status("rebase", rebase_args)?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn canonicalize_author_reuses_first_email_for_name() {
        let mut email_aliases = HashMap::new();
        let mut name_to_email = HashMap::new();

        let first = canonicalize_author(
            "alias@microsoft.com",
            "msuser",
            &mut email_aliases,
            &mut name_to_email,
        );
        assert_eq!(first, "alias@microsoft.com");

        let second = canonicalize_author(
            "msuser@microsoft.com",
            "msuser",
            &mut email_aliases,
            &mut name_to_email,
        );
        assert_eq!(second, "alias@microsoft.com");
    }

    #[test]
    fn canonicalize_author_handles_name_change_after_alias() {
        let mut email_aliases = HashMap::new();
        let mut name_to_email = HashMap::new();

        canonicalize_author(
            "alias@microsoft.com",
            "msuser",
            &mut email_aliases,
            &mut name_to_email,
        );
        canonicalize_author(
            "msuser@microsoft.com",
            "msuser",
            &mut email_aliases,
            &mut name_to_email,
        );
        let reused = canonicalize_author(
            "msuser@microsoft.com",
            "display name",
            &mut email_aliases,
            &mut name_to_email,
        );
        assert_eq!(reused, "alias@microsoft.com");
    }

    #[test]
    fn matches_author_filters_by_name_exact() {
        let mut options = RepoStatsOptions::default();
        options.names = vec![String::from("Example User")];

        assert!(matches_author_filters(
            "Example User",
            "user@example.com",
            &options
        ));
        assert!(!matches_author_filters(
            "Someone Else",
            "user@example.com",
            &options
        ));
    }

    #[test]
    fn matches_author_filters_by_name_fuzzy() {
        let mut options = RepoStatsOptions::default();
        options.names = vec![String::from("example")];

        // Fuzzy match: "example" is a substring of "Example User"
        assert!(matches_author_filters(
            "Example User",
            "user@example.com",
            &options
        ));
        // Case insensitive fuzzy match
        assert!(matches_author_filters(
            "EXAMPLE USER",
            "user@example.com",
            &options
        ));
        // No match
        assert!(!matches_author_filters(
            "Someone Else",
            "user@example.com",
            &options
        ));
    }

    #[test]
    fn matches_author_filters_by_name_case_insensitive() {
        let mut options = RepoStatsOptions::default();
        options.names = vec![String::from("EXAMPLE USER")];

        assert!(matches_author_filters(
            "example user",
            "user@example.com",
            &options
        ));
        assert!(matches_author_filters(
            "Example User",
            "user@example.com",
            &options
        ));
    }

    #[test]
    fn matches_author_filters_by_email_exact() {
        let mut options = RepoStatsOptions::default();
        options.emails = vec![String::from("user@example.com")];

        assert!(matches_author_filters(
            "Example User",
            "user@example.com",
            &options
        ));
        assert!(!matches_author_filters(
            "Example User",
            "other@example.com",
            &options
        ));
    }

    #[test]
    fn matches_author_filters_by_email_fuzzy() {
        let mut options = RepoStatsOptions::default();
        options.emails = vec![String::from("example.com")];

        // Fuzzy match: "example.com" is a substring of "user@example.com"
        assert!(matches_author_filters(
            "Example User",
            "user@example.com",
            &options
        ));
        // Also matches other emails from the same domain
        assert!(matches_author_filters(
            "Example User",
            "other@example.com",
            &options
        ));
        // No match for different domain
        assert!(!matches_author_filters(
            "Example User",
            "user@other.com",
            &options
        ));
    }

    #[test]
    fn matches_author_filters_by_email_case_insensitive() {
        let mut options = RepoStatsOptions::default();
        options.emails = vec![String::from("USER@EXAMPLE.COM")];

        assert!(matches_author_filters(
            "Example User",
            "user@example.com",
            &options
        ));
        assert!(matches_author_filters(
            "Example User",
            "User@Example.Com",
            &options
        ));
    }

    #[test]
    fn matches_author_filters_requires_all_filters() {
        let mut options = RepoStatsOptions::default();
        options.names = vec![String::from("Example User")];
        options.emails = vec![String::from("user@example.com")];

        assert!(matches_author_filters(
            "Example User",
            "user@example.com",
            &options
        ));
        assert!(!matches_author_filters(
            "Example User",
            "other@other.com",
            &options
        ));
        assert!(!matches_author_filters(
            "Another User",
            "user@example.com",
            &options
        ));
    }

    #[test]
    fn matches_author_filters_fuzzy_with_multiple_filters() {
        let mut options = RepoStatsOptions::default();
        options.names = vec![String::from("john"), String::from("jane")];

        // Matches first filter
        assert!(matches_author_filters(
            "John Smith",
            "john@example.com",
            &options
        ));
        // Matches second filter
        assert!(matches_author_filters(
            "Jane Doe",
            "jane@example.com",
            &options
        ));
        // No match
        assert!(!matches_author_filters(
            "Bob Wilson",
            "bob@example.com",
            &options
        ));
    }
}
