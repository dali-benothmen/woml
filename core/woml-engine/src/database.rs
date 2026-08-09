//! Database v1 capability with the SC7 user-owned SQLite backend.

use std::{
  collections::{HashMap, HashSet},
  path::{Component, Path, PathBuf},
  sync::{Arc, Mutex, TryLockError},
  time::Duration,
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use futures_util::future::BoxFuture;
use rusqlite::{
  params_from_iter,
  types::{Value as SqlValue, ValueRef},
  Connection, Error as SqlError, ErrorCode, OpenFlags, TransactionBehavior,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::{
  CapabilityCancellationToken, CapabilityDescriptor, CapabilityEffect, CapabilityFailure,
  CapabilityFailureKind, CapabilityHandler,
};

const DATABASE_CONTRACT: &str = "woml.database";
const DATABASE_CONTRACT_VERSION: u32 = 1;
const MAX_SQL_BYTES: usize = 262_144;
const MAX_ROWS: usize = 10_000;
const MAX_COLUMNS: usize = 256;
const MAX_TRANSACTION_OPERATIONS: usize = 100;
const MAX_RESULT_BYTES: usize = 4_194_304;
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

const DATABASE_OPERATIONS: [&str; 7] = [
  "query",
  "execute",
  "read",
  "insert",
  "update",
  "delete",
  "transaction",
];
const TRANSACTION_OPERATIONS: [&str; 5] = ["query", "execute", "insert", "update", "delete"];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DatabaseRequest {
  contract: String,
  contract_version: u32,
  kind: String,
  driver: String,
  connection: String,
  operation: String,
  input: Value,
}

struct PooledSqliteConnection {
  connection: Mutex<Connection>,
  interrupt: Arc<rusqlite::InterruptHandle>,
}

impl std::fmt::Debug for PooledSqliteConnection {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter.debug_struct("PooledSqliteConnection").finish()
  }
}

#[derive(Debug, Default)]
pub struct ManagedDatabasePool {
  connections: Mutex<HashMap<PathBuf, Arc<PooledSqliteConnection>>>,
  protected_paths: Mutex<HashSet<PathBuf>>,
}

impl ManagedDatabasePool {
  pub fn protect_path(&self, path: &Path) -> Result<(), CapabilityFailure> {
    let path = normalize_path(path)?;
    self
      .protected_paths
      .lock()
      .expect("database protected-path lock")
      .insert(path);
    Ok(())
  }

  fn validate_path(&self, connection: &str) -> Result<PathBuf, CapabilityFailure> {
    let path = database_path(connection)?;
    if self
      .protected_paths
      .lock()
      .expect("database protected-path lock")
      .contains(&path)
    {
      return Err(database_failure(
        CapabilityFailureKind::InvalidInput,
        "WOML_DATABASE_PATH_FORBIDDEN",
        "The selected database is reserved for WOML runtime state.",
        false,
        false,
      ));
    }
    Ok(path)
  }

  fn connection(&self, connection: &str) -> Result<Arc<PooledSqliteConnection>, CapabilityFailure> {
    let path = self.validate_path(connection)?;
    let mut connections = self.connections.lock().expect("database pool lock");
    if let Some(connection) = connections.get(&path) {
      return Ok(connection.clone());
    }
    let connection = open_connection(&path)?;
    let interrupt = Arc::new(connection.get_interrupt_handle());
    let pooled = Arc::new(PooledSqliteConnection {
      connection: Mutex::new(connection),
      interrupt,
    });
    connections.insert(path, pooled.clone());
    Ok(pooled)
  }

  fn read_connection(
    &self,
    connection: &str,
  ) -> Result<Arc<PooledSqliteConnection>, CapabilityFailure> {
    let path = self.validate_path(connection)?;
    let connection = open_read_connection(&path)?;
    let interrupt = Arc::new(connection.get_interrupt_handle());
    Ok(Arc::new(PooledSqliteConnection {
      connection: Mutex::new(connection),
      interrupt,
    }))
  }
}

fn open_read_connection(path: &Path) -> Result<Connection, CapabilityFailure> {
  let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
    .map_err(|error| sqlite_failure(error, true, false))?;
  connection
    .busy_timeout(Duration::from_secs(5))
    .and_then(|_| connection.execute_batch("PRAGMA foreign_keys = ON;"))
    .map_err(|error| sqlite_failure(error, true, false))?;
  Ok(connection)
}

fn open_connection(path: &Path) -> Result<Connection, CapabilityFailure> {
  let connection = Connection::open(path).map_err(|error| sqlite_failure(error, false, false))?;
  connection
    .busy_timeout(Duration::from_secs(5))
    .and_then(|_| connection.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;"))
    .map_err(|error| sqlite_failure(error, false, false))?;
  Ok(connection)
}

#[derive(Debug)]
pub struct ManagedDatabaseHandler {
  pool: Arc<ManagedDatabasePool>,
  operation: &'static str,
}

impl ManagedDatabaseHandler {
  pub fn handlers(pool: Arc<ManagedDatabasePool>) -> Vec<Arc<Self>> {
    DATABASE_OPERATIONS
      .iter()
      .map(|operation| {
        Arc::new(Self {
          pool: pool.clone(),
          operation,
        })
      })
      .collect()
  }
}

impl CapabilityHandler for ManagedDatabaseHandler {
  fn descriptor(&self) -> CapabilityDescriptor {
    CapabilityDescriptor {
      capability: "db".to_string(),
      operation: self.operation.to_string(),
      input_contract_version: 1,
      result_contract_version: 1,
      effect: if matches!(self.operation, "query" | "read") {
        CapabilityEffect::Read
      } else {
        CapabilityEffect::UnsafeWrite
      },
      supports_cancellation: true,
      supports_provider_idempotency: false,
    }
  }

  fn validate_request(
    &self,
    request: &crate::CapabilityCallRequest,
  ) -> Result<(), CapabilityFailure> {
    let parsed = parse_request(&request.input, self.operation)?;
    self.pool.validate_path(&parsed.connection).map(|_| ())
  }

  fn safe_metadata(&self, _input: &Value) -> Map<String, Value> {
    Map::from_iter([("driver".to_string(), Value::String("sqlite".to_string()))])
  }

  fn safe_result_metadata(&self, result: &Value) -> Map<String, Value> {
    let Some(data) = result.get("data") else {
      return Map::new();
    };
    for field in ["rowCount", "rowsAffected"] {
      if let Some(value) = data.get(field).and_then(Value::as_u64) {
        return Map::from_iter([(field.to_string(), Value::from(value))]);
      }
    }
    Map::new()
  }

  fn execute(
    &self,
    input: Value,
    cancellation: CapabilityCancellationToken,
  ) -> BoxFuture<'static, Result<Value, CapabilityFailure>> {
    let request = match parse_request(&input, self.operation) {
      Ok(request) => request,
      Err(error) => return Box::pin(async move { Err(error) }),
    };
    let write = !matches!(self.operation, "query" | "read");
    let pooled = match if write {
      self.pool.connection(&request.connection)
    } else {
      self.pool.read_connection(&request.connection)
    } {
      Ok(pooled) => pooled,
      Err(error) => return Box::pin(async move { Err(error) }),
    };
    let interrupt = pooled.interrupt.clone();
    Box::pin(async move {
      if cancellation.is_cancelled() {
        return Err(database_cancelled(write));
      }
      let task = tokio::task::spawn_blocking(move || {
        let mut connection = match pooled.connection.try_lock() {
          Ok(connection) => connection,
          Err(TryLockError::WouldBlock) => {
            return Err(database_failure(
              CapabilityFailureKind::ServiceRejected,
              "WOML_DATABASE_BUSY",
              "The local database connection is busy.",
              !write,
              false,
            ))
          }
          Err(TryLockError::Poisoned(_)) => {
            return Err(database_failure(
              CapabilityFailureKind::HandlerCrashed,
              "WOML_DATABASE_CONNECTION_UNAVAILABLE",
              "The local database connection is unavailable.",
              false,
              write,
            ))
          }
        };
        execute_database_request(&mut connection, &request)
      });
      tokio::pin!(task);
      tokio::select! {
        biased;
        _ = cancellation.cancelled() => {
          interrupt.interrupt();
          let _ = task.await;
          Err(database_cancelled(write))
        }
        outcome = &mut task => match outcome {
          Ok(result) => result,
          Err(_) => Err(database_failure(
            CapabilityFailureKind::HandlerCrashed,
            "WOML_DATABASE_HANDLER_CRASHED",
            "The database handler stopped unexpectedly.",
            false,
            write,
          )),
        }
      }
    })
  }
}

fn parse_request(input: &Value, operation: &str) -> Result<DatabaseRequest, CapabilityFailure> {
  let request: DatabaseRequest = serde_json::from_value(input.clone())
    .map_err(|_| invalid_input("Database input does not match the frozen Database v1 envelope."))?;
  if request.contract != DATABASE_CONTRACT
    || request.contract_version != DATABASE_CONTRACT_VERSION
    || request.kind != "request"
    || request.driver != "sqlite"
    || request.operation != operation
    || !DATABASE_OPERATIONS.contains(&request.operation.as_str())
  {
    return Err(invalid_input(
      "Database input does not match the frozen Database v1 envelope.",
    ));
  }
  database_path(&request.connection)?;
  validate_operation_input(operation, &request.input)?;
  Ok(request)
}

fn validate_operation_input(operation: &str, input: &Value) -> Result<(), CapabilityFailure> {
  let object = exact_object(
    input,
    &operation_fields(operation),
    &operation_required(operation),
  )?;
  match operation {
    "query" | "execute" => validate_raw_statement(object),
    "read" => validate_read(object),
    "insert" => validate_insert(object),
    "update" => validate_update(object),
    "delete" => validate_delete(object),
    "transaction" => {
      let operations = object
        .get("operations")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_input("A database transaction requires an operations array."))?;
      if operations.is_empty() || operations.len() > MAX_TRANSACTION_OPERATIONS {
        return Err(invalid_input(
          "A database transaction requires between 1 and 100 operations.",
        ));
      }
      for operation in operations {
        let object = operation
          .as_object()
          .ok_or_else(|| invalid_input("Every transaction operation must be an object."))?;
        let name = object
          .get("operation")
          .and_then(Value::as_str)
          .ok_or_else(|| invalid_input("Every transaction operation requires operation."))?;
        if !TRANSACTION_OPERATIONS.contains(&name) {
          return Err(invalid_input(
            "A nested transaction operation is not supported.",
          ));
        }
        let mut nested = object.clone();
        nested.remove("operation");
        validate_operation_input(name, &Value::Object(nested))?;
      }
      Ok(())
    }
    _ => Err(invalid_input("The database operation is unsupported.")),
  }
}

