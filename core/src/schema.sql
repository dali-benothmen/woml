-- Node-Cronflow Database Schema
-- 
-- This file contains the SQLite schema for the Node-Cronflow core engine.
-- The schema is designed to store workflow definitions, execution runs,
-- and step results with proper indexing for performance.

-- Workflow definitions table
-- Stores the workflow definitions that users create
CREATE TABLE IF NOT EXISTS workflows (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT,
    definition TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Workflow runs table
-- Stores individual executions of workflows
CREATE TABLE IF NOT EXISTS workflow_runs (
    id TEXT PRIMARY KEY,
    workflow_id TEXT NOT NULL,
    status TEXT NOT NULL,
    payload TEXT NOT NULL,
    started_at TEXT NOT NULL,
    completed_at TEXT,
    error TEXT,
    FOREIGN KEY (workflow_id) REFERENCES workflows (id)
);

-- Step results table
-- Stores the results of individual steps within a workflow run
CREATE TABLE IF NOT EXISTS step_results (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL,
    step_id TEXT NOT NULL,
    status TEXT NOT NULL,
    output TEXT,
    error TEXT,
    started_at TEXT NOT NULL,
    completed_at TEXT,
    duration_ms INTEGER,
    FOREIGN KEY (run_id) REFERENCES workflow_runs (id)
);

-- Triggers table
-- Stores trigger configurations for workflows
CREATE TABLE IF NOT EXISTS triggers (
    id TEXT PRIMARY KEY,
    workflow_id TEXT NOT NULL,
    trigger_type TEXT NOT NULL,
    config TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (workflow_id) REFERENCES workflows (id)
);

-- Indexes for performance
CREATE INDEX IF NOT EXISTS idx_workflow_runs_workflow_id ON workflow_runs (workflow_id);
CREATE INDEX IF NOT EXISTS idx_workflow_runs_status ON workflow_runs (status);
CREATE INDEX IF NOT EXISTS idx_workflow_runs_started_at ON workflow_runs (started_at);
CREATE INDEX IF NOT EXISTS idx_step_results_run_id ON step_results (run_id);
CREATE INDEX IF NOT EXISTS idx_step_results_step_id ON step_results (step_id);
CREATE INDEX IF NOT EXISTS idx_step_results_status ON step_results (status);
CREATE INDEX IF NOT EXISTS idx_triggers_workflow_id ON triggers (workflow_id);
CREATE INDEX IF NOT EXISTS idx_triggers_type ON triggers (trigger_type);

-- Views for common queries
CREATE VIEW IF NOT EXISTS v_active_runs AS
SELECT 
    wr.id,
    wr.workflow_id,
    w.name as workflow_name,
    wr.status,
    wr.started_at,
    wr.completed_at
FROM workflow_runs wr
JOIN workflows w ON wr.workflow_id = w.id
WHERE wr.status IN ('Pending', 'Running');

CREATE VIEW IF NOT EXISTS v_step_summary AS
SELECT 
    sr.run_id,
    sr.step_id,
    sr.status,
    sr.duration_ms,
    sr.started_at,
    sr.completed_at
FROM step_results sr
ORDER BY sr.started_at DESC; 