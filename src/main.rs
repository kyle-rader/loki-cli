use std::process::Command;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[clap(version, about, author)]
struct Cli {
    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new branch and push it to origin.
    #[clap(visible_alias = "n")]
    New { name: Vec<String> },
}

fn main() {
    let cli = Cli::parse();

    match &cli.command {
        Commands::New { name } => new_branch(name),
    }
}

fn new_branch(name: &Vec<String>) {
    if name.len() == 0 {
        eprintln!("We need some names!");
        std::process::exit(1);
    }

    let name = name.join("-");

    Command::new("git")
        .arg("switch")
        .arg("--create")
        .arg(name.clone())
        .status()
        .expect("Failed to create branch!");

    Command::new("git")
        .arg("push")
        .arg("--set-upstream")
        .arg("origin")
        .arg(name)
        .status()
        .expect("Failed to push new branch!");
}
