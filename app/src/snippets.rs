//! Snippets: built-in SQL templates plus the user's own (`<config>/snippets.toml`), inserted at
//! the caret from Edit ▸ Insert Snippet.

use rsqldev_core::config;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wxdragon::prelude::*;

const BUILT_IN: &[(&str, &str)] = &[
    ("Select: first rows", "SELECT *\nFROM table_name\nFETCH FIRST 100 ROWS ONLY;"),
    ("Select: count by group", "SELECT col, COUNT(*)\nFROM table_name\nGROUP BY col\nORDER BY 2 DESC;"),
    ("Select: duplicates", "SELECT col, COUNT(*)\nFROM table_name\nGROUP BY col\nHAVING COUNT(*) > 1;"),
    ("With (CTE)", "WITH q AS (\n  SELECT *\n  FROM table_name\n)\nSELECT *\nFROM q;"),
    ("Insert", "INSERT INTO table_name (col1, col2)\nVALUES (val1, val2);"),
    ("Update", "UPDATE table_name\nSET col1 = val1\nWHERE id = :id;"),
    ("Merge", "MERGE INTO target t\nUSING source s\nON (t.id = s.id)\nWHEN MATCHED THEN UPDATE SET t.col = s.col\nWHEN NOT MATCHED THEN INSERT (id, col) VALUES (s.id, s.col);"),
    ("Create table", "CREATE TABLE table_name (\n  id         NUMBER GENERATED ALWAYS AS IDENTITY PRIMARY KEY,\n  name       VARCHAR2(100) NOT NULL,\n  created_at DATE DEFAULT SYSDATE\n);"),
    ("PL/SQL: anonymous block", "DECLARE\n  v_count NUMBER;\nBEGIN\n  NULL;\nEXCEPTION\n  WHEN OTHERS THEN\n    DBMS_OUTPUT.PUT_LINE(SQLERRM);\n    RAISE;\nEND;\n/"),
    ("PL/SQL: cursor loop", "BEGIN\n  FOR r IN (SELECT * FROM table_name) LOOP\n    DBMS_OUTPUT.PUT_LINE(r.id);\n  END LOOP;\nEND;\n/"),
    ("Oracle: table sizes", "SELECT segment_name, segment_type, ROUND(SUM(bytes) / 1024 / 1024, 1) AS mb\nFROM user_segments\nGROUP BY segment_name, segment_type\nORDER BY mb DESC;"),
    ("Oracle: tablespace usage", "SELECT tablespace_name, ROUND(used_space * 8192 / 1024 / 1024) AS used_mb,\n       ROUND(tablespace_size * 8192 / 1024 / 1024) AS size_mb, ROUND(used_percent, 1) AS pct\nFROM dba_tablespace_usage_metrics\nORDER BY pct DESC;"),
    ("Oracle: invalid objects", "SELECT object_type, object_name, last_ddl_time\nFROM user_objects\nWHERE status = 'INVALID'\nORDER BY object_type, object_name;"),
    ("Oracle: recompile schema", "BEGIN\n  DBMS_UTILITY.COMPILE_SCHEMA(schema => USER, compile_all => FALSE);\nEND;\n/"),
    ("Oracle: gather table stats", "BEGIN\n  DBMS_STATS.GATHER_TABLE_STATS(ownname => USER, tabname => 'TABLE_NAME');\nEND;\n/"),
    ("Oracle: object DDL", "SELECT DBMS_METADATA.GET_DDL('TABLE', 'TABLE_NAME') FROM dual;"),
    ("Oracle: running SQL", "SELECT s.sid, s.serial#, s.username, s.status, s.event, q.sql_text\nFROM v$session s\nLEFT JOIN v$sql q ON q.sql_id = s.sql_id AND q.child_number = s.sql_child_number\nWHERE s.type = 'USER' AND s.status = 'ACTIVE';"),
];

#[derive(Serialize, Deserialize, Default)]
struct Mine {
    #[serde(default)]
    snippets: BTreeMap<String, String>,
}

fn path() -> std::path::PathBuf {
    config::config_root().join("snippets.toml")
}

fn load() -> Mine {
    std::fs::read_to_string(path()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
}

/// Pick a snippet and insert it at the caret (replacing the selection).
pub fn insert(parent: &dyn WxWidget, e: StyledTextCtrl) {
    let mine = load().snippets;
    let all: Vec<(String, String)> = mine.iter().map(|(k, v)| (format!("★ {k}"), v.clone())).chain(BUILT_IN.iter().map(|(k, v)| (k.to_string(), v.to_string()))).collect();
    let labels: Vec<&str> = all.iter().map(|(k, _)| k.as_str()).collect();
    let d = SingleChoiceDialog::builder(parent, "Snippet to insert (★ = yours, kept in snippets.toml):", "Insert Snippet", &labels).build();
    if d.show_modal() == ID_OK && d.get_selection() >= 0 {
        e.replace_selection(&all[d.get_selection() as usize].1);
        e.set_focus();
    }
}

/// Save the editor selection as a named snippet of the user's.
pub fn save_selection(parent: &dyn WxWidget, e: StyledTextCtrl) {
    let sql = e.get_selected_text();
    if sql.trim().is_empty() {
        return crate::ui::info(parent, "Select the SQL to keep as a snippet first.");
    }
    let Some(name) = crate::ui::ask(parent, "Snippet name:", "Save as Snippet", "").filter(|n| !n.trim().is_empty()) else { return };
    let mut m = load();
    m.snippets.insert(name.trim().to_string(), sql.trim_end().to_string());
    let r = toml::to_string(&m).map_err(|e| e.to_string()).and_then(|s| {
        std::fs::create_dir_all(config::config_root()).and_then(|_| std::fs::write(path(), s)).map_err(|e| e.to_string())
    });
    if let Err(err) = r {
        crate::ui::error(parent, &format!("Saving snippet failed: {err}"));
    }
}
