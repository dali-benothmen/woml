//! Database operations for the Node-Cronflow Core Engine

use rusqlite::Connection;
use std::path::Path;
use std::fs;
use crate::error::CoreResult;
use crate::models::{WorkflowDefinition, WorkflowRun, StepResult};

/// Database connection wrapper
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Create a new database connection
    pub fn new(path: &str) -> CoreResult<Self> {
        // Ensure parent directory exists
        if let Some(parent) = Path::new(path).parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }
        
        let conn = Connection::open(path)?;
        let db = Database { conn };
        db.init_schema()?;
        Ok(db)
    }

    /// Initialize database schema
    fn init_schema(&self) -> CoreResult<()> {
        // Read and execute the schema file
        let schema = include_str!("schema.sql");
        self.conn.execute_batch(schema)?;
        Ok(())
    }

    /// Save a workflow definition
    pub fn save_workflow(&self, workflow: &WorkflowDefinition) -> CoreResult<()> {
        let definition = serde_json::to_string(workflow)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO workflows (id, name, description, definition, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
            (
                &workflow.id,
                &workflow.name,
                &workflow.description,
                &definition,
                &workflow.created_at.to_rfc3339(),
                &workflow.updated_at.to_rfc3339(),
            ),
        )?;
        Ok(())
    }

    /// Get a workflow definition by ID
    pub fn get_workflow(&self, id: &str) -> CoreResult<Option<WorkflowDefinition>> {
        let mut stmt = self.conn.prepare(
            "SELECT definition FROM workflows WHERE id = ?"
        )?;
        
        let mut rows = stmt.query([id])?;
        if let Some(row) = rows.next()? {
            let definition: String = row.get(0)?;
            let workflow: WorkflowDefinition = serde_json::from_str(&definition)?;
            Ok(Some(workflow))
        } else {
            Ok(None)
        }
    }

    /// Get all workflows
    pub fn get_all_workflows(&self) -> CoreResult<Vec<WorkflowDefinition>> {
        let mut stmt = self.conn.prepare(
            "SELECT definition FROM workflows ORDER BY created_at DESC"
        )?;
        
        let mut workflows = Vec::new();
        let mut rows = stmt.query([])?;
        
        while let Some(row) = rows.next()? {
            let definition: String = row.get(0)?;
            let workflow: WorkflowDefinition = serde_json::from_str(&definition)?;
            workflows.push(workflow);
        }
        
        Ok(workflows)
    }

    /// Delete a workflow
    pub fn delete_workflow(&self, id: &str) -> CoreResult<()> {
        self.conn.execute("DELETE FROM workflows WHERE id = ?", [id])?;
        Ok(())
    }

    /// Save a workflow run
    pub fn save_run(&self, run: &WorkflowRun) -> CoreResult<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO workflow_runs (id, workflow_id, status, payload, started_at, completed_at, error) VALUES (?, ?, ?, ?, ?, ?, ?)",
            (
                &run.id.to_string(),
                &run.workflow_id,
                &format!("{:?}", run.status),
                &serde_json::to_string(&run.payload)?,
                &run.started_at.to_rfc3339(),
                &run.completed_at.map(|dt| dt.to_rfc3339()),
                &run.error,
            ),
        )?;
        Ok(())
    }

    /// Get a workflow run by ID
    pub fn get_run(&self, run_id: &str) -> CoreResult<Option<WorkflowRun>> {
        let mut stmt = self.conn.prepare(
            "SELECT workflow_id, status, payload, started_at, completed_at, error FROM workflow_runs WHERE id = ?"
        )?;
        
        let mut rows = stmt.query([run_id])?;
        if let Some(row) = rows.next()? {
            let workflow_id: String = row.get(0)?;
            let status_str: String = row.get(1)?;
            let payload_str: String = row.get(2)?;
            let started_at_str: String = row.get(3)?;
            let completed_at_str: Option<String> = row.get(4)?;
            let error: Option<String> = row.get(5)?;
            
            let status = match status_str.as_str() {
                "Pending" => crate::models::RunStatus::Pending,
                "Running" => crate::models::RunStatus::Running,
                "Completed" => crate::models::RunStatus::Completed,
                "Failed" => crate::models::RunStatus::Failed,
                "Cancelled" => crate::models::RunStatus::Cancelled,
                _ => crate::models::RunStatus::Failed,
            };
            
            let started_at = chrono::DateTime::parse_from_rfc3339(&started_at_str)?.with_timezone(&chrono::Utc);
            let completed_at = completed_at_str
                .map(|s| chrono::DateTime::parse_from_rfc3339(&s))
                .transpose()?
                .map(|dt| dt.with_timezone(&chrono::Utc));
            
            let payload = serde_json::from_str(&payload_str)?;
            
            let run = WorkflowRun {
                id: uuid::Uuid::parse_str(run_id)?,
                workflow_id,
                status,
                payload,
                started_at,
                completed_at,
                error,
            };
            
            Ok(Some(run))
        } else {
            Ok(None)
        }
    }

    /// Get runs for a workflow
    pub fn get_runs_for_workflow(&self, workflow_id: &str) -> CoreResult<Vec<WorkflowRun>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, status, payload, started_at, completed_at, error FROM workflow_runs WHERE workflow_id = ? ORDER BY started_at DESC"
        )?;
        
        let mut runs = Vec::new();
        let mut rows = stmt.query([workflow_id])?;
        
        while let Some(row) = rows.next()? {
            let run_id_str: String = row.get(0)?;
            let status_str: String = row.get(1)?;
            let payload_str: String = row.get(2)?;
            let started_at_str: String = row.get(3)?;
            let completed_at_str: Option<String> = row.get(4)?;
            let error: Option<String> = row.get(5)?;
            
            let status = match status_str.as_str() {
                "Pending" => crate::models::RunStatus::Pending,
                "Running" => crate::models::RunStatus::Running,
                "Completed" => crate::models::RunStatus::Completed,
                "Failed" => crate::models::RunStatus::Failed,
                "Cancelled" => crate::models::RunStatus::Cancelled,
                _ => crate::models::RunStatus::Failed,
            };
            
            let started_at = chrono::DateTime::parse_from_rfc3339(&started_at_str)?.with_timezone(&chrono::Utc);
            let completed_at = completed_at_str
                .map(|s| chrono::DateTime::parse_from_rfc3339(&s))
                .transpose()?
                .map(|dt| dt.with_timezone(&chrono::Utc));
            
            let payload = serde_json::from_str(&payload_str)?;
            
            let run = WorkflowRun {
                id: uuid::Uuid::parse_str(&run_id_str)?,
                workflow_id: workflow_id.to_string(),
                status,
                payload,
                started_at,
                completed_at,
                error,
            };
            
            runs.push(run);
        }
        
        Ok(runs)
    }

    /// Save a step result
    pub fn save_step_result(&self, result: &StepResult, run_id: &str) -> CoreResult<()> {
        self.conn.execute(
            "INSERT INTO step_results (run_id, step_id, status, output, error, started_at, completed_at, duration_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            (
                run_id,
                &result.step_id,
                &format!("{:?}", result.status),
                &result.output.as_ref().map(|v| serde_json::to_string(v)).transpose()?,
                &result.error,
                &result.started_at.to_rfc3339(),
                &result.completed_at.map(|dt| dt.to_rfc3339()),
                &result.duration_ms,
            ),
        )?;
        Ok(())
    }

    /// Get step results for a run
    pub fn get_step_results(&self, run_id: &str) -> CoreResult<Vec<StepResult>> {
        let mut stmt = self.conn.prepare(
            "SELECT step_id, status, output, error, started_at, completed_at, duration_ms FROM step_results WHERE run_id = ? ORDER BY started_at ASC"
        )?;
        
        let mut results = Vec::new();
        let mut rows = stmt.query([run_id])?;
        
        while let Some(row) = rows.next()? {
            let step_id: String = row.get(0)?;
            let status_str: String = row.get(1)?;
            let output_str: Option<String> = row.get(2)?;
            let error: Option<String> = row.get(3)?;
            let started_at_str: String = row.get(4)?;
            let completed_at_str: Option<String> = row.get(5)?;
            let duration_ms: Option<u64> = row.get(6)?;
            
            let status = match status_str.as_str() {
                "Pending" => crate::models::StepStatus::Pending,
                "Running" => crate::models::StepStatus::Running,
                "Completed" => crate::models::StepStatus::Completed,
                "Failed" => crate::models::StepStatus::Failed,
                "Skipped" => crate::models::StepStatus::Skipped,
                _ => crate::models::StepStatus::Failed,
            };
            
            let started_at = chrono::DateTime::parse_from_rfc3339(&started_at_str)?.with_timezone(&chrono::Utc);
            let completed_at = completed_at_str
                .map(|s| chrono::DateTime::parse_from_rfc3339(&s))
                .transpose()?
                .map(|dt| dt.with_timezone(&chrono::Utc));
            
            let output = output_str
                .map(|s| serde_json::from_str(&s))
                .transpose()?;
            
            let result = StepResult {
                step_id,
                status,
                output,
                error,
                started_at,
                completed_at,
                duration_ms,
            };
            
            results.push(result);
        }
        
        Ok(results)
    }

    /// Get database statistics
    pub fn get_stats(&self) -> CoreResult<serde_json::Value> {
        let workflow_count: i64 = self.conn.query_row("SELECT COUNT(*) FROM workflows", [], |row| row.get(0))?;
        let run_count: i64 = self.conn.query_row("SELECT COUNT(*) FROM workflow_runs", [], |row| row.get(0))?;
        let active_run_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM workflow_runs WHERE status IN ('Pending', 'Running')", 
            [], 
            |row| row.get(0)
        )?;
        
        Ok(serde_json::json!({
            "workflows": workflow_count,
            "total_runs": run_count,
            "active_runs": active_run_count
        }))
    }
} 