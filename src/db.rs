use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::PathBuf;

use crate::models::{Category, CommandNote};

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open() -> Result<Self> {
        let path = data_dir()?.join("spellbook.sqlite");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("create data dir")?;
        }
        let conn = Connection::open(&path).context("open sqlite")?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        let db = Self { conn };
        db.migrate()?;
        db.seed_default_categories()?;
        Ok(db)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS category (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                sort_order INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS command_note (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                command TEXT NOT NULL,
                description TEXT DEFAULT '',
                category_id INTEGER REFERENCES category(id) ON DELETE SET NULL,
                tags TEXT DEFAULT '',
                favorite INTEGER DEFAULT 0,
                visit_count INTEGER DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_cmd_category ON command_note(category_id);
            CREATE INDEX IF NOT EXISTS idx_cmd_favorite ON command_note(favorite);
            CREATE INDEX IF NOT EXISTS idx_cmd_updated  ON command_note(updated_at DESC);
            "#,
        )?;
        // Add the soft-delete column if missing. SQLite has no IF NOT EXISTS for
        // ADD COLUMN; we tolerate the duplicate-column error.
        let _ = self
            .conn
            .execute("ALTER TABLE command_note ADD COLUMN deleted_at TEXT", []);
        self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_cmd_deleted ON command_note(deleted_at);",
        )?;
        Ok(())
    }

    fn seed_default_categories(&self) -> Result<()> {
        let count: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM category", [], |r| r.get(0))?;
        if count > 0 {
            return Ok(());
        }
        let defaults = [
            "Git",
            "Docker",
            "Kubernetes",
            "Linux",
            "SSH",
            "Database",
            "NodeJS",
            "Python",
            "Rust",
            "Go",
            "Media",
            "HTTP",
            "System",
            "Package",
            "Other",
        ];
        for (i, name) in defaults.iter().enumerate() {
            self.conn.execute(
                "INSERT INTO category (name, sort_order) VALUES (?1, ?2)",
                params![name, i as i32],
            )?;
        }
        Ok(())
    }

    // ---------- Categories ----------

    pub fn list_categories(&self) -> Result<Vec<Category>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, sort_order FROM category ORDER BY sort_order, name",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Category {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    sort_order: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn add_category(&self, name: &str) -> Result<i64> {
        let next_order: i32 = self
            .conn
            .query_row("SELECT COALESCE(MAX(sort_order), 0) + 1 FROM category", [], |r| {
                r.get(0)
            })?;
        self.conn.execute(
            "INSERT INTO category (name, sort_order) VALUES (?1, ?2)",
            params![name, next_order],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    #[allow(dead_code)]
    pub fn rename_category(&self, id: i64, new_name: &str) -> Result<()> {
        self.conn
            .execute("UPDATE category SET name = ?1 WHERE id = ?2", params![new_name, id])?;
        Ok(())
    }

    pub fn delete_category(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM category WHERE id = ?1", params![id])?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn find_category_by_name(&self, name: &str) -> Result<Option<Category>> {
        let row = self
            .conn
            .query_row(
                "SELECT id, name, sort_order FROM category WHERE LOWER(name) = LOWER(?1)",
                params![name],
                |row| {
                    Ok(Category {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        sort_order: row.get(2)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    pub fn category_counts(&self) -> Result<std::collections::HashMap<i64, i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT category_id, COUNT(*) FROM command_note
             WHERE category_id IS NOT NULL AND deleted_at IS NULL
             GROUP BY category_id",
        )?;
        let mut map = std::collections::HashMap::new();
        for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
            let (cid, c) = row?;
            map.insert(cid, c);
        }
        Ok(map)
    }

    // ---------- Commands ----------

    pub fn list_commands(&self) -> Result<Vec<CommandNote>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, command, description, category_id, tags, favorite, visit_count, created_at, updated_at
             FROM command_note
             WHERE deleted_at IS NULL
             ORDER BY updated_at DESC",
        )?;
        let rows = stmt
            .query_map([], row_to_command)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn list_trashed(&self) -> Result<Vec<(CommandNote, chrono::DateTime<chrono::Utc>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, command, description, category_id, tags, favorite, visit_count, created_at, updated_at, deleted_at
             FROM command_note
             WHERE deleted_at IS NOT NULL
             ORDER BY deleted_at DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                let note = row_to_command(row)?;
                let deleted_at: String = row.get(10)?;
                Ok((note, parse_rfc3339(&deleted_at)))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn soft_delete_command(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE command_note SET deleted_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), id],
        )?;
        Ok(())
    }

    pub fn restore_command(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE command_note SET deleted_at = NULL, updated_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), id],
        )?;
        Ok(())
    }

    /// Purge commands deleted more than `days` ago. Returns count removed.
    pub fn purge_old_deleted(&self, days: i64) -> Result<usize> {
        let cutoff = Utc::now() - chrono::Duration::days(days);
        let n = self.conn.execute(
            "DELETE FROM command_note WHERE deleted_at IS NOT NULL AND deleted_at < ?1",
            params![cutoff.to_rfc3339()],
        )?;
        Ok(n)
    }

    #[allow(dead_code)]
    pub fn count_trashed(&self) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM command_note WHERE deleted_at IS NOT NULL",
            [],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    pub fn insert_command(&self, c: &CommandNote) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO command_note (title, command, description, category_id, tags, favorite, visit_count, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                c.title,
                c.command,
                c.description,
                c.category_id,
                c.tags_csv(),
                c.favorite as i32,
                c.visit_count,
                c.created_at.to_rfc3339(),
                c.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_command(&self, c: &CommandNote) -> Result<()> {
        self.conn.execute(
            "UPDATE command_note
             SET title = ?1, command = ?2, description = ?3, category_id = ?4, tags = ?5,
                 favorite = ?6, visit_count = ?7, updated_at = ?8
             WHERE id = ?9",
            params![
                c.title,
                c.command,
                c.description,
                c.category_id,
                c.tags_csv(),
                c.favorite as i32,
                c.visit_count,
                Utc::now().to_rfc3339(),
                c.id,
            ],
        )?;
        Ok(())
    }

    pub fn delete_command(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM command_note WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn bump_visit(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE command_note SET visit_count = visit_count + 1, updated_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), id],
        )?;
        Ok(())
    }

    pub fn toggle_favorite(&self, id: i64) -> Result<bool> {
        self.conn.execute(
            "UPDATE command_note SET favorite = 1 - favorite WHERE id = ?1",
            params![id],
        )?;
        let fav: i32 = self.conn.query_row(
            "SELECT favorite FROM command_note WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )?;
        Ok(fav != 0)
    }

    pub fn find_by_exact_command(&self, command: &str) -> Result<Option<CommandNote>> {
        let row = self
            .conn
            .query_row(
                "SELECT id, title, command, description, category_id, tags, favorite, visit_count, created_at, updated_at
                 FROM command_note WHERE command = ?1 AND deleted_at IS NULL LIMIT 1",
                params![command],
                row_to_command,
            )
            .optional()?;
        Ok(row)
    }
}

