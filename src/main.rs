pub mod git;
pub mod pruning;
pub mod vars;
pub mod worktree;

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

use vars::{LOKI_NEW_PREFIX, LOKI_REBASE_TARGET, LOKI_WORKTREE_BASE, NO_HOOKS};

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
    #[clap(long, default_value_t = 20)]
    top: usize,

    /// Only count commits on the first-parent chain of HEAD.
    ///
    /// By default `lk repo stats` walks every commit reachable from HEAD
    /// (with patch-id deduplication applied so logically-identical commits
    /// from rebases / cherry-picks / cross-repo migrations are counted
    /// once). Pass `--first-parent` to restrict the walk to the mainline
    /// of merges into HEAD — useful when each PR is merged with a merge
    /// commit and you want one tally per PR.
    #[clap(long, default_value_t = false)]
    first_parent: bool,

    /// Only include commits authored by these names (repeatable, case-insensitive fuzzy match).
    #[clap(long = "name", value_name = "NAME")]
    names: Vec<String>,

    /// Only include commits authored by these emails (repeatable, case-insensitive fuzzy match).
    #[clap(long = "email", value_name = "EMAIL")]
    emails: Vec<String>,

    /// Disable patch-id-based deduplication of logically-identical commits.
    ///
    /// By default, `lk repo stats` collapses commits that share the same
    /// `git patch-id` (different SHAs but identical patches) so that
    /// migrated / rebased / cherry-picked history doesn't double-count
    /// contributors. Pass `--no-dedup` to count every SHA individually
    /// (the pre-2.5.0 behavior).
    #[clap(long, default_value_t = false)]
    no_dedup: bool,
}

#[derive(Debug, Subcommand)]
enum RepoSubcommand {
    /// Analyze commits by author over time.
    #[clap(name = "stats")]
    Stats(RepoStatsOptions),
}

#[derive(Debug, Subcommand)]
enum WorktreeSubcommand {
    /// Create a new worktree and branch.
    #[clap(visible_alias = "a")]
    Add {
        /// Optional prefix to prepend to the branch name.
        #[clap(long, env = LOKI_NEW_PREFIX)]
        prefix: Option<String>,

        /// Base ref to create the worktree from.
        #[clap(short, long, default_value = "origin/main", env = LOKI_WORKTREE_BASE)]
        base: String,

        /// Name parts joined with dashes to form the worktree and branch name.
        name: Vec<String>,
    },

    /// Remove a worktree and its associated branch.
    #[clap(visible_alias = "r")]
    Remove {
        /// Force removal of a dirty worktree.
        #[clap(short, long)]
        force: bool,

        /// Worktree name. If omitted, inferred from the current directory.
        name: Vec<String>,
    },

    /// List all worktrees.
    #[clap(visible_alias = "l")]
    List,

    /// Print a cd command for switching to a worktree (use with eval).
    ///
    /// bash/zsh: eval "$(lk w s <name>)"
    /// PowerShell: lk w s <name> | Invoke-Expression
    #[clap(visible_alias = "s")]
    Switch {
        /// Worktree name. If omitted, switches to the main worktree.
        name: Vec<String>,
    },
}

