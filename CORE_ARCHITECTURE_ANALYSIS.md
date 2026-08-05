# Cronflow Core (Rust) - Internal Architecture Documentation

**Version:** 0.1.0  
**Last Updated:** October 20, 2025  
**Purpose:** Internal technical documentation for core architecture and refactoring

---

## Table of Contents

1. [Overview](#overview)
2. [Architecture Diagram](#architecture-diagram)
3. [Module Breakdown](#module-breakdown)
4. [Data Flow](#data-flow)
5. [Critical Issues & Code Smells](#critical-issues--code-smells)
6. [Recommendations for Refactoring](#recommendations-for-refactoring)
7. [Unused/Repetitive Code](#unusedrepetitive-code)

---

## Overview

The Cronflow Core is a **Rust-based workflow execution engine** that communicates with the Node.js/Bun SDK via **N-API (Node-API)**. It provides:

- **State Management**: Persistent SQLite storage for workflows, runs, and step results
- **Job Dispatch**: Worker pool-based concurrent execution system
- **Trigger Management**: Webhook, manual, and scheduled triggers
- **Step Orchestration**: Control flow (if/else), parallel execution, retries
- **Workflow State Machine**: State transitions and execution flow control

### Key Technologies

- **Rust**: Core language for performance and safety
- **N-API (napi-rs)**: Node.js/Bun.js FFI bridge
- **SQLite (rusqlite)**: Embedded database for persistence
- **Tokio**: Async runtime (underutilized - see issues below)
- **Actix-web**: HTTP server framework for webhooks
- **Serde**: JSON serialization/deserialization

---

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│                     Node.js / Bun.js SDK                        │
│                  (TypeScript Application Layer)                 │
└────────────────────────────┬────────────────────────────────────┘
                             │ N-API Bridge
                             │ (JSON serialization)
┌────────────────────────────▼────────────────────────────────────┐
│                    Rust Core Engine (core/)                     │
│                                                                  │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │              bridge.rs (N-API Layer)                     │  │
│  │  • Workflow Registration  • Run Creation                 │  │
│  │  • Step Execution         • Hook Execution               │  │
│  │  • Trigger Execution      • Job Management               │  │
│  └─────┬──────────────────────────────────────────────┬─────┘  │
│        │                                              │         │
│  ┌─────▼───────────────────┐              ┌──────────▼──────┐  │
│  │   State Manager         │              │   Job Dispatcher│  │
│  │   (state.rs)            │              │   (dispatcher.rs│  │
│  │                         │              │                 │  │
│  │ • Workflow Registry     │              │ • Worker Pool   │  │
│  │ • Active Runs Cache     │              │ • Job Queue     │  │
│  │ • Status Updates        │              │ • Concurrency   │  │
│  └─────┬───────────────────┘              └──────┬──────────┘  │
│        │                                         │              │
│  ┌─────▼─────────────────────────────────────────▼──────────┐  │
│  │            Database Layer (database.rs)                  │  │
│  │  • Workflows Table    • Runs Table    • Step Results    │  │
│  │  • SQLite Persistence • Schema Management               │  │
│  └──────────────────────────────────────────────────────────┘  │
│                                                                  │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │          Trigger System                                  │  │
│  │  ┌─────────────────┐  ┌──────────────┐  ┌────────────┐ │  │
│  │  │ Trigger Manager │──│ Trigger      │──│ Webhook    │ │  │
│  │  │ (triggers.rs)   │  │ Executor     │  │ Server     │ │  │
│  │  │                 │  │ (trigger_    │  │ (webhook_  │ │  │
│  │  │ • Registration  │  │  executor.rs)│  │  server.rs)│ │  │
│  │  │ • Routing       │  │ • Execution  │  │ • HTTP API │ │  │
│  │  └─────────────────┘  └──────────────┘  └────────────┘ │  │
│  └──────────────────────────────────────────────────────────┘  │
│                                                                  │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │          Execution Layer                                 │  │
│  │  ┌───────────────┐  ┌─────────────────┐  ┌───────────┐ │  │
│  │  │ Step          │  │ Workflow State  │  │ Condition │ │  │
│  │  │ Orchestrator  │  │ Machine         │  │ Evaluator │ │  │
│  │  │ (step_orch-   │  │ (workflow_      │  │ (condition│ │  │
│  │  │  estrator.rs)  │  │  state_        │  │  _eval-   │ │  │
│  │  │               │  │  machine.rs)    │  │  uator.rs)│ │  │
│  │  │ • Step Flow   │  │ • State Trans.  │  │ • If/Else │ │  │
│  │  │ • Parallel    │  │ • Control Flow  │  │ • Boolean │ │  │
│  │  │ • Retries     │  │ • Pause/Resume  │  │ • Expr    │ │  │
│  │  └───────────────┘  └─────────────────┘  └───────────┘ │  │
│  └──────────────────────────────────────────────────────────┘  │
│                                                                  │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │          Supporting Modules                              │  │
│  │  • models.rs (Data Types)   • error.rs (Error Types)    │  │
│  │  • config.rs (Configuration)• context.rs (Exec Context) │  │
│  │  • job.rs (Job Queue Model) • execution.rs (Step Exec)  │  │
│  └──────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────┘
```

---

## How Cronflow Works (Complete Flow)

### 1. User Experience (SDK Layer)

**User writes a workflow:**
```typescript
const workflow = cronflow.define({
  id: 'user-registration',
  name: 'User Registration Workflow',
  concurrency: 10,
  timeout: '5m',
  hooks: {
    onSuccess: ctx => console.log(`✅ Completed: ${ctx.run.id}`),
    onFailure: ctx => console.error(`❌ Failed: ${ctx.error}`)
  }
})
.step('validate-email', async ctx => {
  const { email } = ctx.payload;
  if (!email || !email.includes('@')) {
    throw new Error('Invalid email');
  }
  return { valid: true, email };
})
.step('check-existing', async ctx => {
  const exists = await db.users.findOne({ email: ctx.last.email });
  if (exists) throw new Error('User already exists');
  return { exists: false };
})
.step('create-user', async ctx => {
  const user = await db.users.create(ctx.payload);
  return { userId: user.id };
})
.onWebhook('/api/register', { method: 'POST' })
.manual();

await cronflow.start();
```

### 2. Registration Phase (SDK → Rust)

```
SDK: cronflow.start()
  ↓
SDK: Iterate over all defined workflows
  ↓
For each workflow:
  ↓ Serialize to JSON
  {
    "id": "user-registration",
    "name": "User Registration Workflow",
    "steps": [
      {
        "id": "validate-email",
        "name": "validate-email",
        "action": "step_handler",  // Placeholder - actual function in SDK
        "timeout": 5000,
        "retry": null,
        "depends_on": []
      },
      {
        "id": "check-existing",
        "name": "check-existing",
        "action": "step_handler",
        "timeout": 5000,
        "depends_on": ["validate-email"]
      },
      {
        "id": "create-user",
        "name": "create-user",
        "action": "step_handler",
        "timeout": 5000,
        "depends_on": ["check-existing"]
      }
    ],
    "triggers": [
      { "Webhook": { "path": "/api/register", "method": "POST" } },
      { "Manual": null }
    ]
  }
  ↓ N-API: core.registerWorkflow(json, dbPath)
  
Rust: bridge::register_workflow()
  ↓ Deserialize JSON → WorkflowDefinition
  ↓ Validate workflow
  ↓ state_manager.register_workflow()
    ↓ db.save_workflow()
      ↓ SQLite INSERT INTO workflows
      ✓ Stored in database
  ↓ trigger_executor.register_workflow_triggers()
    ↓ For each trigger:
      - Webhook: trigger_manager.register_webhook_trigger()
      - Manual: (no registration needed)
  ✓ Workflow registered

SDK: Store step handlers in memory
  step_registry.set('user-registration:validate-email', handlerFunction)
  step_registry.set('user-registration:check-existing', handlerFunction)
  step_registry.set('user-registration:create-user', handlerFunction)
```

### 3. Trigger Phase (HTTP → Rust → SDK)

**Webhook Trigger:**
```
HTTP POST /api/register
  body: { "email": "user@example.com", "name": "John" }
  ↓
Webhook Server (Actix-web in Rust)
  ↓ Match route → workflow_id = 'user-registration'
  ↓ Parse request body
  ↓ trigger_executor.execute_webhook_trigger()
    ↓ state_manager.create_run(workflow_id, payload)
      ↓ Generate run_id = UUID
      ↓ Create WorkflowRun { status: Pending, payload, started_at }
      ↓ db.save_run()
        ✓ SQLite INSERT INTO workflow_runs
    ↓ dispatcher.submit_job(job)
      ↓ job_queue.enqueue(Job { run_id, workflow_id, ... })
  ↓ Return HTTP 200 { run_id }
```

**Manual Trigger:**
```
SDK: cronflow.trigger('user-registration', { email: '...', name: '...' })
  ↓ N-API: core.createRun(workflowId, payload, dbPath)
  
Rust: bridge::create_run()
  ↓ [Same as webhook flow]
  ↓ Returns run_id
  
SDK: Receives run_id
  ↓ Start execution (next phase)
```

### 4. Execution Phase (Rust Orchestrator ↔ SDK Handlers)

```
Worker Thread (from dispatcher pool):
  ↓ job_queue.dequeue() → Job
  ↓ Get workflow definition from state_manager
  ↓ step_orchestrator.start_step_execution(run_id, workflow_id)
  
Step Orchestrator:
  ↓ Get run from database
  ↓ Load workflow definition
  ↓ Initialize: current_step_index = 0, completed_steps = []
  
  For each step in workflow.steps:
    ┌─────────────────────────────────────────────────┐
    │ STEP 1: validate-email                          │
    └─────────────────────────────────────────────────┘
    ↓ Check dependencies: [] (none) → can execute
    ↓ Build execution context:
      {
        run_id: "550e8400-e29b-41d4-a716-446655440000",
        workflow_id: "user-registration",
        payload: { "email": "user@example.com", "name": "John" },
        steps: {},  // No previous steps yet
        last: null,
        state: { get, set, incr },
        trigger: { headers: {...} }
      }
    ↓ Serialize context to JSON
    ↓ N-API: SDK.executeStepHandler('validate-email', contextJson, run_id)
    
    SDK:
      ↓ Parse context JSON
      ↓ Lookup handler: step_registry.get('user-registration:validate-email')
      ↓ Execute user function:
        try {
          const result = await handlerFunction(context);
          // result = { valid: true, email: "user@example.com" }
        } catch (error) {
          // Handle error (retry, fail, etc.)
        }
      ↓ Return result to Rust via N-API
    
    Rust:
      ↓ Receive result
      ↓ Create StepResult:
        {
          step_id: "validate-email",
          status: "Completed",
          output: { "valid": true, "email": "user@example.com" },
          started_at: "2025-10-20T10:00:00Z",
          completed_at: "2025-10-20T10:00:01Z",
          duration_ms: 1000
        }
      ↓ state_manager.save_step_result(run_id, step_result)
        ↓ db.save_step_result()
          ✓ SQLite INSERT INTO step_results
      ↓ Add to completed_steps list
      ↓ Update context.last = result
    
    ┌─────────────────────────────────────────────────┐
    │ STEP 2: check-existing                          │
    └─────────────────────────────────────────────────┘
    ↓ Check dependencies: ["validate-email"] → completed ✓
    ↓ Build execution context:
      {
        run_id: "...",
        workflow_id: "user-registration",
        payload: { "email": "user@example.com", "name": "John" },
        steps: {
          "validate-email": {
            output: { "valid": true, "email": "user@example.com" }
          }
        },
        last: { "valid": true, "email": "user@example.com" },
        state: { get, set, incr },
        trigger: { headers: {...} }
      }
    ↓ [Same execution flow as Step 1]
    ↓ Result: { exists: false }
    ↓ Save step result
    
    ┌─────────────────────────────────────────────────┐
    │ STEP 3: create-user                             │
    └─────────────────────────────────────────────────┘
    ↓ Check dependencies: ["check-existing"] → completed ✓
    ↓ Build context with all previous steps
    ↓ Execute → { userId: 123 }
    ↓ Save step result
  
  All steps completed!
  ↓ state_manager.complete_run(run_id, RunStatus::Completed, None)
    ↓ Update workflow_runs SET status='Completed', completed_at=NOW()
    ✓ Run completed
```

### 5. Hook Execution Phase (Rust → SDK)

```
Rust: Run completed successfully
  ↓ Get workflow definition
  ↓ Check if hooks.onSuccess exists
  ↓ Build completion context:
    {
      run_id: "...",
      workflow_id: "user-registration",
      status: "Completed",
      completed_steps: [
        { step_id: "validate-email", status: "Completed", output: {...} },
        { step_id: "check-existing", status: "Completed", output: {...} },
        { step_id: "create-user", status: "Completed", output: {...} }
      ],
      payload: { ... },
      final_output: { userId: 123 },
      duration_ms: 3500
    }
  ↓ N-API: SDK.executeHook('onSuccess', contextJson, workflow_id)

SDK:
  ↓ Parse context
  ↓ Lookup hook function from workflow definition
  ↓ Execute: hooks.onSuccess(context)
    → console.log('✅ Completed: 550e8400-...')
  ✓ Hook completed
```

### 6. Error Handling Flow

**Step Execution Error:**
```
Step fails with error
  ↓ SDK catches error
  ↓ Check if step has retry config
    Yes: Retry with backoff
      ↓ Attempt 1, 2, 3... up to max_attempts
      ↓ If all attempts fail → mark step as Failed
    No: Immediately mark step as Failed
  ↓ Create StepResult { status: "Failed", error: "..." }
  ↓ Save to database
  ↓ state_manager.complete_run(run_id, RunStatus::Failed, error_message)
  ↓ Execute hooks.onFailure if defined
  ✓ Run marked as failed
```

### 7. Control Flow (if/else)

**Conditional Execution:**
```typescript
workflow
  .if('has-premium', async ctx => ctx.payload.plan === 'premium')
    .step('send-premium-email', async ctx => { ... })
  .else()
    .step('send-basic-email', async ctx => { ... })
  .endIf()
  .step('log-activity', async ctx => { ... })
```

**Execution:**
```
Step: if_has-premium
  ↓ Evaluate condition in SDK
  ↓ condition(ctx) → true or false
  ↓ Mark control flow block: { condition_met: true/false }
  ↓ Save in run state

Next steps:
  ↓ Check if step is inside if block
  ↓ If condition_met = true:
    ↓ Execute: send-premium-email
    ↓ Skip: send-basic-email
  ↓ If condition_met = false:
    ↓ Skip: send-premium-email
    ↓ Execute: send-basic-email

Step: endIf
  ↓ Close control flow block
  ↓ Continue normal execution: log-activity
```

### 8. Parallel Execution

**Parallel Steps:**
```typescript
workflow.parallel([
  async ctx => { /* API call 1 */ },
  async ctx => { /* API call 2 */ },
  async ctx => { /* API call 3 */ }
])
```

**Execution:**
```
Step: parallel_group_abc123
  ↓ Create ParallelStepGroup { group_id, step_ids: [1, 2, 3] }
  ↓ Execute all steps concurrently:
    ┌─ Step 1: Execute in worker thread 1
    ├─ Step 2: Execute in worker thread 2
    └─ Step 3: Execute in worker thread 3
  ↓ Wait for ALL to complete (Promise.all)
  ↓ Collect results: [result1, result2, result3]
  ↓ If any fail and fail_fast=true → fail entire group
  ↓ Save ParallelStepGroup result
  ✓ Continue to next step
```

### 9. State Management

**State Access in Steps:**
```typescript
.step('increment-counter', async ctx => {
  const current = await ctx.state.get('counter', 0);
  await ctx.state.set('counter', current + 1);
  return { counter: current + 1 };
})
```

**Implementation:**
```
SDK: ctx.state.get('counter', 0)
  ↓ N-API: core.getWorkflowState(workflow_id, 'counter', default=0)
  
Rust: bridge::get_workflow_state()
  ↓ Query database: SELECT value FROM state WHERE workflow_id=? AND key=?
  ↓ Return value or default
  
SDK: ctx.state.set('counter', 5)
  ↓ N-API: core.setWorkflowState(workflow_id, 'counter', 5)
  
Rust: bridge::set_workflow_state()
  ↓ INSERT OR REPLACE INTO state (workflow_id, key, value)
  ✓ State persisted
```

### 10. Concurrency Control

**Workflow Concurrency Limit:**
```typescript
cronflow.define({
  id: 'email-sender',
  concurrency: 5  // Max 5 concurrent runs
})
```

**Enforcement:**
```
Trigger event received
  ↓ dispatcher.submit_job(job)
  ↓ Check concurrency limit:
    active_runs = COUNT(workflow_runs WHERE workflow_id=? AND status IN ('Pending', 'Running'))
    if active_runs >= 5:
      ↓ Queue job (wait for slot)
    else:
      ↓ Assign to worker immediately
```

**Worker Pool:**
```
Dispatcher maintains:
  - Min workers: 4 (always running)
  - Max workers: 16 (scale up under load)
  - Job queue with priority
  
Worker lifecycle:
  1. Worker starts → status = Idle
  2. Job available → dequeue job → status = Busy
  3. Execute workflow run
  4. Complete → status = Idle
  5. Get next job or wait
```

### 11. Database Schema Details

**workflows Table:**
```sql
CREATE TABLE workflows (
    id TEXT PRIMARY KEY,              -- 'user-registration'
    name TEXT NOT NULL,               -- 'User Registration Workflow'
    description TEXT,                 -- Optional description
    definition TEXT NOT NULL,         -- Full JSON workflow definition
    created_at TEXT NOT NULL,         -- ISO 8601 timestamp
    updated_at TEXT NOT NULL          -- ISO 8601 timestamp
);
```

**workflow_runs Table:**
```sql
CREATE TABLE workflow_runs (
    id TEXT PRIMARY KEY,              -- UUID: '550e8400-e29b-...'
    workflow_id TEXT NOT NULL,        -- FK: 'user-registration'
    status TEXT NOT NULL,             -- 'Pending'|'Running'|'Completed'|'Failed'
    payload TEXT NOT NULL,            -- JSON: {"email":"...","name":"..."}
    started_at TEXT NOT NULL,         -- ISO 8601 timestamp
    completed_at TEXT,                -- NULL until completed
    error TEXT,                       -- Error message if failed
    FOREIGN KEY (workflow_id) REFERENCES workflows (id)
);

-- Indexes for performance
CREATE INDEX idx_workflow_runs_workflow_id ON workflow_runs (workflow_id);
CREATE INDEX idx_workflow_runs_status ON workflow_runs (status);
```

**step_results Table:**
```sql
CREATE TABLE step_results (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL,             -- FK: UUID of workflow run
    step_id TEXT NOT NULL,            -- 'validate-email'
    status TEXT NOT NULL,             -- 'Pending'|'Running'|'Completed'|'Failed'
    output TEXT,                      -- JSON: {"valid":true,"email":"..."}
    error TEXT,                       -- Error message if failed
    started_at TEXT NOT NULL,         -- ISO 8601 timestamp
    completed_at TEXT,                -- ISO 8601 timestamp
    duration_ms INTEGER,              -- Execution time in milliseconds
    FOREIGN KEY (run_id) REFERENCES workflow_runs (id)
);

-- Indexes for performance
CREATE INDEX idx_step_results_run_id ON step_results (run_id);
CREATE INDEX idx_step_results_step_id ON step_results (step_id);
```

---

## Module Breakdown

### 1. **lib.rs** (Entry Point)
- **Purpose**: Module declaration and initialization
- **Functions**: 
  - `init()` - Initializes logging
  - Exports all public modules
- **Tests**: Comprehensive unit tests for serialization, validation, N-API bridge
- **Size**: 327 lines

### 2. **bridge.rs** (N-API Layer) ⚠️ CRITICAL - NEEDS REFACTORING
- **Purpose**: Node.js/Bun.js FFI bridge via N-API
- **Size**: **1,627 lines** (TOO LARGE!)
- **Key Components**:
  - `Bridge` struct: Main bridge with state manager, trigger manager, dispatcher
  - `BRIDGE_CACHE`: Global singleton cache (OnceLock pattern)
  - `get_shared_bridge()`: Shared instance retrieval
  - **60+ N-API functions** (excessive duplication)

#### N-API Functions (Exported):
```rust
// Workflow Management
register_workflow()
register_webhook_trigger()
get_webhook_triggers()

// Run Management
create_run()
get_run_status()

// Step Execution (DUPLICATED! 🚨)
execute_step()
execute_step_function()
execute_step_in_bun()
execute_step_via_bun()
execute_workflow_steps()

// Job Management (DUPLICATED! 🚨)
execute_job()
execute_job_function()
get_job_status()
cancel_job()
get_dispatcher_stats()

// Trigger Execution
execute_webhook_trigger()
execute_manual_trigger()
get_trigger_stats()
get_workflow_triggers()
unregister_workflow_triggers()

// Webhook Server
start_webhook_server()
stop_webhook_server()

// Hooks
execute_workflow_hook()

// Pause/Resume
pause_workflow()
resume_workflow()

// Workflow Run Status
get_workflow_run_status()
get_workflow_completed_steps()
```

#### **🚨 CRITICAL ISSUES**:
1. **Multiple Bridge Creations**: Each N-API function creates a new `Bridge::new()` instead of using shared instance
2. **Inconsistent Pattern**: `with_shared_bridge!` macro used only for 4 functions, rest ignore it
3. **Function Duplication**: 
   - `execute_step()`, `execute_step_function()`, `execute_step_in_bun()`, `execute_step_via_bun()` - ALL DO THE SAME THING
   - `execute_job()` and `execute_job_function()` - IDENTICAL
4. **No Async**: Functions are synchronous despite Tokio dependency
5. **Lock Hell**: Excessive mutex locking/unlocking within same function

### 3. **state.rs** (State Management)
- **Purpose**: Workflow and run state management
- **Size**: 150 lines
- **Key Components**:
  - `StateManager`: Wrapper around Database + in-memory cache
  - `active_runs`: HashMap cache for hot path optimization
  - Operations: register, create_run, update_status, save_step_result
- **Issues**:
  - ✅ Clean and focused
  - ⚠️ `active_runs` cache never expires or cleans up automatically
  - ⚠️ No thread-safe access patterns (relies on external Mutex)

### 4. **database.rs** (Persistence Layer)
- **Purpose**: SQLite database operations
- **Size**: 297 lines
- **Schema**: workflows, workflow_runs, step_results (see schema.sql)
- **Operations**:
  - Workflow CRUD
  - Run CRUD
  - Step result storage
  - Statistics queries
- **Issues**:
  - ✅ Clean separation of concerns
  - ✅ Proper error handling
  - ⚠️ No connection pooling (single connection)
  - ⚠️ No migration system (uses `include_str!("schema.sql")`)

### 5. **dispatcher.rs** (Job Execution System)
- **Purpose**: Worker pool-based job dispatcher
- **Size**: 1,034 lines (Large but acceptable for complexity)
- **Key Components**:
  - `Dispatcher`: Main dispatcher with worker pool
  - `Worker`: Individual worker thread
  - `WorkerPoolConfig`: Configuration (min/max workers, timeouts)
  - `JobQueue`: Queue management
  - `DispatcherStats`: Telemetry
- **Worker Pool**:
  - Min workers: 4 (default)
  - Max workers: 16 (default)
  - Thread-based (not Tokio tasks - ⚠️ inconsistent with async runtime)
- **Issues**:
  - ⚠️ No graceful shutdown mechanism
  - ⚠️ Worker threads never join on shutdown
  - ⚠️ Timeout monitor runs in separate thread (no coordination)
  - ⚠️ Uses std::thread instead of Tokio tasks (despite Tokio dependency)

### 6. **triggers.rs** (Trigger Management)
- **Purpose**: Trigger registration and routing
- **Size**: ~400 lines (estimated)
- **Trigger Types**:
  - `Webhook`: HTTP endpoint triggers
  - `Manual`: Programmatic triggers
  - `Schedule`: Cron-based (handled in SDK layer)
- **Key Components**:
  - `TriggerManager`: Registry of triggers
  - `WebhookTrigger`: Webhook configuration
  - `WebhookRequest`: Incoming request model
- **Issues**:
  - ⚠️ Schedule triggers defined but not implemented in Rust (SDK handles it)
  - ⚠️ No trigger validation beyond basic checks

### 7. **trigger_executor.rs** (Trigger Execution)
- **Purpose**: Execute triggers and create workflow runs
- **Size**: ~300 lines (estimated)
- **Flow**:
  1. Receive trigger event
  2. Look up associated workflow
  3. Create run in state manager
  4. Submit job to dispatcher
- **Issues**:
  - ⚠️ Tightly coupled to specific trigger types
  - ⚠️ No middleware/plugin system for custom triggers

### 8. **step_orchestrator.rs** (Step Execution Flow)
- **Purpose**: Orchestrate step-by-step workflow execution
- **Size**: ~500 lines (estimated)
- **Responsibilities**:
  - Step dependency resolution
  - Parallel step execution
  - Retry logic
  - Control flow (if/else) handling
  - Timeout management
- **Issues**:
  - ⚠️ Control flow partially implemented (if/else markers exist, full logic in SDK)
  - ⚠️ Parallel execution partially implemented
  - ⚠️ Calls back to N-API for actual step execution (circular dependency)

### 9. **workflow_state_machine.rs** (State Transitions)
- **Purpose**: Manage workflow state transitions
- **Size**: ~400 lines (estimated)
- **States**: Pending → Running → Completed/Failed/Cancelled
- **Transitions**: Validated state transitions with guard conditions
- **Issues**:
  - ⚠️ Partially implemented - not fully integrated with step orchestrator
  - ⚠️ Pause/Resume states defined but not fully functional

### 10. **condition_evaluator.rs** (Control Flow Logic)
- **Purpose**: Evaluate conditional expressions for if/else steps
- **Size**: ~200 lines (estimated)
- **Expressions**: Boolean logic, comparisons, context variable access
- **Issues**:
  - ⚠️ Expression evaluation happens in SDK (Bun.js), this is just a placeholder
  - ⚠️ Not actually used in current flow

### 11. **execution.rs** (Step Execution)
- **Purpose**: Execute individual step functions
- **Size**: ~300 lines (estimated)
- **Issues**:
  - ⚠️ Mostly delegates back to SDK via N-API
  - ⚠️ Unclear separation between Rust and SDK execution

### 12. **models.rs** (Data Models) ✅ WELL-DESIGNED
- **Purpose**: Workflow, Run, Step, and supporting data structures
- **Size**: 822 lines
- **Key Types**:
  - `WorkflowDefinition`, `StepDefinition`, `TriggerDefinition`
  - `WorkflowRun`, `RunStatus`, `StepResult`, `StepStatus`
  - `ControlFlowBlock`, `ConditionType`, `ConditionResult`
  - `ParallelStepGroup`, `ParallelGroupStatus`
  - `WorkflowCompletionContext`
  - `RetryConfig`
- **Validation**: Comprehensive validation methods on all models
- **Issues**: None - this is clean and well-structured

### 13. **error.rs** (Error Handling) ✅ CLEAN
- **Purpose**: Centralized error types using `thiserror`
- **Size**: 64 lines
- **Error Types**: Database, Serialization, HTTP, Validation, State, etc.
- **Issues**: None - proper error handling

### 14. **context.rs** (Execution Context)
- **Purpose**: Context passed to step execution
- **Size**: ~200 lines (estimated)
- **Contains**: run_id, workflow_id, payload, step results, state access
- **Issues**:
  - ⚠️ Context creation is complex and duplicated across multiple functions
  - ⚠️ State access methods not fully implemented

### 15. **config.rs** (Configuration)
- **Purpose**: Centralized configuration management
- **Size**: ~150 lines (estimated)
- **Configuration**: Worker pool, timeouts, database path, etc.
- **Issues**: ✅ Clean centralized config

### 16. **job.rs** (Job Model)
- **Purpose**: Job queue and job state management
- **Size**: ~300 lines (estimated)
- **Key Types**: `Job`, `JobQueue`, `JobState`, `JobPriority`
- **Issues**:
  - ⚠️ Priority queue not fully utilized
  - ⚠️ Job cancellation partially implemented

### 17. **webhook_server.rs** (HTTP Server)
- **Purpose**: Actix-web HTTP server for webhook endpoints
- **Size**: ~500 lines (estimated)
- **Issues**:
  - ⚠️ Server lifecycle management unclear
  - ⚠️ Async server but bridge functions are sync
  - ⚠️ Start/stop functions in bridge.rs don't actually control the server

---

## Data Flow

### Workflow Registration Flow

```
SDK (TypeScript)
    ↓ cronflow.define(config)
    ↓ core.registerWorkflow(json)
N-API Bridge
    ↓ bridge::register_workflow()
    ↓ Bridge::register_workflow()
State Manager
    ↓ state_manager.register_workflow()
Database
    ↓ db.save_workflow()
SQLite
    ✓ workflows table
```

### Workflow Execution Flow

```
SDK Trigger
    ↓ cronflow.trigger(workflowId, payload)
    ↓ core.createRun(workflowId, payload)
N-API Bridge
    ↓ bridge::create_run()
    ↓ Bridge::create_run()
State Manager
    ↓ state_manager.create_run()
    ↓ run_id = UUID::new()
Database
    ↓ db.save_run(run)
    ✓ workflow_runs table

Step Execution Trigger
    ↓ Bridge::execute_workflow_steps()
Step Orchestrator
    ↓ StepOrchestrator::start_step_execution()
    ↓ Get workflow definition
    ↓ Get step dependencies
    ↓ For each step:
        ↓ Create context
        ↓ Check conditions (if control flow)
        ↓ Execute step (CALLS BACK TO SDK!)
        ↓ Save step result
State Manager
    ↓ state_manager.save_step_result()
Database
    ↓ db.save_step_result()
    ✓ step_results table
```

### **🚨 CIRCULAR DEPENDENCY ISSUE**

```
Rust Core
    ↓ Executes workflow
    ↓ Needs to run step handler function
    ↓ Calls N-API function
    ↓ Calls SDK step handler
SDK (TypeScript/Bun.js)
    ↓ Executes step handler
    ↓ Returns result
    ↓ Calls N-API to save result
Rust Core
    ↓ Saves step result
    ↓ Continues to next step
    ↓ ...repeat...
```

**Problem**: Excessive N-API boundary crossings, performance overhead

---

## Critical Issues & Code Smells

### 1. **🔴 CRITICAL: Bridge Function Duplication**

**Problem**: 4 different functions that do the same thing:

```rust
// bridge.rs
execute_step()           // Lines 187-234
execute_step_function()  // Lines 738-825
execute_step_in_bun()    // Lines 828-849
execute_step_via_bun()   // Lines 852-882
```

**Impact**: 
- Code maintainability nightmare
- Confusing API for SDK developers
- Bug-prone (fixes need to be applied to all 4)

**Solution**: Consolidate into ONE function with clear naming

### 2. **🔴 CRITICAL: Bridge Instance Management**

**Problem**: Inconsistent Bridge creation patterns

```rust
// Some functions use shared bridge (good)
#[napi]
pub fn register_workflow(workflow_json: String, db_path: String) -> WorkflowRegistrationResult {
    with_shared_bridge!(...)  // ✅ Uses cached instance
}

// Most functions create new bridge every call (bad!)
#[napi]
pub fn register_webhook_trigger(workflow_id: String, trigger_json: String, db_path: String) -> WebhookTriggerRegistrationResult {
    let bridge = match Bridge::new(&db_path) {  // ❌ Creates new instance
        Ok(bridge) => bridge,
        Err(e) => { ... }
    };
}
```

**Impact**:
- Multiple StateManager instances (data inconsistency)
- Multiple database connections (resource waste)
- Performance degradation

**Solution**: ALL N-API functions must use `get_shared_bridge()`

### 3. **🔴 CRITICAL: No Async/Await Usage**

**Problem**: Tokio is a dependency but never used

```toml
# Cargo.toml
tokio = { version = "1.0", features = ["full"] }  # Unused!
```

**Current**:
```rust
#[napi]
pub fn execute_step(...) -> StepExecutionResult {
    // Synchronous blocking code
}
```

**Should Be**:
```rust
#[napi(ts_return_type = "Promise<StepExecutionResult>")]
pub async fn execute_step(...) -> napi::Result<StepExecutionResult> {
    // Async non-blocking code
}
```

**Impact**:
- Blocks Node.js event loop
- Poor scalability
- Cannot leverage Rust async ecosystem

**Solution**: Refactor to async/await with napi-rs async support

### 4. **🟡 MEDIUM: Excessive Mutex Locking**

**Example**:
```rust
pub fn execute_step(&self, run_id: &str, step_id: &str) -> CoreResult<String> {
    let state_manager = self.state_manager.lock().unwrap();  // Lock 1
    let run = state_manager.get_run(&run_uuid)?;            // Database call while holding lock
    let workflow = state_manager.get_workflow(...)?;        // Another database call
    let completed_steps = state_manager.get_completed_steps(...)?; // Another call
    // Lock held for entire function duration!
}
```

**Problem**: Lock contention, blocking other threads unnecessarily

**Solution**: Acquire lock, clone data, release lock, then operate on cloned data

### 5. **🟡 MEDIUM: Worker Pool Not Using Tokio**

**Problem**: Worker pool uses std::thread instead of Tokio tasks

```rust
// dispatcher.rs
thread::spawn(move || {
    // Worker loop
});
```

**Should Be**:
```rust
tokio::spawn(async move {
    // Async worker loop
});
```

**Impact**: Cannot leverage async I/O, thread overhead

### 6. **🟡 MEDIUM: State Machine Not Integrated**

**Problem**: `workflow_state_machine.rs` exists but not used

The step orchestrator doesn't use the state machine for state transitions. It manually updates status.

**Impact**: No centralized state validation, potential invalid state transitions

### 7. **🟢 MINOR: No Database Connection Pooling**

**Problem**: Single database connection per StateManager

**Impact**: Potential bottleneck under high load

**Solution**: Use `r2d2` connection pool

### 8. **🟢 MINOR: No Schema Migrations**

**Problem**: Schema is embedded as string, no migration support

**Impact**: Cannot upgrade production databases without downtime

**Solution**: Add migration system (e.g., `refinery` or `rusqlite_migration`)

---

## Unused/Repetitive Code

### **Completely Unused Files**

1. **webhook_test.rs**: Test file that is never compiled or run
   - **Action**: Delete or move to tests/ directory

### **Partially Implemented / Dead Code**

1. **condition_evaluator.rs**: Expression evaluator not used
   - Condition evaluation happens in SDK
   - **Action**: Either fully implement or delete

2. **execution.rs**: Mostly delegates to SDK
   - **Action**: Merge with step_orchestrator.rs or clarify responsibility

3. **Workflow State Machine**: Defined but not integrated
   - **Action**: Integrate with step orchestrator or remove

### **Duplicate Functions (bridge.rs)**

| Function                    | Lines   | Purpose              | Keep/Remove |
|-----------------------------|---------|----------------------|-------------|
| `execute_step`              | 187-234 | Prepare step context | **Keep**    |
| `execute_step_function`     | 738-825 | Same as above        | **Remove**  |
| `execute_step_in_bun`       | 828-849 | Same as above        | **Remove**  |
| `execute_step_via_bun`      | 852-882 | Same as above        | **Remove**  |
| `execute_job`               | 966-1038| Execute job          | **Keep**    |
| `execute_job_function`      | 886-962 | Same as above        | **Remove**  |
| `pause_workflow`            | 1580    | Placeholder          | **Remove**  |
| `resume_workflow`           | 1605    | Placeholder          | **Remove**  |

**Estimated Line Savings**: ~400-500 lines

### **Repetitive Result Types**

```rust
// bridge.rs has 15+ different result types:
WorkflowRegistrationResult
RunCreationResult
RunStatusResult
StepExecutionResult
WebhookTriggerRegistrationResult
WebhookTriggersResult
TriggerExecutionResult
TriggerStatsResult
WorkflowTriggersResult
TriggerUnregistrationResult
JobExecutionResult
JobStatusResult
JobCancellationResult
DispatcherStatsResult
... and more
```

**Problem**: All follow same pattern: `{ success: bool, message: String, data?: T }`

**Solution**: Create generic `NapiResult<T>` type:

```rust
#[napi(object)]
pub struct NapiResult<T> {
    pub success: bool,
    pub data: Option<T>,
    pub message: String,
}
```

**Estimated Line Savings**: ~200 lines

---

## Recommendations for Refactoring

### Phase 1: Critical Fixes (High Priority)

#### 1.1 Consolidate Bridge Functions
```rust
// Remove these:
- execute_step_function
- execute_step_in_bun  
- execute_step_via_bun
- execute_job_function

// Keep and rename:
execute_step          → prepare_step_execution
execute_job           → execute_workflow_job
```

#### 1.2 Fix Bridge Instance Management
```rust
// Update ALL N-API functions to use shared bridge:
#[napi]
pub fn some_function(db_path: String) -> SomeResult {
    with_shared_bridge!(
        &db_path,
        |result| { /* success handler */ },
        |error| { /* error handler */ },
        |bridge: Arc<Bridge>| {
            bridge.some_operation()
        }
    )
}
```

#### 1.3 Add Async Support
```rust
// Convert synchronous functions to async:
#[napi]
pub async fn execute_step(
    run_id: String, 
    step_id: String, 
    db_path: String
) -> napi::Result<StepExecutionResult> {
    // Use Tokio async operations
}
```

### Phase 2: Architecture Improvements (Medium Priority)

#### 2.1 Integrate State Machine
```rust
// step_orchestrator.rs
impl StepOrchestrator {
    pub fn execute_step(&self, ...) -> CoreResult<()> {
        let state_machine = WorkflowStateMachine::new();
        state_machine.transition(current_state, StepEvent::Started)?;
        // Execute step
        state_machine.transition(current_state, StepEvent::Completed)?;
    }
}
```

#### 2.2 Remove/Implement Condition Evaluator
**Option A**: Fully implement in Rust
**Option B**: Remove file, document that SDK handles it

#### 2.3 Consolidate Result Types
```rust
// Use generic result type
#[napi(object)]
pub struct NapiResult<T: ToNapiValue> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}
```

### Phase 3: Performance & Scalability (Low Priority)

#### 3.1 Add Database Connection Pooling
```toml
[dependencies]
r2d2 = "0.8"
r2d2_sqlite = "0.22"
```

#### 3.2 Migrate Worker Pool to Tokio
```rust
// dispatcher.rs
tokio::spawn(async move {
    // Worker logic
});
```

#### 3.3 Add Schema Migrations
```toml
[dependencies]
refinery = "0.8"
```

---

## File Size Summary

| File                      | Lines | Status    | Action Needed            |
|---------------------------|-------|-----------|--------------------------|
| bridge.rs                 | 1,627 | 🔴 BLOATED | Split + consolidate      |
| dispatcher.rs             | 1,034 | 🟡 LARGE  | Acceptable, review logic |
| models.rs                 | 822   | ✅ GOOD   | Keep as-is               |
| step_orchestrator.rs      | ~500  | 🟡 MEDIUM | Integrate state machine  |
| webhook_server.rs         | ~500  | 🟡 MEDIUM | Clarify lifecycle        |
| triggers.rs               | ~400  | ✅ GOOD   | Minor cleanup            |
| trigger_executor.rs       | ~300  | ✅ GOOD   | Keep as-is               |
| database.rs               | 297   | ✅ GOOD   | Add connection pooling   |
| state.rs                  | 150   | ✅ GOOD   | Add cache expiration     |
| execution.rs              | ~300  | ⚠️ UNCLEAR | Merge or clarify         |
| condition_evaluator.rs    | ~200  | ⚠️ UNUSED | Remove or implement      |
| workflow_state_machine.rs | ~400  | ⚠️ UNUSED | Integrate or remove      |
| context.rs                | ~200  | ✅ GOOD   | Cleanup duplication      |
| job.rs                    | ~300  | ✅ GOOD   | Keep as-is               |
| config.rs                 | ~150  | ✅ GOOD   | Keep as-is               |
| error.rs                  | 64    | ✅ PERFECT| Keep as-is               |
| lib.rs                    | 327   | ✅ GOOD   | Keep as-is               |

**Total Estimated Lines**: ~8,500-9,000  
**Removable/Consolidateable**: ~600-800 lines (7-9%)

---

## Next Steps

1. **Immediate Actions**:
   - [ ] Consolidate bridge.rs duplicate functions
   - [ ] Fix bridge instance management (use shared instance everywhere)
   - [ ] Remove dead code (execute_step_* duplicates)

2. **Short-term (1-2 weeks)**:
   - [ ] Add async/await support to critical paths
   - [ ] Integrate workflow state machine
   - [ ] Remove or fully implement condition evaluator

3. **Medium-term (1 month)**:
   - [ ] Migrate worker pool to Tokio
   - [ ] Add database connection pooling
   - [ ] Add schema migration system

4. **Long-term (Ongoing)**:
   - [ ] Performance profiling and optimization
   - [ ] Add comprehensive integration tests
   - [ ] Documentation improvements

---

## Conclusion

The Cronflow Rust core has a **solid foundation** but suffers from:

1. **Excessive code duplication** (especially in bridge.rs)
2. **Incomplete async implementation** (Tokio dependency unused)
3. **Architecture drift** (state machine defined but not integrated)
4. **API confusion** (4 functions doing the same thing)

**Estimated Refactoring Impact**:
- Remove ~600-800 lines of duplicate/dead code
- Improve performance by 2-3x with async/await
- Reduce complexity and maintainability issues
- Enable future scalability improvements

**Priority Order**: Critical Fixes → Architecture Improvements → Performance Optimization

