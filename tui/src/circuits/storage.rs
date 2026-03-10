use crate::circuits::types::*;
use anyhow::Result;
use rusqlite::Connection;

/// Create the circuits table if it doesn't exist
pub fn init_circuits_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS circuits (
            id TEXT PRIMARY KEY,
            prompt TEXT NOT NULL,
            interval_secs INTEGER NOT NULL,
            provider TEXT NOT NULL,
            model TEXT NOT NULL,
            permission_mode TEXT NOT NULL DEFAULT 'default',
            kind TEXT NOT NULL DEFAULT 'persistent',
            status TEXT NOT NULL DEFAULT 'active',
            last_run TEXT,
            next_run TEXT,
            total_cost REAL NOT NULL DEFAULT 0.0,
            run_count INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS circuit_runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            circuit_id TEXT NOT NULL REFERENCES circuits(id) ON DELETE CASCADE,
            output TEXT NOT NULL,
            cost REAL NOT NULL DEFAULT 0.0,
            tokens_used INTEGER NOT NULL DEFAULT 0,
            success INTEGER NOT NULL DEFAULT 1,
            completed_at TEXT NOT NULL
        );"
    )?;
    Ok(())
}

/// Insert a new circuit
pub fn insert_circuit(conn: &Connection, circuit: &Circuit) -> Result<()> {
    conn.execute(
        "INSERT INTO circuits (id, prompt, interval_secs, provider, model, permission_mode, kind, status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            circuit.id,
            circuit.prompt,
            circuit.interval_secs,
            circuit.provider,
            circuit.model,
            circuit.permission_mode,
            serde_json::to_string(&circuit.kind)?,
            serde_json::to_string(&circuit.status)?,
            circuit.created_at,
        ],
    )?;
    Ok(())
}

/// List all circuits, optionally filtered by kind
pub fn list_circuits(conn: &Connection, kind: Option<&CircuitKind>) -> Result<Vec<Circuit>> {
    let mut stmt = conn.prepare(
        "SELECT id, prompt, interval_secs, provider, model, permission_mode, kind, status, last_run, next_run, total_cost, run_count, created_at
         FROM circuits ORDER BY created_at DESC"
    )?;

    let circuits = stmt.query_map([], |row| {
        let kind_str: String = row.get(6)?;
        let status_str: String = row.get(7)?;
        Ok(Circuit {
            id: row.get(0)?,
            prompt: row.get(1)?,
            interval_secs: row.get(2)?,
            provider: row.get(3)?,
            model: row.get(4)?,
            permission_mode: row.get(5)?,
            kind: serde_json::from_str(&kind_str).unwrap_or(CircuitKind::InSession),
            status: serde_json::from_str(&status_str).unwrap_or(CircuitStatus::Active),
            last_run: row.get(8)?,
            next_run: row.get(9)?,
            total_cost: row.get(10)?,
            run_count: row.get(11)?,
            created_at: row.get(12)?,
        })
    })?.collect::<Result<Vec<_>, _>>()?;

    match kind {
        Some(k) => Ok(circuits.into_iter().filter(|c| &c.kind == k).collect()),
        None => Ok(circuits),
    }
}

/// Update a circuit's status
pub fn update_circuit_status(conn: &Connection, id: &str, status: &CircuitStatus) -> Result<()> {
    conn.execute(
        "UPDATE circuits SET status = ?1 WHERE id = ?2",
        rusqlite::params![serde_json::to_string(status)?, id],
    )?;
    Ok(())
}

/// Delete a circuit
pub fn delete_circuit(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM circuits WHERE id = ?1", [id])?;
    Ok(())
}

/// Record a circuit run result
pub fn insert_circuit_run(conn: &Connection, run: &CircuitRun) -> Result<()> {
    conn.execute(
        "INSERT INTO circuit_runs (circuit_id, output, cost, tokens_used, success, completed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            run.circuit_id,
            run.output,
            run.cost,
            run.tokens_used,
            run.success as i32,
            run.completed_at,
        ],
    )?;
    // Update the circuit's aggregate stats
    conn.execute(
        "UPDATE circuits SET last_run = ?1, total_cost = total_cost + ?2, run_count = run_count + 1 WHERE id = ?3",
        rusqlite::params![run.completed_at, run.cost, run.circuit_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_circuits_table(&conn).unwrap();
        conn
    }

    fn sample_circuit() -> Circuit {
        Circuit {
            id: "test-1".into(),
            prompt: "check build".into(),
            interval_secs: 300,
            provider: "anthropic".into(),
            model: "claude-sonnet".into(),
            permission_mode: "default".into(),
            kind: CircuitKind::InSession,
            status: CircuitStatus::Active,
            last_run: None,
            next_run: None,
            created_at: "2026-03-10T00:00:00Z".into(),
            total_cost: 0.0,
            run_count: 0,
        }
    }

    #[test]
    fn test_insert_and_list() {
        let conn = test_db();
        let circuit = sample_circuit();
        insert_circuit(&conn, &circuit).unwrap();
        let all = list_circuits(&conn, None).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].prompt, "check build");
    }

    #[test]
    fn test_update_status() {
        let conn = test_db();
        insert_circuit(&conn, &sample_circuit()).unwrap();
        update_circuit_status(&conn, "test-1", &CircuitStatus::Paused).unwrap();
        let all = list_circuits(&conn, None).unwrap();
        assert_eq!(all[0].status, CircuitStatus::Paused);
    }

    #[test]
    fn test_delete_circuit() {
        let conn = test_db();
        insert_circuit(&conn, &sample_circuit()).unwrap();
        delete_circuit(&conn, "test-1").unwrap();
        let all = list_circuits(&conn, None).unwrap();
        assert!(all.is_empty());
    }

    #[test]
    fn test_circuit_run_updates_stats() {
        let conn = test_db();
        insert_circuit(&conn, &sample_circuit()).unwrap();
        let run = CircuitRun {
            circuit_id: "test-1".into(),
            output: "all good".into(),
            cost: 0.005,
            tokens_used: 500,
            completed_at: "2026-03-10T01:00:00Z".into(),
            success: true,
        };
        insert_circuit_run(&conn, &run).unwrap();
        let all = list_circuits(&conn, None).unwrap();
        assert_eq!(all[0].run_count, 1);
        assert!((all[0].total_cost - 0.005).abs() < f64::EPSILON);
    }
}