#[derive(Parser)]
#[clap(version, about, author, color = clap::ColorChoice::Auto, styles = styles())]
enum Cli {
    /// Create a new branch from HEAD and push it to origin.
    /// Set a prefix for all new branch names with `--prefix` or `LOKI_NEW_PREFIX`.
    #[clap(visible_alias = "n")]
    New {
        /// Optional prefix to prepend to the generated branch name.
        #[clap(long, env = LOKI_NEW_PREFIX)]
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
        #[clap(default_value = "main", env = LOKI_REBASE_TARGET)]
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

    /// Manage git worktrees.
    #[clap(visible_alias = "w")]
    Worktree {
        #[clap(subcommand)]
        command: WorktreeSubcommand,
    },

    /// Push the main branch to the release branch.
    #[clap(visible_alias = "r")]
    Release,
}

fn main() -> Result<(), String> {
    let cli = Cli::parse();

    match &cli {
        Cli::New { name, prefix } => new_branch(name, prefix.as_deref()),
        Cli::Push { force } => push_branch(*force),
        Cli::Pull => pull_prune(),
        Cli::Fetch => fetch_prune(),
        Cli::Save(commit_options) => save(commit_options),
        Cli::Commit(commit_options) => commit(commit_options),
        Cli::Rebase {
            target,
            interactive,
        } => rebase(target, *interactive),
        Cli::NoHooks { command } => no_hooks(command),
        Cli::Repo {
            command: RepoSubcommand::Stats(options),
        } => repo_stats(options),
        Cli::Worktree { command } => match command {
            WorktreeSubcommand::Add { name, base, prefix } => {
                worktree::worktree_add(name, base, prefix.as_deref())
            }
            WorktreeSubcommand::Remove { name, force } => worktree::worktree_remove(name, *force),
            WorktreeSubcommand::List => worktree::worktree_list(),
            WorktreeSubcommand::Switch { name } => worktree::worktree_switch(name),
        },
        Cli::Release => release(),
    }
}

fn release() -> Result<(), String> {
    git_command_status(
        "push main to release",
        vec!["push", "origin", "main:release"],
    )?;
    Ok(())
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
    if options.top == 0 {
        return Err(String::from("--top must be greater than zero."));
    }

    // Phase 1: collect candidate commits in one `git log` pass.
    let raw_commits = collect_raw_commits(options, &range)?;

    // Phase 2: optionally deduplicate by patch-id.
    let (effective_commits, duplicates_collapsed) = if options.no_dedup {
        (raw_commits, 0usize)
    } else {
        let shas: Vec<&str> = raw_commits.iter().map(|c| c.sha.as_str()).collect();
        let patch_ids = compute_patch_ids(&shas)?;
        dedup_commits(raw_commits, &patch_ids)
    };

    // Phase 3: tally (filters, aliasing, active windows).
    let mut totals: HashMap<String, usize> = HashMap::new();
    let mut email_to_name: HashMap<String, String> = HashMap::new();
    let mut email_aliases: HashMap<String, String> = HashMap::new();
    let mut name_to_email: HashMap<String, String> = HashMap::new();
    let mut latest_commit_date_in_range: Option<NaiveDate> = None;
    let mut latest_commit_ts_by_author: HashMap<String, i64> = HashMap::new();
    let mut oldest_commit_ts_by_author: HashMap<String, i64> = HashMap::new();

    let name_filters_lower: Vec<String> = options.names.iter().map(|s| s.to_lowercase()).collect();
    let email_filters_lower: Vec<String> =
        options.emails.iter().map(|s| s.to_lowercase()).collect();

    // `collect_raw_commits` preserves git log's reverse-chronological order, so
    // the per-author "latest/oldest" tracking remains correct.
    for commit in &effective_commits {
        let email = if commit.email.is_empty() {
            "Unknown"
        } else {
            commit.email.as_str()
        };
        let name = commit.name.as_str();
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

        let date = DateTime::from_timestamp(commit.timestamp, 0)
            .ok_or_else(|| format!("Commit timestamp out of range: {}", commit.timestamp))?
            .date_naive();
        if latest_commit_date_in_range.is_none() {
            latest_commit_date_in_range = Some(date);
        } else if let Some(current_latest) = latest_commit_date_in_range {
            if date > current_latest {
                latest_commit_date_in_range = Some(date);
            }
        }

        // Per-author windows: track min and max independently because dedup
        // may have reordered commits relative to git log's stream.
        latest_commit_ts_by_author
            .entry(canonical_email.clone())
            .and_modify(|ts| {
                if commit.timestamp > *ts {
                    *ts = commit.timestamp;
                }
            })
            .or_insert(commit.timestamp);
        oldest_commit_ts_by_author
            .entry(canonical_email.clone())
            .and_modify(|ts| {
                if commit.timestamp < *ts {
                    *ts = commit.timestamp;
                }
            })
            .or_insert(commit.timestamp);

        *totals.entry(canonical_email.clone()).or_insert(0) += 1;
    }

    progress.finish();

    if totals.is_empty() {
        if options.first_parent {
            println!(
                "No first-parent commits found between {} and {}.",
                range.start_label, range.end_label
            );
        } else {
            println!(
                "No commits found between {} and {}.",
                range.start_label, range.end_label
            );
        }
        return Ok(());
    }

    let mut author_counts: Vec<(String, usize)> = totals.into_iter().collect();
    author_counts.sort_by(|(email_a, count_a), (email_b, count_b)| {
        count_b.cmp(count_a).then_with(|| email_a.cmp(email_b))
    });

    let total_commits: usize = author_counts.iter().map(|(_, count)| *count).sum();
    let unique_authors = author_counts.len();
    let display_author_counts: Vec<(String, usize)> =
        author_counts.iter().take(options.top).cloned().collect();

    let resolved_end_label = if range.end_is_latest {
        latest_commit_date_in_range
            .map(|date| format!("{date} (latest commit)"))
            .unwrap_or_else(|| range.end_label.clone())
    } else {
        range.end_label.clone()
    };

    // Dashboard-style stats list
    println!("Repository Statistics");
    println!("  Range: {} to {}", range.start_label, resolved_end_label);
    let total_commits_str = total_commits.to_string().green();
    if duplicates_collapsed > 0 {
        println!(
            "  Total commits: {total_commits_str} ({} duplicate patch{} collapsed; --no-dedup to disable)",
            duplicates_collapsed,
            if duplicates_collapsed == 1 { "" } else { "es" },
        );
    } else {
        println!("  Total commits: {total_commits_str}");
    }
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
    print_author_graph(
        &display_author_counts_with_names,
        &latest_commit_ts_by_author,
        &oldest_commit_ts_by_author,
    );

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawCommit {
    sha: String,
    timestamp: i64,
    name: String,
    email: String,
}

fn collect_raw_commits(
    options: &RepoStatsOptions,
    range: &TimeRange,
) -> Result<Vec<RawCommit>, String> {
    let mut git_args: Vec<String> = vec!["log".to_string()];
    if options.first_parent {
        git_args.push("--first-parent".to_string());
    }
    git_args.push("--pretty=format:%H%x09%ct%x09%an%x09%ae".to_string());
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
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|err| format!("collect author stats failed to start: {err}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| String::from("collect author stats failed to capture stdout"))?;
    let reader = std::io::BufReader::new(stdout);

    let mut commits = Vec::new();
    for raw_line in reader.lines() {
        let raw_line = raw_line.map_err(|err| format!("Failed to read git log output: {err}"))?;
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let mut parts = trimmed.splitn(4, '\t');
        let (sha_part, timestamp_part, name_part, email_part) = match (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) {
            (Some(sha), Some(ts), Some(name), Some(email)) => (sha, ts, name, email),
            _ => {
                return Err(format!(
                        "Unexpected git log output (expected `<sha>\\t<timestamp>\\t<name>\\t<email>`): `{trimmed}`"
                    ));
            }
        };
        if sha_part.is_empty() || timestamp_part.is_empty() {
            return Err(format!(
                "Unexpected git log output (expected `<sha>\\t<timestamp>\\t<name>\\t<email>`): `{trimmed}`"
            ));
        }

        let timestamp = timestamp_part.parse::<i64>().map_err(|err| {
            format!("Failed to parse git log timestamp `{timestamp_part}`: {err}")
        })?;

        commits.push(RawCommit {
            sha: sha_part.to_string(),
            timestamp,
            name: name_part.trim().to_string(),
            email: email_part.trim().to_string(),
        });
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

    Ok(commits)
}

/// Resolve each commit SHA to a `git patch-id --stable` value, when one
/// exists. Returns a map from SHA → patch-id. Commits with no patch
/// (empty diffs, merge commits) simply won't appear in the map.
fn compute_patch_ids(shas: &[&str]) -> Result<HashMap<String, String>, String> {
    if shas.is_empty() {
        return Ok(HashMap::new());
    }

    // git diff-tree --stdin -p < SHAs  |  git patch-id --stable
    let mut diff_tree = Command::new("git")
        .args(["diff-tree", "--stdin", "-p"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|err| format!("failed to spawn `git diff-tree`: {err}"))?;

    let diff_tree_stdout = diff_tree
        .stdout
        .take()
        .ok_or_else(|| String::from("failed to capture `git diff-tree` stdout"))?;

    let mut patch_id = Command::new("git")
        .args(["patch-id", "--stable"])
        .stdin(Stdio::from(diff_tree_stdout))
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|err| format!("failed to spawn `git patch-id`: {err}"))?;

    // Writer thread feeds SHAs into diff-tree's stdin; the chained patch-id
    // process consumes diff-tree's stdout concurrently to avoid deadlock on
    // a full pipe buffer.
    let mut diff_tree_stdin = diff_tree
        .stdin
        .take()
        .ok_or_else(|| String::from("failed to capture `git diff-tree` stdin"))?;
    let shas_owned: Vec<String> = shas.iter().map(|s| s.to_string()).collect();
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        for sha in &shas_owned {
            writeln!(diff_tree_stdin, "{sha}")?;
        }
        drop(diff_tree_stdin);
        Ok(())
    });

    let patch_id_stdout = patch_id
        .stdout
        .take()
        .ok_or_else(|| String::from("failed to capture `git patch-id` stdout"))?;
    let reader = std::io::BufReader::new(patch_id_stdout);
    let mut map: HashMap<String, String> = HashMap::new();
    for line in reader.lines() {
        let line = line.map_err(|err| format!("Failed to read patch-id output: {err}"))?;
        // Each line is "<patch-id> <commit-sha>".
        let mut parts = line.split_whitespace();
        let (Some(pid), Some(commit_sha)) = (parts.next(), parts.next()) else {
            continue;
        };
        map.insert(commit_sha.to_string(), pid.to_string());
    }

    writer
        .join()
        .map_err(|_| String::from("patch-id writer thread panicked"))?
        .map_err(|err| format!("failed writing SHAs to `git diff-tree`: {err}"))?;

    let diff_tree_status = diff_tree
        .wait()
        .map_err(|err| format!("`git diff-tree` failed to wait: {err}"))?;
    if !diff_tree_status.success() {
        return Err(format!(
            "`git diff-tree` failed with exit code: {}",
            diff_tree_status.code().unwrap_or(-1)
        ));
    }
    let patch_id_status = patch_id
        .wait()
        .map_err(|err| format!("`git patch-id` failed to wait: {err}"))?;
    if !patch_id_status.success() {
        return Err(format!(
            "`git patch-id` failed with exit code: {}",
            patch_id_status.code().unwrap_or(-1)
        ));
    }

    Ok(map)
}

/// Collapse commits that share a `git patch-id`. Commits without an
/// entry in `patch_ids` are treated as unique (their SHA becomes their
/// own dedup key). For each patch-id group, the winner is the commit
/// with the smallest `(timestamp, sha)` tuple — i.e. earliest author
/// date, with SHA breaking ties deterministically.
///
/// Returns `(winners, duplicates_collapsed)`.
fn dedup_commits(
    commits: Vec<RawCommit>,
    patch_ids: &HashMap<String, String>,
) -> (Vec<RawCommit>, usize) {
    let original_count = commits.len();
    let mut by_key: HashMap<String, RawCommit> = HashMap::new();
    for commit in commits {
        let key = match patch_ids.get(&commit.sha) {
            Some(pid) => format!("p:{pid}"),
            None => format!("s:{}", commit.sha),
        };
        match by_key.entry(key) {
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(commit);
            }
            std::collections::hash_map::Entry::Occupied(mut e) => {
                let existing = e.get();
                let new_key = (commit.timestamp, commit.sha.as_str());
                let existing_key = (existing.timestamp, existing.sha.as_str());
                if new_key < existing_key {
                    e.insert(commit);
                }
            }
        }
    }
    let winners: Vec<RawCommit> = by_key.into_values().collect();
    let dups = original_count - winners.len();
    (winners, dups)
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

fn active_weeks_inclusive(latest_ts: i64, oldest_ts: i64) -> f64 {
    let span_seconds = latest_ts.saturating_sub(oldest_ts).max(0);
    // Inclusive day window keeps single-commit authors reasonable (1 day => 1/7 week),
    // and still properly boosts authors who only started partway through the range.
    let active_days = (span_seconds / 86_400) + 1;
    (active_days as f64) / 7.0
}

fn active_days_inclusive(latest_ts: i64, oldest_ts: i64) -> i64 {
    let span_seconds = latest_ts.saturating_sub(oldest_ts).max(0);
    (span_seconds / 86_400) + 1
}

fn format_active_span(latest_ts: i64, oldest_ts: i64) -> String {
    // Use an average Gregorian year/month to avoid jumpy “calendar” math.
    let days = active_days_inclusive(latest_ts, oldest_ts) as f64;
    let years = days / 365.25;
    if years >= 1.0 {
        let rounded = (years * 10.0).round() / 10.0;
        let unit = if (rounded - 1.0).abs() < 1e-9 {
            "year"
        } else {
            "years"
        };
        format!("{rounded:.1} {unit}")
    } else {
        let months = days / (365.25 / 12.0);
        let rounded = (months * 10.0).round() / 10.0;
        let unit = if (rounded - 1.0).abs() < 1e-9 {
            "month"
        } else {
            "months"
        };
        format!("{rounded:.1} {unit}")
    }
}

fn print_author_graph(
    author_counts: &[(String, usize)],
    latest_commit_ts_by_author: &HashMap<String, i64>,
    oldest_commit_ts_by_author: &HashMap<String, i64>,
) {
    if author_counts.is_empty() {
        return;
    }

    const MIN_COMMITS_FOR_RATE: usize = 3;

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

        let commits_per_week_suffix: String = if *count >= MIN_COMMITS_FOR_RATE {
            let email_key = extract_email_key(author_display);
            email_key
                .and_then(|email| {
                    let latest_ts = latest_commit_ts_by_author.get(email)?;
                    let oldest_ts = oldest_commit_ts_by_author.get(email)?;
                    let weeks = active_weeks_inclusive(*latest_ts, *oldest_ts);
                    if weeks <= 0.0 {
                        return None;
                    }
                    let commits_per_week = (*count as f64) / weeks;
                    let span = format_active_span(*latest_ts, *oldest_ts);
                    Some(
                        format!("({commits_per_week:.1}/wk over {span})")
                            .purple()
                            .to_string(),
                    )
                })
                .unwrap_or_default()
        } else {
            String::new()
        };

        if commits_per_week_suffix.is_empty() {
            println!("({count_str}) {colored_author}");
        } else {
            println!("({count_str}) {colored_author} {commits_per_week_suffix}");
        }
    }
}

fn extract_email_key(author_display: &str) -> Option<&str> {
    if let Some(start) = author_display.find('<') {
        let end = author_display.rfind('>')?;
        (start < end).then(|| author_display[start + 1..end].trim())
    } else {
        Some(author_display.trim())
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
        vec!["-c", NO_HOOKS, "fetch", "origin", fetch_target.as_str()],
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

    #[test]
    fn active_weeks_inclusive_counts_single_day_as_one_seventh_week() {
        // Same-day activity => 1 active day => 1/7 week.
        let weeks = active_weeks_inclusive(1_700_000_000, 1_700_000_000);
        assert!((weeks - (1.0 / 7.0)).abs() < 1e-9, "weeks={weeks}");
    }

    #[test]
    fn active_weeks_inclusive_increases_with_span_days() {
        // 8 days inclusive => 8/7 weeks.
        let oldest = 1_700_000_000;
        let latest = oldest + (7 * 86_400);
        let weeks = active_weeks_inclusive(latest, oldest);
        assert!((weeks - (8.0 / 7.0)).abs() < 1e-9, "weeks={weeks}");
    }

    #[test]
    fn format_active_span_uses_months_below_one_year() {
        // 6 months-ish
        let oldest = 1_700_000_000;
        let latest = oldest + (183 * 86_400);
        let span = format_active_span(latest, oldest);
        assert!(
            span.contains("month"),
            "expected months in span, got `{span}`"
        );
    }

    #[test]
    fn format_active_span_uses_years_at_or_above_one_year() {
        // ~400 days
        let oldest = 1_700_000_000;
        let latest = oldest + (399 * 86_400);
        let span = format_active_span(latest, oldest);
        assert!(
            span.contains("year"),
            "expected years in span, got `{span}`"
        );
    }

    fn raw(sha: &str, ts: i64, name: &str, email: &str) -> RawCommit {
        RawCommit {
            sha: sha.to_string(),
            timestamp: ts,
            name: name.to_string(),
            email: email.to_string(),
        }
    }

    #[test]
    fn dedup_commits_no_patch_ids_means_all_unique() {
        let commits = vec![
            raw("aaa", 1000, "Alice", "a@example.com"),
            raw("bbb", 1100, "Bob", "b@example.com"),
            raw("ccc", 1200, "Cara", "c@example.com"),
        ];
        let patch_ids: HashMap<String, String> = HashMap::new();
        let (winners, dups) = dedup_commits(commits.clone(), &patch_ids);
        assert_eq!(dups, 0);
        assert_eq!(winners.len(), 3);
    }

    #[test]
    fn dedup_commits_collapses_shared_patch_id_keeping_earliest() {
        // Two commits share patch-id `pid1`. The earlier author date wins.
        let commits = vec![
            raw("late", 2000, "Alice", "a@example.com"),
            raw("early", 1000, "Bob", "b@example.com"),
            raw("solo", 1500, "Cara", "c@example.com"),
        ];
        let mut patch_ids = HashMap::new();
        patch_ids.insert("late".to_string(), "pid1".to_string());
        patch_ids.insert("early".to_string(), "pid1".to_string());
        patch_ids.insert("solo".to_string(), "pid2".to_string());

        let (mut winners, dups) = dedup_commits(commits, &patch_ids);
        winners.sort_by(|a, b| a.sha.cmp(&b.sha));

        assert_eq!(dups, 1);
        assert_eq!(winners.len(), 2);

        let by_sha: HashMap<&str, &RawCommit> =
            winners.iter().map(|c| (c.sha.as_str(), c)).collect();
        // Earliest-by-timestamp (1000) wins, attributed to Bob.
        assert_eq!(by_sha["early"].name, "Bob");
        // `solo` is unaffected.
        assert_eq!(by_sha["solo"].name, "Cara");
        // `late` was dropped.
        assert!(!by_sha.contains_key("late"));
    }

    #[test]
    fn dedup_commits_tiebreaks_equal_timestamps_by_sha() {
        // Two commits share patch-id AND timestamp. Lower SHA wins for
        // determinism.
        let commits = vec![
            raw("b_sha", 1000, "Alice", "a@example.com"),
            raw("a_sha", 1000, "Bob", "b@example.com"),
        ];
        let mut patch_ids = HashMap::new();
        patch_ids.insert("b_sha".to_string(), "pid1".to_string());
        patch_ids.insert("a_sha".to_string(), "pid1".to_string());

        let (winners, dups) = dedup_commits(commits, &patch_ids);
        assert_eq!(dups, 1);
        assert_eq!(winners.len(), 1);
        assert_eq!(winners[0].sha, "a_sha");
        assert_eq!(winners[0].name, "Bob");
    }

    #[test]
    fn dedup_commits_commits_without_patch_id_are_unique_by_sha() {
        // Two commits with no patch-id mapping (e.g. empty diffs or
        // merge commits) must NOT collapse together even though they
        // share author/timestamp.
        let commits = vec![
            raw("merge1", 1000, "Alice", "a@example.com"),
            raw("merge2", 1000, "Alice", "a@example.com"),
        ];
        let patch_ids: HashMap<String, String> = HashMap::new();
        let (winners, dups) = dedup_commits(commits, &patch_ids);
        assert_eq!(dups, 0);
        assert_eq!(winners.len(), 2);
    }

    #[test]
    fn dedup_commits_mixes_patch_id_and_unmapped_commits() {
        // A patch-id group of 3, plus an unrelated solo commit, plus an
        // unmapped (no patch-id) commit. Result: 1 winner from the
        // group + the solo + the unmapped = 3 winners, 2 duplicates.
        let commits = vec![
            raw("x1", 3000, "Alice", "a@example.com"),
            raw("x2", 1000, "Bob", "b@example.com"),
            raw("x3", 2000, "Cara", "c@example.com"),
            raw("solo", 1500, "Dan", "d@example.com"),
            raw("nopatch", 2500, "Eve", "e@example.com"),
        ];
        let mut patch_ids = HashMap::new();
        patch_ids.insert("x1".to_string(), "pid1".to_string());
        patch_ids.insert("x2".to_string(), "pid1".to_string());
        patch_ids.insert("x3".to_string(), "pid1".to_string());
        patch_ids.insert("solo".to_string(), "pid2".to_string());
        // `nopatch` intentionally not in the map.

        let (mut winners, dups) = dedup_commits(commits, &patch_ids);
        winners.sort_by(|a, b| a.sha.cmp(&b.sha));

        assert_eq!(dups, 2);
        assert_eq!(winners.len(), 3);

        let shas: Vec<&str> = winners.iter().map(|c| c.sha.as_str()).collect();
        assert!(
            shas.contains(&"x2"),
            "earliest (1000) of pid1 group should win, got {shas:?}"
        );
        assert!(shas.contains(&"solo"));
        assert!(shas.contains(&"nopatch"));
    }
}
