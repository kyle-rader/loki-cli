pub mod git;

use clap::Parser;

#[derive(Parser)]
#[clap(version, about, author)]
enum Cli {
    /// Create a new branch from HEAD and push it to origin.
    #[clap(visible_alias = "n")]
    New {
        /// List of names to join with dashes to form a valid branch name.
        name: Vec<String>,
    },
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

    git::git_commands(vec![
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
