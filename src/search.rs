use crate::models::CommandNote;
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;

pub struct SearchEngine {
    matcher: SkimMatcherV2,
}

impl Default for SearchEngine {
    fn default() -> Self {
        Self {
            matcher: SkimMatcherV2::default().ignore_case(),
        }
    }
}

impl SearchEngine {
    /// Score a single command against the query. Returns a positive integer or 0 (no match).
    pub fn score(&self, query: &str, c: &CommandNote, category_name: &str) -> i64 {
        if query.is_empty() {
            return 1; // accept all if no query
        }
        let q = query.trim();
        if q.is_empty() {
            return 1;
        }

        let mut total: i64 = 0;

        if let Some(s) = self.matcher.fuzzy_match(&c.title, q) {
            total += s * 10;
        }
        for tag in &c.tags {
            if let Some(s) = self.matcher.fuzzy_match(tag, q) {
                total += s * 8;
            }
        }
        if let Some(s) = self.matcher.fuzzy_match(&c.command, q) {
            total += s * 6;
        }
        if let Some(s) = self.matcher.fuzzy_match(category_name, q) {
            total += s * 4;
        }
        if !c.description.is_empty() {
            if let Some(s) = self.matcher.fuzzy_match(&c.description, q) {
                total += s * 2;
            }
        }

        // Exact / prefix bonus
        let q_lower = q.to_lowercase();
        if c.title.to_lowercase() == q_lower {
            total += 10_000;
        } else if c.title.to_lowercase().starts_with(&q_lower) {
            total += 1_000;
        }
        if c.command.to_lowercase().starts_with(&q_lower) {
            total += 500;
        }

        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::CommandNote;

    fn note(title: &str, command: &str, tags: &[&str]) -> CommandNote {
        let mut n = CommandNote::new(title.into(), command.into());
        n.tags = tags.iter().map(|s| s.to_string()).collect();
        n
    }

    #[test]
    fn empty_query_accepts_all() {
        let e = SearchEngine::default();
        let n = note("Anything", "cmd", &[]);
        assert!(e.score("", &n, "Cat") > 0);
        assert!(e.score("   ", &n, "Cat") > 0);
    }

    #[test]
    fn title_outweighs_command() {
        let e = SearchEngine::default();
        let title_match = note("docker logs", "ls", &[]);
        let command_match = note("foo", "docker logs", &[]);
        let st = e.score("docker", &title_match, "Other");
        let sc = e.score("docker", &command_match, "Other");
        assert!(st > sc, "title hit should rank higher than command hit ({st} vs {sc})");
    }

    #[test]
    fn exact_title_match_dominates() {
        let e = SearchEngine::default();
        let exact = note("docker ps", "x", &[]);
        let other = note("zzz docker ps zzz", "x", &[]);
        let s_exact = e.score("docker ps", &exact, "Other");
        let s_other = e.score("docker ps", &other, "Other");
        assert!(s_exact > s_other + 5_000, "exact match deserves a big bonus");
    }

    #[test]
    fn prefix_match_outranks_middle() {
        let e = SearchEngine::default();
        let prefix = note("git remote add", "git x", &[]);
        let middle = note("aaa git remote", "git y", &[]);
        let sp = e.score("git", &prefix, "Other");
        let sm = e.score("git", &middle, "Other");
        assert!(sp > sm, "prefix hit should beat middle hit");
    }

    #[test]
    fn tag_match_scores() {
        let e = SearchEngine::default();
        let n = note("a", "b", &["cleanup", "docker"]);
        let s = e.score("cleanup", &n, "Other");
        assert!(s > 0, "tag should be considered");
    }

    #[test]
    fn category_match_scores() {
        let e = SearchEngine::default();
        let n = note("a", "b", &[]);
        let s = e.score("docker", &n, "Docker");
        assert!(s > 0, "category should be considered");
    }

    #[test]
    fn unrelated_query_scores_zero() {
        let e = SearchEngine::default();
        let n = note("docker ps", "docker ps", &["docker"]);
        let s = e.score("xxxxyyyyzzzz", &n, "Other");
        assert_eq!(s, 0);
    }

    #[test]
    fn fuzzy_short_acronym_matches() {
        let e = SearchEngine::default();
        let n = note("docker ps", "docker ps", &[]);
        let s = e.score("dkps", &n, "Other");
        assert!(s > 0, "fuzzy acronym dkps should match docker ps");
    }
}
