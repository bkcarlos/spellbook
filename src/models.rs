use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandNote {
    pub id: i64,
    pub title: String,
    pub command: String,
    pub description: String,
    pub category_id: Option<i64>,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub visit_count: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl CommandNote {
    pub fn new(title: String, command: String) -> Self {
        let now = Utc::now();
        Self {
            id: 0,
            title,
            command,
            description: String::new(),
            category_id: None,
            tags: Vec::new(),
            favorite: false,
            visit_count: 0,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn tags_csv(&self) -> String {
        self.tags.join(",")
    }

    pub fn parse_tags(csv: &str) -> Vec<String> {
        csv.split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Category {
    pub id: i64,
    pub name: String,
    pub sort_order: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportBundle {
    pub version: u32,
    pub exported_at: DateTime<Utc>,
    pub categories: Vec<Category>,
    pub commands: Vec<CommandNote>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_csv_roundtrip() {
        let mut n = CommandNote::new("t".into(), "c".into());
        n.tags = vec!["a".into(), "b".into(), "c".into()];
        assert_eq!(n.tags_csv(), "a,b,c");
        assert_eq!(CommandNote::parse_tags("a,b,c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn parse_tags_trims_whitespace_and_skips_empty() {
        assert_eq!(
            CommandNote::parse_tags(" a , b ,, c ,"),
            vec!["a", "b", "c"]
        );
    }

    #[test]
    fn parse_tags_handles_empty() {
        assert!(CommandNote::parse_tags("").is_empty());
        assert!(CommandNote::parse_tags(",,,").is_empty());
    }

    #[test]
    fn export_bundle_roundtrips_through_json() {
        let bundle = ExportBundle {
            version: 1,
            exported_at: Utc::now(),
            categories: vec![Category {
                id: 1,
                name: "Git".into(),
                sort_order: 0,
            }],
            commands: vec![{
                let mut n = CommandNote::new("Log".into(), "git log".into());
                n.id = 1;
                n.category_id = Some(1);
                n.tags = vec!["git".into()];
                n
            }],
        };
        let json = serde_json::to_string(&bundle).unwrap();
        let back: ExportBundle = serde_json::from_str(&json).unwrap();
        assert_eq!(back.commands.len(), 1);
        assert_eq!(back.commands[0].title, "Log");
        assert_eq!(back.categories[0].name, "Git");
    }

    #[test]
    fn new_command_has_sane_defaults() {
        let n = CommandNote::new("T".into(), "C".into());
        assert_eq!(n.id, 0);
        assert!(!n.favorite);
        assert_eq!(n.visit_count, 0);
        assert!(n.tags.is_empty());
        assert!(n.description.is_empty());
        assert_eq!(n.created_at, n.updated_at);
    }
}