fn operation_fields(operation: &str) -> Vec<&'static str> {
  match operation {
    "query" | "execute" => vec!["text", "values"],
    "read" => vec!["table", "columns", "where", "orderBy", "limit", "offset"],
    "insert" => vec!["table", "values"],
    "update" => vec!["table", "values", "where"],
    "delete" => vec!["table", "where"],
    "transaction" => vec!["operations"],
    _ => Vec::new(),
  }
}

fn operation_required(operation: &str) -> Vec<&'static str> {
  match operation {
    "query" | "execute" => vec!["text"],
    "read" => vec!["table"],
    "insert" => vec!["table", "values"],
    "update" => vec!["table", "values", "where"],
    "delete" => vec!["table", "where"],
    "transaction" => vec!["operations"],
    _ => Vec::new(),
  }
}

fn exact_object<'a>(
  input: &'a Value,
  allowed: &[&str],
  required: &[&str],
) -> Result<&'a Map<String, Value>, CapabilityFailure> {
  let object = input
    .as_object()
    .ok_or_else(|| invalid_input("Database operation input must be an object."))?;
  if object
    .keys()
    .any(|field| !allowed.contains(&field.as_str()))
    || required.iter().any(|field| !object.contains_key(*field))
  {
    return Err(invalid_input(
      "Database operation input has missing or unsupported fields.",
    ));
  }
  Ok(object)
}

