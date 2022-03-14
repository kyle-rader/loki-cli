#[derive(Debug, PartialEq)]
pub enum FetchLine {
    Pruned(String),
    NotPruned,
}

impl TryFrom<String> for FetchLine {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        todo!()
    }
}

#[cfg(test)]
mod prune_tests {
    use super::*;

    #[test]
    fn try_from_pruned_line() {
        let line = String::from(" - [deleted]         (none)     -> origin/command-push");

        let subject = FetchLine::try_from(line);
        assert_eq!(subject, Ok(FetchLine::Pruned(String::from("command-push"))));
    }
}
