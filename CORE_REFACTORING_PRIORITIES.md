# Cronflow Core - Refactoring Priority List

**Date**: October 20, 2025  
**Status**: Action Required

---

## 🔴 CRITICAL Issues (Fix Immediately)

### 1. Bridge Function Duplication (bridge.rs)

**Problem**: 4 identical functions doing the same thing

```rust
✅ Keep: execute_step()           (Line 187)
❌ Remove: execute_step_function()  (Line 738) - DUPLICATE
❌ Remove: execute_step_in_bun()    (Line 828) - DUPLICATE  
❌ Remove: execute_step_via_bun()   (Line 852) - DUPLICATE

✅ Keep: execute_job()             (Line 966)
❌ Remove: execute_job_function()   (Line 886) - DUPLICATE
```

**Impact**: ~400 lines of duplicate code, API confusion

**Action**: Delete duplicate functions, update SDK to use single function

**Estimated Time**: 2 hours

---

### 2. Bridge Instance Management Anti-Pattern

**Problem**: Most N-API functions create new Bridge instance instead of using shared cache

```rust
// ❌ BAD (Current - most functions)
let bridge = match Bridge::new(&db_path) {
    Ok(bridge) => bridge,
    Err(e) => return Error,
};

// ✅ GOOD (Only 4 functions use this)
with_shared_bridge!(&db_path, ...)
```

**Impact**: 
- Multiple StateManager instances → data inconsistency
- Multiple DB connections → resource waste
- Performance degradation

**Files to Fix**: bridge.rs (60+ functions)

**Action**: Update all N-API functions to use `get_shared_bridge()`

**Estimated Time**: 4 hours

---

### 3. No Async/Await (Despite Tokio Dependency)

**Problem**: All N-API functions are blocking/synchronous

```rust
// Current
#[napi]
pub fn execute_step(...) -> StepExecutionResult { ... }

// Should be
#[napi(ts_return_type = "Promise<StepExecutionResult>")]
pub async fn execute_step(...) -> napi::Result<StepExecutionResult> { ... }
```

**Impact**: Blocks Node.js event loop, poor scalability

**Action**: Add async/await to all I/O operations

**Estimated Time**: 8-16 hours (major refactor)

---

## 🟡 HIGH Priority (Fix Soon)

### 4. Unused Modules

```
⚠️ condition_evaluator.rs - Not used (SDK evaluates conditions)
⚠️ workflow_state_machine.rs - Defined but not integrated
⚠️ execution.rs - Unclear purpose, mostly delegates to SDK
⚠️ webhook_test.rs - Test file never compiled
```

**Action Options**:
- **Option A**: Remove unused code (~600-800 lines)
- **Option B**: Fully implement and integrate

**Recommended**: Remove condition_evaluator.rs and webhook_test.rs, integrate state_machine.rs

**Estimated Time**: 4 hours

---

### 5. Excessive Mutex Locking

**Problem**: Locks held during database operations

```rust
// ❌ BAD
let manager = self.state_manager.lock().unwrap();
let run = manager.get_run(&uuid)?;          // DB call with lock!
let workflow = manager.get_workflow(...)?;  // Another DB call!
let steps = manager.get_completed_steps()?; // Another!

// ✅ GOOD
let (run, workflow, steps) = {
    let manager = self.state_manager.lock().unwrap();
    (
        manager.get_run(&uuid)?,
        manager.get_workflow(...)?,
        manager.get_completed_steps()?
    )
}; // Lock released
// Now work with cloned data
```

**Action**: Refactor to minimize lock hold time

**Estimated Time**: 6 hours

---

## 🟢 MEDIUM Priority (Improvement)

### 6. Worker Pool Using std::thread Instead of Tokio

**Problem**: Worker pool spawns OS threads instead of Tokio tasks

```rust
// Current
thread::spawn(move || { ... });

// Should be
tokio::spawn(async move { ... });
```

**Action**: Migrate worker pool to Tokio tasks

**Estimated Time**: 8 hours

---

### 7. No Database Connection Pooling

**Problem**: Single connection per StateManager

**Action**: Add r2d2 connection pooling

```toml
[dependencies]
r2d2 = "0.8"
r2d2_sqlite = "0.22"
```

