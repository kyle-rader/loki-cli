use std::ops::Index;

#[derive(Debug, PartialEq)]
pub enum FetchLine {
    Pruned(String),
    NotPruned,
}

const ORIGIN: &str = "origin/";
const DELETED: &str = " - [deleted]";

impl From<String> for FetchLine {
    fn from(s: String) -> Self {
        if s.starts_with(DELETED) {
            match s.find(ORIGIN) {
                Some(ix) => FetchLine::Pruned(String::from(&s[ix + ORIGIN.len()..])),
                None => FetchLine::NotPruned,
            }
        } else {
            FetchLine::NotPruned
        }
    }
}

#[cfg(test)]
mod prune_tests {
    use super::FetchLine;
    use test_case::test_case;

    #[test]
    fn from_pruned_line() {
        let subject: FetchLine = FetchLine::from(String::from(
            " - [deleted]         (none)     -> origin/command-push",
        ));
        assert_eq!(subject, FetchLine::Pruned(String::from("command-push")));
    }

    #[test_case("remote: Enumerating objects: 81, done.")]
    #[test_case("remote: Counting objects: 100% (81/81), done.")]
    #[test_case("remote: Compressing objects: 100% (41/41), done.")]
    #[test_case("remote: Total 70 (delta 30), reused 57 (delta 21), pack-reused 0")]
    #[test_case("Unpacking objects: 100% (70/70), 17.12 KiB | 36.00 KiB/s, done.")]
    #[test_case("From github.com:kyle-rader/loki-cli")]
    #[test_case("   01c2f3a..e4b40f0  main       -> origin/main")]
    #[test_case(" * [new tag]         loki-cli-0.2.0 -> loki-cli-0.2.0")]
    fn from_not_pruned(input: &str) {
        let line = String::from(input);
        let subject = FetchLine::from(line);
        assert_eq!(subject, FetchLine::NotPruned);
    }
}