fn validate_raw_statement(object: &Map<String, Value>) -> Result<(), CapabilityFailure> {
  let text = object
    .get("text")
    .and_then(Value::as_str)
    .ok_or_else(|| invalid_input("A database statement requires text."))?;
  if text.trim().is_empty() || text.len() > MAX_SQL_BYTES {
    return Err(invalid_input(
      "Database SQL text must be between 1 byte and 256 KiB.",
    ));
  }
  validate_parameters(object.get("values"))
}

fn validate_read(object: &Map<String, Value>) -> Result<(), CapabilityFailure> {
  validate_identifier_field(object, "table")?;
  if let Some(columns) = object.get("columns") {
    let columns = columns
      .as_array()
      .ok_or_else(|| invalid_input("Database columns must be an array."))?;
    if columns.is_empty() || columns.len() > MAX_COLUMNS {
      return Err(invalid_input(
        "Database columns must contain 1 to 256 names.",
      ));
    }
    let mut unique = HashSet::new();
    for column in columns {
      let column = column
        .as_str()
        .ok_or_else(|| invalid_input("A database column name must be a string."))?;
      validate_identifier(column)?;
      if !unique.insert(column) {
        return Err(invalid_input("Database column names must be unique."));
      }
    }
  }
  if let Some(filters) = object.get("where") {
    validate_value_map(filters, false)?;
  }
  if let Some(order) = object.get("orderBy") {
    let order = order
      .as_array()
      .ok_or_else(|| invalid_input("Database orderBy must be an array."))?;
    if order.len() > 32 {
      return Err(invalid_input("Database orderBy accepts at most 32 fields."));
    }
    for item in order {
      let item = exact_object(item, &["column", "direction"], &["column"])?;
      validate_identifier_field(item, "column")?;
      if item
        .get("direction")
        .is_some_and(|value| !matches!(value.as_str(), Some("asc" | "desc")))
      {
        return Err(invalid_input(
          "Database order direction must be asc or desc.",
        ));
      }
    }
  }
  if object
    .get("limit")
    .is_some_and(|value| !matches!(value.as_u64(), Some(1..=10_000)))
    || object
      .get("offset")
      .is_some_and(|value| value.as_u64().is_none_or(|value| value > 2_147_483_647))
  {
    return Err(invalid_input(
      "Database limit or offset is outside its bounds.",
    ));
  }
  Ok(())
}