**Estimated Time**: 4 hours

---

### 8. No Schema Migration System

**Problem**: Schema is embedded string, no migration support

**Action**: Add migration system (refinery or rusqlite_migration)

**Estimated Time**: 6 hours

---

## 🔵 LOW Priority (Nice to Have)

### 9. Generic Result Type

**Problem**: 15+ specific result types all following same pattern

```rust
// Current
pub struct WorkflowRegistrationResult { success: bool, message: String }
pub struct RunCreationResult { success: bool, run_id: Option<String>, message: String }
// ... 13 more ...

// Better
#[napi(object)]
pub struct NapiResult<T: ToNapiValue> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}
```

**Action**: Consolidate result types

**Estimated Time**: 3 hours

---

### 10. Placeholder Functions

**Problem**: Empty/placeholder implementations

```rust
#[napi]
pub fn pause_workflow(...) -> PauseResumeResult {
    // TODO: Implement
    PauseResumeResult { success: true, status: Some("paused".to_string()), ... }
}
```

**Action**: Either implement or remove

**Estimated Time**: 2 hours per function

---

## Summary Metrics

| Priority | Issues | Lines Affected | Estimated Time | Impact        |
|----------|--------|----------------|----------------|---------------|
| 🔴 Critical | 3    | ~1,200        | 14-24 hours   | Very High     |
| 🟡 High     | 2    | ~800          | 10 hours      | High          |
| 🟢 Medium   | 3    | ~500          | 18 hours      | Medium        |
| 🔵 Low      | 2    | ~200          | 5 hours       | Low           |
| **Total**   | **10** | **~2,700**  | **47-57 hours** | **High** |

---

## Recommended Refactoring Order

### Week 1: Critical Fixes (14-24 hours)
1. ✅ Remove duplicate bridge functions (2h)
2. ✅ Fix bridge instance management (4h)
3. ✅ Remove unused modules (4h)
4. ⏳ Start async/await refactor (8-16h)

### Week 2: High Priority (18 hours)
1. ⏳ Complete async/await refactor
2. ✅ Fix excessive mutex locking (6h)

### Week 3: Medium Priority (18 hours)
1. ✅ Migrate worker pool to Tokio (8h)
2. ✅ Add connection pooling (4h)
3. ✅ Add schema migrations (6h)

### Week 4: Low Priority + Testing (8 hours)
1. ✅ Consolidate result types (3h)
2. ✅ Remove/implement placeholder functions (2h)
3. ✅ Integration testing (3h)

---

## Quick Win: Remove Duplicate Code Now

**Files to Edit**: `core/src/bridge.rs`

**Delete These Functions** (Safe to remove):
- `execute_step_function()` - Line 738
- `execute_step_in_bun()` - Line 828
- `execute_step_via_bun()` - Line 852
- `execute_job_function()` - Line 886
- `pause_workflow()` - Line 1580 (placeholder)
- `resume_workflow()` - Line 1605 (placeholder)

**Update SDK** (Node.js/TypeScript side):
- Change all calls from `core.execute_step_function()` to `core.execute_step()`
- Change all calls from `core.execute_job_function()` to `core.execute_job()`
- Remove calls to `pause_workflow()` and `resume_workflow()` (not implemented)

**Result**: ~400-500 lines removed, cleaner API, zero behavioral change

---

## Testing Checklist

After each refactoring phase:

- [ ] All existing tests pass
- [ ] No N-API errors in SDK
- [ ] Workflow registration works
- [ ] Workflow execution works
- [ ] Step results saved correctly
- [ ] Webhook triggers work
- [ ] Manual triggers work
- [ ] Error handling unchanged
- [ ] Performance not degraded

---

## Notes

- **Breaking Changes**: Async refactor will require SDK updates to handle Promises
- **Database Migration**: Add before v1.0 release
- **Performance**: Expect 2-3x improvement after async + connection pooling
- **Code Quality**: Removing ~600-800 lines of duplicate/dead code

---

## References

- Full Analysis: `CORE_ARCHITECTURE_ANALYSIS.md`
- Database Schema: `core/src/schema.sql`
- N-API Functions: `core/src/bridge.rs`
- Worker Pool: `core/src/dispatcher.rs`

