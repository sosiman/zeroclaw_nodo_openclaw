use rusqlite::{params, Connection, Result};
use std::sync::{Arc, Mutex};
use std::path::Path;

#[derive(Clone)]
pub struct JobStore {
    conn: Arc<Mutex<Connection>>,
}

#[derive(Debug, Clone)]
pub struct JobRecord {
    pub id: String,
    pub status: String,
    pub command: String,
    pub args: String,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub exit_code: Option<i32>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
}

impl JobStore {
    pub fn new(db_path: &Path) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        
        conn.execute(
            "CREATE TABLE IF NOT EXISTS jobs (
                id TEXT PRIMARY KEY,
                status TEXT NOT NULL,
                command TEXT NOT NULL,
                args TEXT NOT NULL,
                started_at INTEGER,
                ended_at INTEGER,
                exit_code INTEGER,
                stdout TEXT,
                stderr TEXT
            )",
            [],
        )?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn insert_job(&self, id: &str, command: &str, args: &str, started_at: u64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO jobs (id, status, command, args, started_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, "running", command, args, started_at],
        )?;
        Ok(())
    }

    pub fn update_job_success(&self, id: &str, ended_at: u64, exit_code: i32, stdout: &str, stderr: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE jobs SET status = 'success', ended_at = ?2, exit_code = ?3, stdout = ?4, stderr = ?5 WHERE id = ?1",
            params![id, ended_at, exit_code, stdout, stderr],
        )?;
        Ok(())
    }

    pub fn update_job_error(&self, id: &str, ended_at: u64, error: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE jobs SET status = 'error', ended_at = ?2, stderr = ?3 WHERE id = ?1",
            params![id, ended_at, error],
        )?;
        Ok(())
    }

    pub fn complete_job(&self, id: &str, status: &str, exit_code: i32, stdout_bytes: i64, stderr_bytes: i64) -> Result<()> {
        let ended_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        if status == "success" {
            self.update_job_success(id, ended_at, exit_code, &stdout_bytes.to_string(), &stderr_bytes.to_string())
        } else {
            self.update_job_error(id, ended_at, &format!("exit_code={}", exit_code))
        }
    }

    pub fn get_job(&self, id: &str) -> Result<Option<JobRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id, status, command, args, started_at, ended_at, exit_code, stdout, stderr FROM jobs WHERE id = ?1")?;
        let mut rows = stmt.query(params![id])?;

        if let Some(row) = rows.next()? {
            let ended_at: Option<i64> = row.get(5)?;
            Ok(Some(JobRecord {
                id: row.get(0)?,
                status: row.get(1)?,
                command: row.get(2)?,
                args: row.get(3)?,
                started_at: row.get::<_, i64>(4)? as u64,
                ended_at: ended_at.map(|v| v as u64),
                exit_code: row.get(6)?,
                stdout: row.get(7)?,
                stderr: row.get(8)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn prune_finished_older_than(&self, cutoff_unix_secs: u64) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let changed = conn.execute(
            "DELETE FROM jobs WHERE ended_at IS NOT NULL AND ended_at < ?1",
            params![cutoff_unix_secs as i64],
        )?;
        Ok(changed)
    }
}