fn validate_insert(object: &Map<String, Value>) -> Result<(), CapabilityFailure> {
  validate_identifier_field(object, "table")?;
  validate_value_map(
    object
      .get("values")
      .ok_or_else(|| invalid_input("Database insert requires values."))?,
    true,
  )
}

fn validate_update(object: &Map<String, Value>) -> Result<(), CapabilityFailure> {
  validate_identifier_field(object, "table")?;
  validate_value_map(
    object
      .get("values")
      .ok_or_else(|| invalid_input("Database update requires values."))?,
    true,
  )?;
  validate_value_map(
    object
      .get("where")
      .ok_or_else(|| invalid_input("Database update requires a non-empty where filter."))?,
    true,
  )
}

fn validate_delete(object: &Map<String, Value>) -> Result<(), CapabilityFailure> {
  validate_identifier_field(object, "table")?;
  validate_value_map(
    object
      .get("where")
      .ok_or_else(|| invalid_input("Database delete requires a non-empty where filter."))?,
    true,
  )
}

fn validate_value_map(value: &Value, require_non_empty: bool) -> Result<(), CapabilityFailure> {
  let object = value
    .as_object()
    .ok_or_else(|| invalid_input("Database values and filters must be objects."))?;
  if (require_non_empty && object.is_empty()) || object.len() > MAX_COLUMNS {
    return Err(invalid_input(
      "Database values and filters must contain between 1 and 256 fields.",
    ));
  }
  for (name, value) in object {
    validate_identifier(name)?;
    json_parameter(value)?;
  }
  Ok(())
}

fn validate_parameters(value: Option<&Value>) -> Result<(), CapabilityFailure> {
  let Some(value) = value else { return Ok(()) };
  let parameters = value
    .as_array()
    .ok_or_else(|| invalid_input("Database statement values must be an array."))?;
  if parameters.len() > 10_000 {
    return Err(invalid_input(
      "Database statements accept at most 10,000 values.",
    ));
  }
  for value in parameters {
    json_parameter(value)?;
  }
  Ok(())
}

fn validate_identifier_field(
  object: &Map<String, Value>,
  field: &str,
) -> Result<(), CapabilityFailure> {
  let value = object
    .get(field)
    .and_then(Value::as_str)
    .ok_or_else(|| invalid_input("A database identifier is missing or invalid."))?;
  validate_identifier(value)
}

fn validate_identifier(identifier: &str) -> Result<(), CapabilityFailure> {
  if identifier.is_empty()
    || identifier.len() > 128
    || !identifier.bytes().enumerate().all(|(index, byte)| {
      byte == b'_' || byte.is_ascii_alphabetic() || (index > 0 && byte.is_ascii_digit())
    })
  {
    return Err(invalid_input(
      "Database identifiers must use ASCII letters, digits, and underscores.",
    ));
  }
  Ok(())
}

fn execute_database_request(
  connection: &mut Connection,
  request: &DatabaseRequest,
) -> Result<Value, CapabilityFailure> {
  let data = match request.operation.as_str() {
    "query" => execute_query(connection, &request.input)?,
    "execute" => execute_statement(connection, &request.input, false)?,
    "read" => execute_read(connection, &request.input)?,
    "insert" => execute_insert(connection, &request.input)?,
    "update" => execute_update(connection, &request.input)?,
    "delete" => execute_delete(connection, &request.input)?,
    "transaction" => execute_transaction(connection, &request.input)?,
    _ => return Err(invalid_input("The database operation is unsupported.")),
  };
  Ok(json!({
    "contract": DATABASE_CONTRACT,
    "contractVersion": DATABASE_CONTRACT_VERSION,
    "kind": "result",
    "operation": request.operation,
    "data": data,
  }))
}

fn execute_query(connection: &Connection, input: &Value) -> Result<Value, CapabilityFailure> {
  let object = input.as_object().expect("validated query input");
  let text = object["text"].as_str().expect("validated SQL text");
  let values = sql_parameters(object.get("values"))?;
  query_sql(connection, text, &values)
}

