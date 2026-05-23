use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

// Schema com FTS5 sincronizada via triggers.
// `approved=1` = sinal positivo (auto-alimenta exemplos-{categoria}.md).
// `approved=0` = descartada (futuro: alimenta evitar.md via diff edit).
// `edited` = ai_raw_output != final_output (sinal de que o usuário modificou).
// `category` = string descoberta pela DeepSeek (ex: "fiscal", "vendas", "promocao").
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    raw_input TEXT NOT NULL,
    ai_raw_output TEXT NOT NULL,
    final_output TEXT NOT NULL,
    approved INTEGER NOT NULL DEFAULT 0,
    edited INTEGER NOT NULL DEFAULT 0,
    model TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_entries_approved ON entries(approved);
CREATE INDEX IF NOT EXISTS idx_entries_created_at ON entries(created_at DESC);

CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts USING fts5(
    raw_input, ai_raw_output, final_output,
    content='entries', content_rowid='id'
);

CREATE TRIGGER IF NOT EXISTS entries_ai AFTER INSERT ON entries BEGIN
    INSERT INTO entries_fts(rowid, raw_input, ai_raw_output, final_output)
    VALUES (new.id, new.raw_input, new.ai_raw_output, new.final_output);
END;

CREATE TRIGGER IF NOT EXISTS entries_ad AFTER DELETE ON entries BEGIN
    INSERT INTO entries_fts(entries_fts, rowid, raw_input, ai_raw_output, final_output)
    VALUES ('delete', old.id, old.raw_input, old.ai_raw_output, old.final_output);
END;
"#;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Entry {
    pub id: i64,
    pub raw_input: String,
    pub ai_raw_output: String,
    pub final_output: String,
    pub approved: bool,
    pub edited: bool,
    pub model: String,
    pub created_at: i64,
    pub category: Option<String>,
}

pub struct History {
    conn: Mutex<Connection>,
}

impl History {
    pub fn open(db_path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(SCHEMA)?;
        ensure_category_column(&conn)?;
        ensure_category_index(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn save(
        &self,
        raw_input: &str,
        ai_raw_output: &str,
        final_output: &str,
        approved: bool,
        model: &str,
        category: Option<&str>,
    ) -> anyhow::Result<i64> {
        let edited = ai_raw_output != final_output;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO entries (raw_input, ai_raw_output, final_output, approved, edited, model, created_at, category)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![raw_input, ai_raw_output, final_output, approved, edited, model, now, category],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_recent(&self, limit: usize, approved_only: bool) -> anyhow::Result<Vec<Entry>> {
        let conn = self.conn.lock().unwrap();
        let sql = if approved_only {
            "SELECT id, raw_input, ai_raw_output, final_output, approved, edited, model, created_at, category
             FROM entries WHERE approved = 1 ORDER BY created_at DESC LIMIT ?1"
        } else {
            "SELECT id, raw_input, ai_raw_output, final_output, approved, edited, model, created_at, category
             FROM entries ORDER BY created_at DESC LIMIT ?1"
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([limit as i64], row_to_entry)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Aprovadas mais recentes da categoria dada. Reservado para uso futuro
    /// (search/UI/stats) — a injeção de few-shot agora vem do arquivo .md
    /// no vault, não do SQLite.
    #[allow(dead_code)]
    pub fn list_approved_by_category(
        &self,
        category: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<Entry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, raw_input, ai_raw_output, final_output, approved, edited, model, created_at, category
             FROM entries
             WHERE approved = 1 AND category = ?1
             ORDER BY created_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![category, limit as i64], row_to_entry)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Lista distinct das categorias já vistas (para o prompt de classificação anchorar).
    pub fn list_categories(&self) -> anyhow::Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT category FROM entries
             WHERE category IS NOT NULL AND category != ''
             ORDER BY category ASC",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<Entry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT e.id, e.raw_input, e.ai_raw_output, e.final_output, e.approved, e.edited, e.model, e.created_at, e.category
             FROM entries_fts f
             JOIN entries e ON e.id = f.rowid
             WHERE entries_fts MATCH ?1
             ORDER BY e.created_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![query, limit as i64], row_to_entry)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn delete(&self, id: i64) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM entries WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Aprovadas QUE foram editadas (sinal de "a IA gerou mas eu mudei").
    /// Estes são os pares que alimentam a análise de padrões para sugestões ao evitar.md.
    pub fn list_edited_approved(&self, limit: usize) -> anyhow::Result<Vec<Entry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, raw_input, ai_raw_output, final_output, approved, edited, model, created_at, category
             FROM entries
             WHERE approved = 1 AND edited = 1
             ORDER BY created_at DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], row_to_entry)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn count_edited_approved(&self) -> anyhow::Result<usize> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM entries WHERE approved = 1 AND edited = 1",
            [],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    /// Aprovadas SEM edição (a IA acertou de cara). Sinal positivo puro,
    /// usado pela síntese de estilo.md (#19) como amostra de referência.
    pub fn list_approved_unedited(&self, limit: usize) -> anyhow::Result<Vec<Entry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, raw_input, ai_raw_output, final_output, approved, edited, model, created_at, category
             FROM entries
             WHERE approved = 1 AND edited = 0
             ORDER BY created_at DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], row_to_entry)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn count_approved_unedited(&self) -> anyhow::Result<usize> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM entries WHERE approved = 1 AND edited = 0",
            [],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    pub fn count(&self, approved_only: bool) -> anyhow::Result<usize> {
        let conn = self.conn.lock().unwrap();
        let sql = if approved_only {
            "SELECT COUNT(*) FROM entries WHERE approved = 1"
        } else {
            "SELECT COUNT(*) FROM entries"
        };
        let n: i64 = conn.query_row(sql, [], |r| r.get(0))?;
        Ok(n as usize)
    }
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: row.get(0)?,
        raw_input: row.get(1)?,
        ai_raw_output: row.get(2)?,
        final_output: row.get(3)?,
        approved: row.get(4)?,
        edited: row.get(5)?,
        model: row.get(6)?,
        created_at: row.get(7)?,
        category: row.get(8)?,
    })
}