fn row_to_command(row: &rusqlite::Row) -> rusqlite::Result<CommandNote> {
    let tags: String = row.get(5)?;
    let created: String = row.get(8)?;
    let updated: String = row.get(9)?;
    let fav: i32 = row.get(6)?;
    Ok(CommandNote {
        id: row.get(0)?,
        title: row.get(1)?,
        command: row.get(2)?,
        description: row.get(3)?,
        category_id: row.get(4)?,
        tags: CommandNote::parse_tags(&tags),
        favorite: fav != 0,
        visit_count: row.get(7)?,
        created_at: parse_rfc3339(&created),
        updated_at: parse_rfc3339(&updated),
    })
}

fn parse_rfc3339(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

pub fn data_dir() -> Result<PathBuf> {
    let base = dirs::data_dir().context("no data dir for OS")?;
    Ok(base.join("Spellbook"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Db {
        let db = Db::open_in_memory().expect("in-mem db");
        db.seed_default_categories().expect("seed");
        db
    }

    fn make_note(title: &str, command: &str, cat_id: Option<i64>) -> CommandNote {
        let mut n = CommandNote::new(title.into(), command.into());
        n.category_id = cat_id;
        n
    }

    #[test]
    fn seeds_15_default_categories() {
        let db = fresh();
        let cats = db.list_categories().unwrap();
        assert_eq!(cats.len(), 15, "expected 15 seeded categories");
        let names: Vec<&str> = cats.iter().map(|c| c.name.as_str()).collect();
        for must in &["Git", "Docker", "Linux", "Other"] {
            assert!(names.contains(must), "missing {must}");
        }
    }

    #[test]
    fn seed_is_idempotent() {
        let db = fresh();
        db.seed_default_categories().unwrap();
        db.seed_default_categories().unwrap();
        assert_eq!(db.list_categories().unwrap().len(), 15);
    }

    #[test]
    fn add_and_find_category() {
        let db = fresh();
        let id = db.add_category("CustomCat").unwrap();
        let found = db.find_category_by_name("customcat").unwrap();
        assert!(found.is_some(), "lookup should be case-insensitive");
        assert_eq!(found.unwrap().id, id);
    }

    #[test]
    fn add_category_assigns_sort_order() {
        let db = fresh();
        let id = db.add_category("Zebra").unwrap();
        let cat = db.list_categories().unwrap().into_iter().find(|c| c.id == id).unwrap();
        assert!(cat.sort_order > 0);
    }

    #[test]
    fn rename_category_works() {
        let db = fresh();
        let id = db.add_category("OldName").unwrap();
        db.rename_category(id, "NewName").unwrap();
        assert!(db.find_category_by_name("NewName").unwrap().is_some());
        assert!(db.find_category_by_name("OldName").unwrap().is_none());
    }

    #[test]
    fn insert_and_list_commands() {
        let db = fresh();
        let docker = db.find_category_by_name("Docker").unwrap().unwrap().id;
        let mut n = make_note("List", "docker ps", Some(docker));
        n.tags = vec!["docker".into(), "list".into()];
        let id = db.insert_command(&n).unwrap();
        assert!(id > 0);
        let all = db.list_commands().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].title, "List");
        assert_eq!(all[0].tags, vec!["docker", "list"]);
        assert_eq!(all[0].category_id, Some(docker));
    }

    #[test]
    fn update_command_persists_changes() {
        let db = fresh();
        let id = db.insert_command(&make_note("A", "ls", None)).unwrap();
        let mut n = db.list_commands().unwrap().into_iter().find(|c| c.id == id).unwrap();
        n.title = "B".into();
        n.description = "new desc".into();
        n.tags = vec!["x".into()];
        db.update_command(&n).unwrap();
        let after = db.list_commands().unwrap().into_iter().find(|c| c.id == id).unwrap();
        assert_eq!(after.title, "B");
        assert_eq!(after.description, "new desc");
        assert_eq!(after.tags, vec!["x"]);
    }

    #[test]
    fn delete_command_removes_it() {
        let db = fresh();
        let id = db.insert_command(&make_note("X", "ls", None)).unwrap();
        db.delete_command(id).unwrap();
        assert!(db.list_commands().unwrap().is_empty());
    }

    #[test]
    fn toggle_favorite_flips_state() {
        let db = fresh();
        let id = db.insert_command(&make_note("X", "ls", None)).unwrap();
        assert_eq!(db.toggle_favorite(id).unwrap(), true);
        assert_eq!(db.toggle_favorite(id).unwrap(), false);
    }

    #[test]
    fn bump_visit_increments() {
        let db = fresh();
        let id = db.insert_command(&make_note("X", "ls", None)).unwrap();
        for _ in 0..3 {
            db.bump_visit(id).unwrap();
        }
        let n = db.list_commands().unwrap().into_iter().find(|c| c.id == id).unwrap();
        assert_eq!(n.visit_count, 3);
    }

    #[test]
    fn find_by_exact_command_is_exact() {
        let db = fresh();
        db.insert_command(&make_note("A", "docker ps", None)).unwrap();
        assert!(db.find_by_exact_command("docker ps").unwrap().is_some());
        assert!(db.find_by_exact_command("docker p").unwrap().is_none());
        assert!(db.find_by_exact_command("docker ps -a").unwrap().is_none());
    }

    #[test]
    fn category_counts_aggregate() {
        let db = fresh();
        let docker = db.find_category_by_name("Docker").unwrap().unwrap().id;
        let git = db.find_category_by_name("Git").unwrap().unwrap().id;
        for c in ["docker ps", "docker images", "docker logs"] {
            db.insert_command(&make_note("t", c, Some(docker))).unwrap();
        }
        db.insert_command(&make_note("t", "git log", Some(git))).unwrap();
        let counts = db.category_counts().unwrap();
        assert_eq!(counts.get(&docker).copied(), Some(3));
        assert_eq!(counts.get(&git).copied(), Some(1));
    }

    #[test]
    fn deleting_category_nullifies_commands() {
        let db = fresh();
        let cid = db.add_category("Temp").unwrap();
        let nid = db.insert_command(&make_note("t", "x", Some(cid))).unwrap();
        db.delete_category(cid).unwrap();
        let n = db.list_commands().unwrap().into_iter().find(|c| c.id == nid).unwrap();
        assert_eq!(n.category_id, None, "FK should null out on delete");
    }

    #[test]
    fn tags_roundtrip_through_storage() {
        let db = fresh();
        let mut n = make_note("t", "x", None);
        n.tags = vec!["a, b".into(), "c".into()]; // tag containing comma should be sanitized on parse
        let id = db.insert_command(&n).unwrap();
        let back = db.list_commands().unwrap().into_iter().find(|c| c.id == id).unwrap();
        // storage is csv; parse_tags splits on comma, so "a, b" comes back as ["a", "b", "c"]
        assert_eq!(back.tags, vec!["a", "b", "c"]);
    }

    #[test]
    fn description_defaults_to_empty() {
        let db = fresh();
        let id = db.insert_command(&make_note("t", "x", None)).unwrap();
        let n = db.list_commands().unwrap().into_iter().find(|c| c.id == id).unwrap();
        assert_eq!(n.description, "");
    }

    #[test]
    fn soft_delete_hides_from_list() {
        let db = fresh();
        let id = db.insert_command(&make_note("X", "ls", None)).unwrap();
        db.soft_delete_command(id).unwrap();
        assert!(db.list_commands().unwrap().is_empty());
        assert_eq!(db.count_trashed().unwrap(), 1);
    }

    #[test]
    fn restore_brings_back() {
        let db = fresh();
        let id = db.insert_command(&make_note("X", "ls", None)).unwrap();
        db.soft_delete_command(id).unwrap();
        db.restore_command(id).unwrap();
        assert_eq!(db.list_commands().unwrap().len(), 1);
        assert_eq!(db.count_trashed().unwrap(), 0);
    }

    #[test]
    fn find_by_exact_ignores_trashed() {
        let db = fresh();
        let id = db.insert_command(&make_note("X", "ls -la", None)).unwrap();
        db.soft_delete_command(id).unwrap();
        assert!(db.find_by_exact_command("ls -la").unwrap().is_none());
    }

    #[test]
    fn category_counts_ignore_trashed() {
        let db = fresh();
        let docker = db.find_category_by_name("Docker").unwrap().unwrap().id;
        let a = db.insert_command(&make_note("a", "docker ps", Some(docker))).unwrap();
        db.insert_command(&make_note("b", "docker logs", Some(docker))).unwrap();
        db.soft_delete_command(a).unwrap();
        let counts = db.category_counts().unwrap();
        assert_eq!(counts.get(&docker).copied(), Some(1));
    }

    #[test]
    fn purge_removes_only_old() {
        let db = fresh();
        let id = db.insert_command(&make_note("X", "ls", None)).unwrap();
        db.soft_delete_command(id).unwrap();
        // Just deleted — should not be purged
        let n = db.purge_old_deleted(7).unwrap();
        assert_eq!(n, 0);
        assert_eq!(db.count_trashed().unwrap(), 1);
        // Now pretend it's older
        db.conn.execute(
            "UPDATE command_note SET deleted_at = ?1 WHERE id = ?2",
            params![(Utc::now() - chrono::Duration::days(10)).to_rfc3339(), id],
        ).unwrap();
        let n = db.purge_old_deleted(7).unwrap();
        assert_eq!(n, 1);
        assert_eq!(db.count_trashed().unwrap(), 0);
    }

    #[test]
    fn list_orders_by_updated_desc() {
        let db = fresh();
        let a = db.insert_command(&make_note("a", "ls -a", None)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        let b = db.insert_command(&make_note("b", "ls -b", None)).unwrap();
        let order: Vec<i64> = db.list_commands().unwrap().into_iter().map(|c| c.id).collect();
        assert_eq!(order, vec![b, a], "newer should come first");
    }
}