fn query_sql(
  connection: &Connection,
  text: &str,
  values: &[SqlValue],
) -> Result<Value, CapabilityFailure> {
  let mut statement = connection
    .prepare(text)
    .map_err(|error| sqlite_failure(error, true, false))?;
  if !statement.readonly() {
    return Err(invalid_input(
      "db.query accepts only a read-only SQL statement; use db.execute for writes.",
    ));
  }
  let column_count = statement.column_count();
  if column_count > MAX_COLUMNS {
    return Err(invalid_result(
      "The database query returned too many columns.",
    ));
  }
  let names = statement
    .column_names()
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
  let unique = names.iter().collect::<HashSet<_>>();
  if unique.len() != names.len() {
    return Err(invalid_result(
      "The database query returned duplicate column names.",
    ));
  }
  let mut query = statement
    .query(params_from_iter(values.iter()))
    .map_err(|error| sqlite_failure(error, true, false))?;
  let mut rows = Vec::new();
  let mut result_bytes = 0_usize;
  while let Some(row) = query
    .next()
    .map_err(|error| sqlite_failure(error, true, false))?
  {
    if rows.len() == MAX_ROWS {
      return Err(database_failure(
        CapabilityFailureKind::ResultTooLarge,
        "WOML_DATABASE_ROWS_TOO_LARGE",
        "The database query returned more than 10,000 rows.",
        false,
        false,
      ));
    }
    let mut result = Map::new();
    for (index, name) in names.iter().enumerate() {
      result.insert(
        name.clone(),
        sql_result_value(
          row
            .get_ref(index)
            .map_err(|_| invalid_result("The database returned an unreadable column value."))?,
        )?,
      );
    }
    let result = Value::Object(result);
    result_bytes = result_bytes.saturating_add(
      serde_json::to_vec(&result)
        .map_err(|_| invalid_result("The database row could not be serialized."))?
        .len(),
    );
    if result_bytes > MAX_RESULT_BYTES {
      return Err(database_failure(
        CapabilityFailureKind::ResultTooLarge,
        "WOML_DATABASE_RESULT_TOO_LARGE",
        "The database result exceeds 4 MiB.",
        false,
        false,
      ));
    }
    rows.push(result);
  }
  let row_count = rows.len();
  Ok(json!({ "rows": rows, "rowCount": row_count }))
}

fn execute_statement(
  connection: &Connection,
  input: &Value,
  force_insert_id: bool,
) -> Result<Value, CapabilityFailure> {
  let object = input.as_object().expect("validated execute input");
  let text = object["text"].as_str().expect("validated SQL text");
  let values = sql_parameters(object.get("values"))?;
  mutation_sql(connection, text, &values, force_insert_id)
}

fn mutation_sql(
  connection: &Connection,
  text: &str,
  values: &[SqlValue],
  force_insert_id: bool,
) -> Result<Value, CapabilityFailure> {
  let affected = connection
    .execute(text, params_from_iter(values.iter()))
    .map_err(|error| sqlite_failure(error, false, true))?;
  let insert = force_insert_id
    || matches!(
      text
        .trim_start()
        .split_whitespace()
        .next()
        .map(str::to_ascii_uppercase)
        .as_deref(),
      Some("INSERT" | "REPLACE")
    );
  let last_insert_id = if insert {
    let id = connection.last_insert_rowid();
    if !(-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&id) {
      return Err(invalid_result(
        "The database insert ID is outside JavaScript's safe integer range.",
      ));
    }
    Some(id)
  } else {
    None
  };
  Ok(json!({ "rowsAffected": affected, "lastInsertId": last_insert_id }))
}

fn execute_read(connection: &Connection, input: &Value) -> Result<Value, CapabilityFailure> {
  let object = input.as_object().expect("validated read input");
  let table = quote_identifier(object["table"].as_str().expect("validated table"));
  let columns = object
    .get("columns")
    .and_then(Value::as_array)
    .map(|columns| {
      columns
        .iter()
        .map(|column| quote_identifier(column.as_str().expect("validated column")))
        .collect::<Vec<_>>()
        .join(", ")
    })
    .unwrap_or_else(|| "*".to_string());
  let mut values = Vec::new();
  let where_clause = build_where(object.get("where"), &mut values)?;
  let mut text = format!("SELECT {columns} FROM {table}{where_clause}");
  if let Some(order) = object.get("orderBy").and_then(Value::as_array) {
    if !order.is_empty() {
      let fields = order
        .iter()
        .map(|item| {
          let item = item.as_object().expect("validated order item");
          format!(
            "{} {}",
            quote_identifier(item["column"].as_str().expect("validated order column")),
            item
              .get("direction")
              .and_then(Value::as_str)
              .unwrap_or("asc")
              .to_ascii_uppercase()
          )
        })
        .collect::<Vec<_>>()
        .join(", ");
      text.push_str(" ORDER BY ");
      text.push_str(&fields);
    }
  }
  if let Some(limit) = object.get("limit").and_then(Value::as_u64) {
    text.push_str(&format!(" LIMIT {limit}"));
  }
  if let Some(offset) = object.get("offset").and_then(Value::as_u64) {
    if object.get("limit").is_none() {
      text.push_str(" LIMIT -1");
    }
    text.push_str(&format!(" OFFSET {offset}"));
  }
  query_sql(connection, &text, &values)
}