/// Migração idempotente: adiciona a coluna `category` se ainda não existir.
/// Necessário porque o schema original (sessão anterior) não tinha esse campo.
fn ensure_category_column(conn: &Connection) -> rusqlite::Result<()> {
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('entries') WHERE name = 'category'",
        [],
        |r| r.get(0),
    )?;
    if exists == 0 {
        conn.execute("ALTER TABLE entries ADD COLUMN category TEXT", [])?;
        tracing::info!("migração SQLite: coluna `category` adicionada");
    }
    Ok(())
}

fn ensure_category_index(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_entries_category ON entries(category, approved, created_at DESC)",
        [],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_history() -> History {
        let dir = std::env::temp_dir().join(format!(
            "artemis_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        History::open(&dir.join("test.db")).unwrap()
    }

    #[test]
    fn save_marks_edited_correctly() {
        let h = temp_history();
        let id1 = h
            .save("input", "saida-ia", "saida-ia", true, "deepseek", Some("fiscal"))
            .unwrap();
        let id2 = h
            .save("input", "saida-ia", "saida-editada", true, "deepseek", Some("fiscal"))
            .unwrap();
        let entries = h.list_recent(10, false).unwrap();
        assert_eq!(entries.len(), 2);
        let e1 = entries.iter().find(|e| e.id == id1).unwrap();
        let e2 = entries.iter().find(|e| e.id == id2).unwrap();
        assert!(!e1.edited);
        assert!(e2.edited);
    }

    #[test]
    fn list_by_category_filters_correctly() {
        let h = temp_history();
        h.save("a", "b", "b", true, "m", Some("fiscal")).unwrap();
        h.save("c", "d", "d", true, "m", Some("vendas")).unwrap();
        h.save("e", "f", "f", true, "m", Some("fiscal")).unwrap();
        h.save("g", "h", "h", false, "m", Some("fiscal")).unwrap(); // not approved

        let fiscal = h.list_approved_by_category("fiscal", 10).unwrap();
        assert_eq!(fiscal.len(), 2);
        assert!(fiscal.iter().all(|e| e.category.as_deref() == Some("fiscal")));
        assert!(fiscal.iter().all(|e| e.approved));
    }

    #[test]
    fn list_categories_distinct() {
        let h = temp_history();
        h.save("a", "b", "b", true, "m", Some("fiscal")).unwrap();
        h.save("c", "d", "d", true, "m", Some("vendas")).unwrap();
        h.save("e", "f", "f", true, "m", Some("fiscal")).unwrap();
        let cats = h.list_categories().unwrap();
        assert_eq!(cats, vec!["fiscal", "vendas"]);
    }

    #[test]
    fn list_approved_unedited_filters_correctly() {
        let h = temp_history();
        // aprovada, sem edição → entra
        let id_ok = h
            .save("in1", "saida", "saida", true, "m", Some("fiscal"))
            .unwrap();
        // aprovada, mas editada → fora
        h.save("in2", "saida-ia", "saida-final", true, "m", Some("fiscal"))
            .unwrap();
        // descartada (mesmo sem edição) → fora
        h.save("in3", "x", "x", false, "m", Some("fiscal")).unwrap();

        let list = h.list_approved_unedited(10).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id_ok);
        assert!(list[0].approved);
        assert!(!list[0].edited);
    }

    #[test]
    fn count_approved_unedited_matches_list() {
        let h = temp_history();
        h.save("a", "x", "x", true, "m", Some("fiscal")).unwrap();
        h.save("b", "y", "y", true, "m", Some("vendas")).unwrap();
        h.save("c", "z", "z2", true, "m", None).unwrap(); // editada — fora
        h.save("d", "w", "w", false, "m", None).unwrap(); // descartada — fora

        let n = h.count_approved_unedited().unwrap();
        let list = h.list_approved_unedited(usize::MAX).unwrap();
        assert_eq!(n, list.len());
        assert_eq!(n, 2);
    }
}