fn execute_insert(connection: &Connection, input: &Value) -> Result<Value, CapabilityFailure> {
  let object = input.as_object().expect("validated insert input");
  let table = quote_identifier(object["table"].as_str().expect("validated table"));
  let values = object["values"].as_object().expect("validated values");
  let columns = values
    .keys()
    .map(|column| quote_identifier(column))
    .collect::<Vec<_>>()
    .join(", ");
  let placeholders = std::iter::repeat_n("?", values.len())
    .collect::<Vec<_>>()
    .join(", ");
  let parameters = values
    .values()
    .map(json_parameter)
    .collect::<Result<Vec<_>, _>>()?;
  mutation_sql(
    connection,
    &format!("INSERT INTO {table} ({columns}) VALUES ({placeholders})"),
    &parameters,
    true,
  )
}

fn execute_update(connection: &Connection, input: &Value) -> Result<Value, CapabilityFailure> {
  let object = input.as_object().expect("validated update input");
  let table = quote_identifier(object["table"].as_str().expect("validated table"));
  let values = object["values"].as_object().expect("validated values");
  let mut parameters = values
    .values()
    .map(json_parameter)
    .collect::<Result<Vec<_>, _>>()?;
  let assignments = values
    .keys()
    .map(|column| format!("{} = ?", quote_identifier(column)))
    .collect::<Vec<_>>()
    .join(", ");
  let where_clause = build_where(object.get("where"), &mut parameters)?;
  mutation_sql(
    connection,
    &format!("UPDATE {table} SET {assignments}{where_clause}"),
    &parameters,
    false,
  )
}

fn execute_delete(connection: &Connection, input: &Value) -> Result<Value, CapabilityFailure> {
  let object = input.as_object().expect("validated delete input");
  let table = quote_identifier(object["table"].as_str().expect("validated table"));
  let mut parameters = Vec::new();
  let where_clause = build_where(object.get("where"), &mut parameters)?;
  mutation_sql(
    connection,
    &format!("DELETE FROM {table}{where_clause}"),
    &parameters,
    false,
  )
}

fn execute_transaction(
  connection: &mut Connection,
  input: &Value,
) -> Result<Value, CapabilityFailure> {
  let operations = input["operations"]
    .as_array()
    .expect("validated transaction operations");
  let transaction = connection
    .transaction_with_behavior(TransactionBehavior::Immediate)
    .map_err(|error| sqlite_failure(error, false, true))?;
  let mut results = Vec::with_capacity(operations.len());
  for (index, operation) in operations.iter().enumerate() {
    let object = operation
      .as_object()
      .expect("validated transaction operation");
    let name = object["operation"].as_str().expect("validated operation");
    let mut nested = object.clone();
    nested.remove("operation");
    let result = match name {
      "query" => execute_query(&transaction, &Value::Object(nested)),
      "execute" => execute_statement(&transaction, &Value::Object(nested), false),
      "insert" => execute_insert(&transaction, &Value::Object(nested)),
      "update" => execute_update(&transaction, &Value::Object(nested)),
      "delete" => execute_delete(&transaction, &Value::Object(nested)),
      _ => Err(invalid_input("The transaction operation is unsupported.")),
    }
    .map_err(|mut failure| {
      let mut details = failure.details.take().unwrap_or_default();
      details.insert("operationIndex".to_string(), Value::from(index));
      failure.details = Some(details);
      failure
    })?;
    results.push(result);
  }
  transaction
    .commit()
    .map_err(|error| sqlite_failure(error, false, true))?;
  Ok(json!({ "results": results }))
}

fn build_where(
  value: Option<&Value>,
  parameters: &mut Vec<SqlValue>,
) -> Result<String, CapabilityFailure> {
  let Some(filters) = value.and_then(Value::as_object) else {
    return Ok(String::new());
  };
  if filters.is_empty() {
    return Ok(String::new());
  }
  let mut clauses = Vec::with_capacity(filters.len());
  for (column, value) in filters {
    if value.is_null() {
      clauses.push(format!("{} IS NULL", quote_identifier(column)));
    } else {
      clauses.push(format!("{} = ?", quote_identifier(column)));
      parameters.push(json_parameter(value)?);
    }
  }
  Ok(format!(" WHERE {}", clauses.join(" AND ")))
}

fn sql_parameters(value: Option<&Value>) -> Result<Vec<SqlValue>, CapabilityFailure> {
  value
    .and_then(Value::as_array)
    .map(|values| values.iter().map(json_parameter).collect())
    .unwrap_or_else(|| Ok(Vec::new()))
}

fn json_parameter(value: &Value) -> Result<SqlValue, CapabilityFailure> {
  match value {
    Value::Null => Ok(SqlValue::Null),
    Value::Bool(value) => Ok(SqlValue::Integer(i64::from(*value))),
    Value::String(value) if value.len() <= 1_048_576 => Ok(SqlValue::Text(value.clone())),
    Value::Number(value) => {
      if let Some(value) = value.as_i64() {
        if (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&value) {
          Ok(SqlValue::Integer(value))
        } else {
          Err(invalid_input(
            "Database integer parameters must be JavaScript safe integers.",
          ))
        }
      } else if let Some(value) = value.as_f64().filter(|value| value.is_finite()) {
        Ok(SqlValue::Real(value))
      } else {
        Err(invalid_input("Database numeric parameters must be finite."))
      }
    }
    Value::Object(object) if object.len() == 1 => object
      .get("bytesBase64")
      .and_then(Value::as_str)
      .ok_or_else(|| invalid_input("Database byte parameters require bytesBase64."))
      .and_then(|value| {
        BASE64
          .decode(value)
          .map(SqlValue::Blob)
          .map_err(|_| invalid_input("Database bytesBase64 is invalid."))
      }),
    _ => Err(invalid_input(
      "Database parameters must be null, boolean, number, string, or bytesBase64.",
    )),
  }
}

fn sql_result_value(value: ValueRef<'_>) -> Result<Value, CapabilityFailure> {
  match value {
    ValueRef::Null => Ok(Value::Null),
    ValueRef::Integer(value) if (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&value) => {
      Ok(Value::from(value))
    }
    ValueRef::Integer(_) => Err(invalid_result(
      "The database returned an integer outside JavaScript's safe range.",
    )),
    ValueRef::Real(value) if value.is_finite() => Ok(Value::from(value)),
    ValueRef::Real(_) => Err(invalid_result("The database returned a non-finite number.")),
    ValueRef::Text(value) if value.len() > MAX_RESULT_BYTES => Err(database_failure(
      CapabilityFailureKind::ResultTooLarge,
      "WOML_DATABASE_RESULT_TOO_LARGE",
      "A database value exceeds 4 MiB.",
      false,
      false,
    )),
    ValueRef::Text(value) => std::str::from_utf8(value)
      .map(|value| Value::String(value.to_string()))
      .map_err(|_| invalid_result("The database returned invalid UTF-8 text.")),
    ValueRef::Blob(value) if value.len() > MAX_RESULT_BYTES => Err(database_failure(
      CapabilityFailureKind::ResultTooLarge,
      "WOML_DATABASE_RESULT_TOO_LARGE",
      "A database value exceeds 4 MiB.",
      false,
      false,
    )),
    ValueRef::Blob(value) => Ok(json!({ "bytesBase64": BASE64.encode(value) })),
  }
}

fn quote_identifier(identifier: &str) -> String {
  format!("\"{identifier}\"")
}

fn database_path(connection: &str) -> Result<PathBuf, CapabilityFailure> {
  if connection.is_empty()
    || connection.len() > 4_096
    || connection == ":memory:"
    || connection.starts_with("file:")
    || connection.contains('\0')
  {
    return Err(database_failure(
      CapabilityFailureKind::InvalidInput,
      "WOML_DATABASE_CONNECTION_INVALID",
      "SQLite requires a bounded user-owned filesystem path.",
      false,
      false,
    ));
  }
  normalize_path(Path::new(connection))
}

fn normalize_path(path: &Path) -> Result<PathBuf, CapabilityFailure> {
  let absolute = if path.is_absolute() {
    path.to_path_buf()
  } else {
    std::env::current_dir()
      .map_err(|_| invalid_input("The database working directory is unavailable."))?
      .join(path)
  };
  let mut normalized = PathBuf::new();
  for component in absolute.components() {
    match component {
      Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
      Component::RootDir => normalized.push(Path::new("/")),
      Component::CurDir => {}
      Component::ParentDir => {
        normalized.pop();
      }
      Component::Normal(component) => normalized.push(component),
    }
  }
  if normalized.is_dir() {
    return Err(invalid_input(
      "The database connection path is a directory.",
    ));
  }
  if normalized.exists() {
    std::fs::canonicalize(&normalized)
      .map_err(|_| invalid_input("The database connection path cannot be resolved."))
  } else {
    Ok(normalized)
  }
}

fn sqlite_failure(error: SqlError, read: bool, write: bool) -> CapabilityFailure {
  let code = match &error {
    SqlError::SqliteFailure(error, _) => Some(error.code),
    _ => None,
  };
  match code {
    Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => database_failure(
      CapabilityFailureKind::ServiceRejected,
      "WOML_DATABASE_BUSY",
      "The database is busy or locked.",
      read,
      false,
    ),
    Some(ErrorCode::ConstraintViolation) => database_failure(
      CapabilityFailureKind::ServiceRejected,
      "WOML_DATABASE_CONSTRAINT",
      "A database constraint rejected the operation.",
      false,
      false,
    ),
    Some(ErrorCode::OperationInterrupted) => database_cancelled(write),
    Some(ErrorCode::CannotOpen | ErrorCode::ReadOnly) => database_failure(
      CapabilityFailureKind::ServiceRejected,
      "WOML_DATABASE_UNAVAILABLE",
      "The database cannot be opened or modified.",
      false,
      false,
    ),
    _ => database_failure(
      CapabilityFailureKind::ServiceRejected,
      if read {
        "WOML_DATABASE_QUERY_FAILED"
      } else {
        "WOML_DATABASE_STATEMENT_FAILED"
      },
      if read {
        "The database query failed."
      } else {
        "The database statement failed."
      },
      false,
      false,
    ),
  }
}

fn database_cancelled(write: bool) -> CapabilityFailure {
  database_failure(
    CapabilityFailureKind::Cancelled,
    "WOML_DATABASE_CANCELLED",
    "The database operation was cancelled.",
    false,
    write,
  )
}

fn invalid_input(message: &str) -> CapabilityFailure {
  database_failure(
    CapabilityFailureKind::InvalidInput,
    "WOML_DATABASE_INPUT_INVALID",
    message,
    false,
    false,
  )
}

fn invalid_result(message: &str) -> CapabilityFailure {
  database_failure(
    CapabilityFailureKind::InvalidResult,
    "WOML_DATABASE_RESULT_INVALID",
    message,
    false,
    false,
  )
}

fn database_failure(
  kind: CapabilityFailureKind,
  code: &str,
  message: &str,
  retryable: bool,
  ambiguous: bool,
) -> CapabilityFailure {
  CapabilityFailure {
    kind,
    code: code.to_string(),
    message: message.to_string(),
    retryable,
    ambiguous,
    details: None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn request(connection: &Path, operation: &str, input: Value) -> Value {
    json!({
      "contract": DATABASE_CONTRACT,
      "contractVersion": DATABASE_CONTRACT_VERSION,
      "kind": "request",
      "driver": "sqlite",
      "connection": connection.to_string_lossy(),
      "operation": operation,
      "input": input,
    })
  }

  fn handler(pool: Arc<ManagedDatabasePool>, operation: &str) -> Arc<ManagedDatabaseHandler> {
    ManagedDatabaseHandler::handlers(pool)
      .into_iter()
      .find(|handler| handler.operation == operation)
      .unwrap()
  }

  fn temporary_database(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
      "woml-database-unit-{name}-{}.sqlite",
      uuid::Uuid::new_v4().simple()
    ))
  }

  #[tokio::test(flavor = "current_thread")]
  async fn contention_is_bounded_and_cancellation_marks_only_writes_ambiguous() {
    let path = temporary_database("safety");
    let pool = Arc::new(ManagedDatabasePool::default());
    let pooled = pool.connection(path.to_str().unwrap()).unwrap();
    let guard = pooled.connection.lock().unwrap();
    let busy = handler(pool.clone(), "execute")
      .execute(
        request(
          &path,
          "execute",
          json!({ "text": "CREATE TABLE item (id INTEGER)" }),
        ),
        CapabilityCancellationToken::default(),
      )
      .await
      .unwrap_err();
    assert_eq!(busy.code, "WOML_DATABASE_BUSY");
    assert!(!busy.ambiguous);
    drop(guard);

    for (operation, input, expected_ambiguous) in [
      ("query", json!({ "text": "SELECT 1 AS value" }), false),
      (
        "execute",
        json!({ "text": "CREATE TABLE item (id INTEGER)" }),
        true,
      ),
    ] {
      let cancellation = CapabilityCancellationToken::default();
      cancellation.cancel();
      let failure = handler(pool.clone(), operation)
        .execute(request(&path, operation, input), cancellation)
        .await
        .unwrap_err();
      assert_eq!(failure.code, "WOML_DATABASE_CANCELLED");
      assert_eq!(failure.ambiguous, expected_ambiguous);
    }
    drop(pooled);
    drop(pool);
    let _ = std::fs::remove_file(path);
  }
}
