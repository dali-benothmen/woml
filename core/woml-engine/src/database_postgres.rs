//! PostgreSQL backend for the frozen Database v1 capability.

use std::{
  collections::{HashMap, HashSet},
  ops::{Deref, DerefMut},
  str::FromStr,
  sync::{Arc, Mutex},
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use bytes::BytesMut;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use futures_util::{future::BoxFuture, TryStreamExt};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_postgres::{
  config::SslMode,
  types::{to_sql_checked, FromSqlOwned, IsNull, Json, ToSql, Type},
  Client, Config, Error as PostgresError, GenericClient, NoTls, Row,
};
use tokio_postgres_rustls::MakeRustlsConnect;

use crate::{CapabilityCancellationToken, CapabilityFailure, CapabilityFailureKind};

const DATABASE_CONTRACT: &str = "woml.database";
const DATABASE_CONTRACT_VERSION: u32 = 1;
const MAX_ROWS: usize = 10_000;
const MAX_COLUMNS: usize = 256;
const MAX_RESULT_BYTES: usize = 4_194_304;
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
const MAX_POSTGRES_CONNECTIONS: usize = 16;
const MAX_POSTGRES_POOLS: usize = 64;

#[derive(Default)]
pub(crate) struct ManagedPostgresPools {
  pools: Mutex<HashMap<String, Arc<PostgresPool>>>,
}

impl std::fmt::Debug for ManagedPostgresPools {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter
      .debug_struct("ManagedPostgresPools")
      .field(
        "pool_count",
        &self.pools.lock().map(|pools| pools.len()).unwrap_or(0),
      )
      .finish()
  }
}

impl ManagedPostgresPools {
  fn pool(&self, connection: &str) -> Result<Arc<PostgresPool>, CapabilityFailure> {
    let config = postgres_config(connection)?;
    let key = connection_key(connection);
    let mut pools = self.pools.lock().expect("PostgreSQL pool registry lock");
    if let Some(pool) = pools.get(&key) {
      return Ok(pool.clone());
    }
    if pools.len() >= MAX_POSTGRES_POOLS {
      return Err(postgres_failure(
        CapabilityFailureKind::ServiceRejected,
        "WOML_DATABASE_POOL_LIMIT",
        "The PostgreSQL pool registry reached its configured limit.",
        false,
        false,
      ));
    }
    let pool = Arc::new(PostgresPool::new(config));
    pools.insert(key, pool.clone());
    Ok(pool)
  }
}

pub(crate) fn validate_connection(connection: &str) -> Result<(), CapabilityFailure> {
  postgres_config(connection).map(|_| ())
}

pub(crate) fn execute(
  pools: Arc<ManagedPostgresPools>,
  operation: String,
  connection: String,
  input: Value,
  cancellation: CapabilityCancellationToken,
) -> BoxFuture<'static, Result<Value, CapabilityFailure>> {
  let write = !matches!(operation.as_str(), "query" | "read");
  Box::pin(async move {
    if cancellation.is_cancelled() {
      return Err(postgres_cancelled(false));
    }
    let pool = pools.pool(&connection)?;
    let checkout = pool.checkout();
    tokio::pin!(checkout);
    let mut lease = tokio::select! {
      _ = cancellation.cancelled() => return Err(postgres_cancelled(false)),
      result = &mut checkout => result?,
    };
    let outcome = {
      let execution = execute_operation(&mut lease, &operation, &input);
      tokio::pin!(execution);
      tokio::select! {
        _ = cancellation.cancelled() => None,
        result = &mut execution => Some(result),
      }
    };
    match outcome {
      Some(result) => result.map(|data| database_result(&operation, data)),
      None => {
        lease.discard();
        Err(postgres_cancelled(write))
      }
    }
  })
}

#[derive(Debug)]
struct PostgresPool {
  config: Config,
  idle: Mutex<Vec<Client>>,
  permits: Arc<Semaphore>,
}

impl PostgresPool {
  fn new(config: Config) -> Self {
    Self {
      config,
      idle: Mutex::new(Vec::new()),
      permits: Arc::new(Semaphore::new(MAX_POSTGRES_CONNECTIONS)),
    }
  }

  async fn checkout(self: &Arc<Self>) -> Result<PostgresLease, CapabilityFailure> {
    let permit = self
      .permits
      .clone()
      .acquire_owned()
      .await
      .map_err(|_| postgres_unavailable(false, false))?;
    loop {
      let client = self.idle.lock().expect("PostgreSQL idle pool lock").pop();
      match client {
        Some(client) if client.is_closed() => continue,
        Some(client) => return Ok(PostgresLease::new(self.clone(), permit, client)),
        None => break,
      }
    }
    let client = connect(&self.config).await?;
    Ok(PostgresLease::new(self.clone(), permit, client))
  }
}

struct PostgresLease {
  pool: Arc<PostgresPool>,
  permit: Option<OwnedSemaphorePermit>,
  client: Option<Client>,
  reusable: bool,
}

impl PostgresLease {
  fn new(pool: Arc<PostgresPool>, permit: OwnedSemaphorePermit, client: Client) -> Self {
    Self {
      pool,
      permit: Some(permit),
      client: Some(client),
      reusable: true,
    }
  }

  fn discard(&mut self) {
    self.reusable = false;
    self.client.take();
  }
}

impl Deref for PostgresLease {
  type Target = Client;

  fn deref(&self) -> &Self::Target {
    self.client.as_ref().expect("active PostgreSQL lease")
  }
}

impl DerefMut for PostgresLease {
  fn deref_mut(&mut self) -> &mut Self::Target {
    self.client.as_mut().expect("active PostgreSQL lease")
  }
}

impl Drop for PostgresLease {
  fn drop(&mut self) {
    if self.reusable {
      if let Some(client) = self.client.take().filter(|client| !client.is_closed()) {
        self
          .pool
          .idle
          .lock()
          .expect("PostgreSQL idle pool lock")
          .push(client);
      }
    }
    self.permit.take();
  }
}

async fn connect(config: &Config) -> Result<Client, CapabilityFailure> {
  if config.get_ssl_mode() == SslMode::Disable {
    let (client, connection) = config
      .connect(NoTls)
      .await
      .map_err(|error| postgres_error(error, true, false, false))?;
    tokio::spawn(async move {
      let _ = connection.await;
    });
    return Ok(client);
  }

  let _ = rustls::crypto::ring::default_provider().install_default();
  let tls = MakeRustlsConnect::with_webpki_roots();
  let (client, connection) = config
    .connect(tls)
    .await
    .map_err(|error| postgres_error(error, true, false, false))?;
  tokio::spawn(async move {
    let _ = connection.await;
  });
  Ok(client)
}

fn postgres_config(connection: &str) -> Result<Config, CapabilityFailure> {
  if connection.is_empty() || connection.len() > 4_096 || connection.contains('\0') {
    return Err(postgres_input(
      "PostgreSQL requires a bounded connection URL or libpq-style connection string.",
    ));
  }
  Config::from_str(connection).map_err(|_| {
    postgres_input("The PostgreSQL connection configuration is malformed or unsupported.")
  })
}

fn connection_key(connection: &str) -> String {
  let digest = Sha256::digest(connection.as_bytes());
  hex::encode(digest)
}

async fn execute_operation(
  client: &mut Client,
  operation: &str,
  input: &Value,
) -> Result<Value, CapabilityFailure> {
  match operation {
    "query" => {
      let transaction = client
        .build_transaction()
        .read_only(true)
        .start()
        .await
        .map_err(|error| postgres_error(error, true, false, false))?;
      let result = query_input(&transaction, input).await;
      match result {
        Ok(result) => {
          transaction
            .commit()
            .await
            .map_err(|error| postgres_error(error, true, false, true))?;
          Ok(result)
        }
        Err(error) => {
          let _ = transaction.rollback().await;
          Err(error)
        }
      }
    }
    "execute" => mutation_input(client, input, false).await,
    "read" => read_input(client, input).await,
    "insert" => insert_input(client, input).await,
    "update" => update_input(client, input).await,
    "delete" => delete_input(client, input).await,
    "transaction" => transaction_input(client, input).await,
    _ => Err(postgres_input("The PostgreSQL operation is unsupported.")),
  }
}

async fn query_input<C>(client: &C, input: &Value) -> Result<Value, CapabilityFailure>
where
  C: GenericClient + Sync,
{
  let object = input.as_object().expect("validated PostgreSQL query input");
  query_sql(
    client,
    object["text"].as_str().expect("validated SQL text"),
    object.get("values").and_then(Value::as_array),
  )
  .await
}

async fn query_sql<C>(
  client: &C,
  text: &str,
  values: Option<&Vec<Value>>,
) -> Result<Value, CapabilityFailure>
where
  C: GenericClient + Sync,
{
  let statement = client
    .prepare(text)
    .await
    .map_err(|error| postgres_error(error, true, false, false))?;
  if statement.columns().len() > MAX_COLUMNS {
    return Err(postgres_result(
      "The database query returned too many columns.",
    ));
  }
  let parameters = postgres_parameters(values, statement.params())?;
  let references = parameters
    .iter()
    .map(|value| value.as_ref() as &(dyn ToSql + Sync))
    .collect::<Vec<_>>();
  let rows = client
    .query_raw(&statement, references)
    .await
    .map_err(|error| postgres_error(error, true, false, true))?;
  tokio::pin!(rows);
  let mut result = Vec::new();
  let mut result_bytes = 0_usize;
  while let Some(row) = rows
    .try_next()
    .await
    .map_err(|error| postgres_error(error, true, false, true))?
  {
    if result.len() == MAX_ROWS {
      return Err(postgres_limit(
        "WOML_DATABASE_ROWS_TOO_LARGE",
        "The database query returned more than 10,000 rows.",
      ));
    }
    let row = postgres_row(&row)?;
    result_bytes = result_bytes.saturating_add(
      serde_json::to_vec(&row)
        .map_err(|_| postgres_result("The PostgreSQL row could not be serialized."))?
        .len(),
    );
    if result_bytes > MAX_RESULT_BYTES {
      return Err(postgres_limit(
        "WOML_DATABASE_RESULT_TOO_LARGE",
        "The database result exceeds 4 MiB.",
      ));
    }
    result.push(row);
  }
  let row_count = result.len();
  Ok(json!({ "rows": result, "rowCount": row_count }))
}

async fn mutation_input<C>(
  client: &C,
  input: &Value,
  _insert: bool,
) -> Result<Value, CapabilityFailure>
where
  C: GenericClient + Sync,
{
  let object = input
    .as_object()
    .expect("validated PostgreSQL execute input");
  mutation_sql(
    client,
    object["text"].as_str().expect("validated SQL text"),
    object.get("values").and_then(Value::as_array),
  )
  .await
}

async fn mutation_sql<C>(
  client: &C,
  text: &str,
  values: Option<&Vec<Value>>,
) -> Result<Value, CapabilityFailure>
where
  C: GenericClient + Sync,
{
  let statement = client
    .prepare(text)
    .await
    .map_err(|error| postgres_error(error, false, true, false))?;
  if !statement.columns().is_empty() {
    return Err(postgres_input(
      "db.execute does not accept a statement that returns rows; use db.query.",
    ));
  }
  let parameters = postgres_parameters(values, statement.params())?;
  let references = parameters
    .iter()
    .map(|value| value.as_ref() as &(dyn ToSql + Sync))
    .collect::<Vec<_>>();
  let affected = client
    .execute(&statement, &references)
    .await
    .map_err(|error| postgres_error(error, false, true, true))?;
  safe_affected(affected)
}

async fn read_input<C>(client: &C, input: &Value) -> Result<Value, CapabilityFailure>
where
  C: GenericClient + Sync,
{
  let object = input.as_object().expect("validated PostgreSQL read input");
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
  let where_clause = build_where(object.get("where"), &mut values);
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
    text.push_str(&format!(" OFFSET {offset}"));
  }
  query_sql(client, &text, Some(&values)).await
}

async fn insert_input<C>(client: &C, input: &Value) -> Result<Value, CapabilityFailure>
where
  C: GenericClient + Sync,
{
  let object = input
    .as_object()
    .expect("validated PostgreSQL insert input");
  let table = quote_identifier(object["table"].as_str().expect("validated table"));
  let values = object["values"].as_object().expect("validated values");
  let columns = values
    .keys()
    .map(|column| quote_identifier(column))
    .collect::<Vec<_>>()
    .join(", ");
  let placeholders = (1..=values.len())
    .map(|index| format!("${index}"))
    .collect::<Vec<_>>()
    .join(", ");
  let parameters = values.values().cloned().collect::<Vec<_>>();
  mutation_sql(
    client,
    &format!("INSERT INTO {table} ({columns}) VALUES ({placeholders})"),
    Some(&parameters),
  )
  .await
}

async fn update_input<C>(client: &C, input: &Value) -> Result<Value, CapabilityFailure>
where
  C: GenericClient + Sync,
{
  let object = input
    .as_object()
    .expect("validated PostgreSQL update input");
  let table = quote_identifier(object["table"].as_str().expect("validated table"));
  let update_values = object["values"].as_object().expect("validated values");
  let mut parameters = update_values.values().cloned().collect::<Vec<_>>();
  let assignments = update_values
    .keys()
    .enumerate()
    .map(|(index, column)| format!("{} = ${}", quote_identifier(column), index + 1))
    .collect::<Vec<_>>()
    .join(", ");
  let where_clause = build_where(object.get("where"), &mut parameters);
  mutation_sql(
    client,
    &format!("UPDATE {table} SET {assignments}{where_clause}"),
    Some(&parameters),
  )
  .await
}

async fn delete_input<C>(client: &C, input: &Value) -> Result<Value, CapabilityFailure>
where
  C: GenericClient + Sync,
{
  let object = input
    .as_object()
    .expect("validated PostgreSQL delete input");
  let table = quote_identifier(object["table"].as_str().expect("validated table"));
  let mut parameters = Vec::new();
  let where_clause = build_where(object.get("where"), &mut parameters);
  mutation_sql(
    client,
    &format!("DELETE FROM {table}{where_clause}"),
    Some(&parameters),
  )
  .await
}

async fn transaction_input(client: &mut Client, input: &Value) -> Result<Value, CapabilityFailure> {
  let operations = input["operations"]
    .as_array()
    .expect("validated PostgreSQL transaction operations");
  let transaction = client
    .transaction()
    .await
    .map_err(|error| postgres_error(error, false, true, false))?;
  let mut results = Vec::with_capacity(operations.len());
  for (index, operation) in operations.iter().enumerate() {
    let object = operation
      .as_object()
      .expect("validated transaction operation");
    let name = object["operation"].as_str().expect("validated operation");
    let mut nested = object.clone();
    nested.remove("operation");
    let result = match name {
      "query" => query_input(&transaction, &Value::Object(nested)).await,
      "execute" => mutation_input(&transaction, &Value::Object(nested), false).await,
      "insert" => insert_input(&transaction, &Value::Object(nested)).await,
      "update" => update_input(&transaction, &Value::Object(nested)).await,
      "delete" => delete_input(&transaction, &Value::Object(nested)).await,
      _ => Err(postgres_input("The transaction operation is unsupported.")),
    };
    match result {
      Ok(result) => results.push(result),
      Err(mut failure) => {
        let mut details = failure.details.take().unwrap_or_default();
        details.insert("operationIndex".to_string(), Value::from(index));
        failure.details = Some(details);
        let _ = transaction.rollback().await;
        return Err(failure);
      }
    }
  }
  transaction
    .commit()
    .await
    .map_err(|error| postgres_error(error, false, true, true))?;
  Ok(json!({ "results": results }))
}

fn build_where(value: Option<&Value>, parameters: &mut Vec<Value>) -> String {
  let Some(filters) = value.and_then(Value::as_object) else {
    return String::new();
  };
  if filters.is_empty() {
    return String::new();
  }
  let mut clauses = Vec::with_capacity(filters.len());
  for (column, value) in filters {
    if value.is_null() {
      clauses.push(format!("{} IS NULL", quote_identifier(column)));
    } else {
      parameters.push(value.clone());
      clauses.push(format!(
        "{} = ${}",
        quote_identifier(column),
        parameters.len()
      ));
    }
  }
  format!(" WHERE {}", clauses.join(" AND "))
}

fn quote_identifier(identifier: &str) -> String {
  format!("\"{identifier}\"")
}

fn postgres_parameters(
  values: Option<&Vec<Value>>,
  types: &[Type],
) -> Result<Vec<Box<dyn ToSql + Sync + Send>>, CapabilityFailure> {
  let empty = Vec::new();
  let values = values.unwrap_or(&empty);
  if values.len() != types.len() {
    return Err(postgres_input(
      "The number of PostgreSQL values does not match the prepared parameters.",
    ));
  }
  values
    .iter()
    .zip(types)
    .map(|(value, data_type)| postgres_parameter(value, data_type))
    .collect()
}

#[derive(Debug)]
struct PostgresNull;

impl ToSql for PostgresNull {
  fn to_sql(
    &self,
    _data_type: &Type,
    _output: &mut BytesMut,
  ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
    Ok(IsNull::Yes)
  }

  fn accepts(_data_type: &Type) -> bool {
    true
  }

  to_sql_checked!();
}

fn postgres_parameter(
  value: &Value,
  data_type: &Type,
) -> Result<Box<dyn ToSql + Sync + Send>, CapabilityFailure> {
  if value.is_null() {
    return Ok(Box::new(PostgresNull));
  }
  if data_type == &Type::BOOL {
    return value
      .as_bool()
      .map(|value| Box::new(value) as Box<dyn ToSql + Sync + Send>)
      .ok_or_else(|| postgres_parameter_mismatch(data_type));
  }
  if data_type == &Type::INT2 {
    return value
      .as_i64()
      .and_then(|value| i16::try_from(value).ok())
      .map(|value| Box::new(value) as Box<dyn ToSql + Sync + Send>)
      .ok_or_else(|| postgres_parameter_mismatch(data_type));
  }
  if data_type == &Type::INT4 {
    return value
      .as_i64()
      .and_then(|value| i32::try_from(value).ok())
      .map(|value| Box::new(value) as Box<dyn ToSql + Sync + Send>)
      .ok_or_else(|| postgres_parameter_mismatch(data_type));
  }
  if data_type == &Type::INT8 {
    return value
      .as_i64()
      .filter(|value| (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(value))
      .map(|value| Box::new(value) as Box<dyn ToSql + Sync + Send>)
      .ok_or_else(|| postgres_parameter_mismatch(data_type));
  }
  if data_type == &Type::FLOAT4 {
    return value
      .as_f64()
      .filter(|value| value.is_finite() && *value >= f32::MIN as f64 && *value <= f32::MAX as f64)
      .map(|value| Box::new(value as f32) as Box<dyn ToSql + Sync + Send>)
      .ok_or_else(|| postgres_parameter_mismatch(data_type));
  }
  if data_type == &Type::FLOAT8 {
    return value
      .as_f64()
      .filter(|value| value.is_finite())
      .map(|value| Box::new(value) as Box<dyn ToSql + Sync + Send>)
      .ok_or_else(|| postgres_parameter_mismatch(data_type));
  }
  if matches!(
    *data_type,
    Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::UNKNOWN
  ) {
    return value
      .as_str()
      .map(|value| Box::new(value.to_string()) as Box<dyn ToSql + Sync + Send>)
      .ok_or_else(|| postgres_parameter_mismatch(data_type));
  }
  if data_type == &Type::BYTEA {
    return value
      .get("bytesBase64")
      .and_then(Value::as_str)
      .and_then(|value| BASE64.decode(value).ok())
      .map(|value| Box::new(value) as Box<dyn ToSql + Sync + Send>)
      .ok_or_else(|| postgres_parameter_mismatch(data_type));
  }
  if data_type == &Type::UUID {
    return value
      .as_str()
      .and_then(|value| uuid::Uuid::parse_str(value).ok())
      .map(|value| Box::new(value) as Box<dyn ToSql + Sync + Send>)
      .ok_or_else(|| postgres_parameter_mismatch(data_type));
  }
  if data_type == &Type::DATE {
    return value
      .as_str()
      .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
      .map(|value| Box::new(value) as Box<dyn ToSql + Sync + Send>)
      .ok_or_else(|| postgres_parameter_mismatch(data_type));
  }
  if data_type == &Type::TIMESTAMP {
    return value
      .as_str()
      .and_then(parse_timestamp)
      .map(|value| Box::new(value) as Box<dyn ToSql + Sync + Send>)
      .ok_or_else(|| postgres_parameter_mismatch(data_type));
  }
  if data_type == &Type::TIMESTAMPTZ {
    return value
      .as_str()
      .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
      .map(|value| Box::new(value.with_timezone(&Utc)) as Box<dyn ToSql + Sync + Send>)
      .ok_or_else(|| postgres_parameter_mismatch(data_type));
  }
  if matches!(*data_type, Type::JSON | Type::JSONB) {
    return value
      .as_str()
      .and_then(|value| serde_json::from_str::<Value>(value).ok())
      .map(|value| Box::new(Json(value)) as Box<dyn ToSql + Sync + Send>)
      .ok_or_else(|| postgres_parameter_mismatch(data_type));
  }
  Err(postgres_input(
    "PostgreSQL parameter type is unsupported by Database v1; cast it to a supported SQL type.",
  ))
}

fn parse_timestamp(value: &str) -> Option<NaiveDateTime> {
  NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f")
    .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f"))
    .ok()
}

fn postgres_parameter_mismatch(data_type: &Type) -> CapabilityFailure {
  let _ = data_type;
  postgres_input("A PostgreSQL parameter does not match its prepared SQL type.")
}

fn postgres_row(row: &Row) -> Result<Value, CapabilityFailure> {
  let mut names = HashSet::new();
  let mut result = Map::new();
  for (index, column) in row.columns().iter().enumerate() {
    if !names.insert(column.name()) {
      return Err(postgres_result(
        "The PostgreSQL query returned duplicate column names.",
      ));
    }
    result.insert(
      column.name().to_string(),
      postgres_column(row, index, column.type_())?,
    );
  }
  Ok(Value::Object(result))
}

fn optional<T>(row: &Row, index: usize) -> Result<Option<T>, CapabilityFailure>
where
  T: FromSqlOwned,
{
  row
    .try_get::<_, Option<T>>(index)
    .map_err(|_| postgres_result("A PostgreSQL value could not be converted safely."))
}

fn postgres_column(row: &Row, index: usize, data_type: &Type) -> Result<Value, CapabilityFailure> {
  if data_type == &Type::BOOL {
    return optional::<bool>(row, index).map(|value| value.map_or(Value::Null, Value::Bool));
  }
  if data_type == &Type::INT2 {
    return optional::<i16>(row, index)
      .map(|value| value.map_or(Value::Null, |value| Value::from(i64::from(value))));
  }
  if data_type == &Type::INT4 {
    return optional::<i32>(row, index)
      .map(|value| value.map_or(Value::Null, |value| Value::from(i64::from(value))));
  }
  if data_type == &Type::INT8 {
    return optional::<i64>(row, index).and_then(|value| match value {
      Some(value) if (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&value) => {
        Ok(Value::from(value))
      }
      Some(_) => Err(postgres_result(
        "PostgreSQL returned an integer outside JavaScript's safe range.",
      )),
      None => Ok(Value::Null),
    });
  }
  if data_type == &Type::FLOAT4 {
    return optional::<f32>(row, index).and_then(finite_float);
  }
  if data_type == &Type::FLOAT8 {
    return optional::<f64>(row, index).and_then(finite_float);
  }
  if matches!(
    *data_type,
    Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME
  ) {
    return optional::<String>(row, index).and_then(bounded_string);
  }
  if data_type == &Type::BYTEA {
    return optional::<Vec<u8>>(row, index).and_then(|value| match value {
      Some(value) if value.len() <= MAX_RESULT_BYTES => {
        Ok(json!({ "bytesBase64": BASE64.encode(value) }))
      }
      Some(_) => Err(postgres_limit(
        "WOML_DATABASE_RESULT_TOO_LARGE",
        "A PostgreSQL byte value exceeds 4 MiB.",
      )),
      None => Ok(Value::Null),
    });
  }
  if data_type == &Type::UUID {
    return optional::<uuid::Uuid>(row, index)
      .map(|value| value.map_or(Value::Null, |value| Value::String(value.to_string())));
  }
  if data_type == &Type::DATE {
    return optional::<NaiveDate>(row, index)
      .map(|value| value.map_or(Value::Null, |value| Value::String(value.to_string())));
  }
  if data_type == &Type::TIMESTAMP {
    return optional::<NaiveDateTime>(row, index).map(|value| {
      value.map_or(Value::Null, |value| {
        Value::String(value.format("%Y-%m-%dT%H:%M:%S%.f").to_string())
      })
    });
  }
  if data_type == &Type::TIMESTAMPTZ {
    return optional::<DateTime<Utc>>(row, index)
      .map(|value| value.map_or(Value::Null, |value| Value::String(value.to_rfc3339())));
  }
  if matches!(*data_type, Type::JSON | Type::JSONB) {
    return optional::<Json<Value>>(row, index).and_then(|value| match value {
      Some(Json(value)) => serde_json::to_string(&value)
        .map(Value::String)
        .map_err(|_| postgres_result("A PostgreSQL JSON value could not be serialized.")),
      None => Ok(Value::Null),
    });
  }
  Err(postgres_result(
    "PostgreSQL returned an unsupported type; cast it to a Database v1 result type.",
  ))
}

fn finite_float<T>(value: Option<T>) -> Result<Value, CapabilityFailure>
where
  T: Into<f64>,
{
  match value.map(Into::into) {
    Some(value) if value.is_finite() => Ok(Value::from(value)),
    Some(_) => Err(postgres_result("PostgreSQL returned a non-finite number.")),
    None => Ok(Value::Null),
  }
}

fn bounded_string(value: Option<String>) -> Result<Value, CapabilityFailure> {
  match value {
    Some(value) if value.len() <= MAX_RESULT_BYTES => Ok(Value::String(value)),
    Some(_) => Err(postgres_limit(
      "WOML_DATABASE_RESULT_TOO_LARGE",
      "A PostgreSQL text value exceeds 4 MiB.",
    )),
    None => Ok(Value::Null),
  }
}

fn safe_affected(affected: u64) -> Result<Value, CapabilityFailure> {
  if affected > MAX_SAFE_INTEGER as u64 {
    return Err(postgres_result(
      "PostgreSQL affected-row count exceeds JavaScript's safe integer range.",
    ));
  }
  Ok(json!({ "rowsAffected": affected, "lastInsertId": null }))
}

fn database_result(operation: &str, data: Value) -> Value {
  json!({
    "contract": DATABASE_CONTRACT,
    "contractVersion": DATABASE_CONTRACT_VERSION,
    "kind": "result",
    "operation": operation,
    "data": data,
  })
}

fn postgres_error(
  error: PostgresError,
  read: bool,
  write: bool,
  dispatched: bool,
) -> CapabilityFailure {
  if let Some(database) = error.as_db_error() {
    let code = database.code().code();
    if code.starts_with("23") {
      return postgres_failure(
        CapabilityFailureKind::ServiceRejected,
        "WOML_DATABASE_CONSTRAINT",
        "A PostgreSQL constraint rejected the operation.",
        false,
        false,
      );
    }
    if matches!(code, "40001" | "40P01") {
      return postgres_failure(
        CapabilityFailureKind::ServiceRejected,
        "WOML_DATABASE_TRANSACTION_CONFLICT",
        "PostgreSQL rejected the transaction because of a concurrency conflict.",
        read,
        false,
      );
    }
    if code == "57014" {
      return postgres_cancelled(write && dispatched);
    }
    if code == "25006" {
      return postgres_failure(
        CapabilityFailureKind::InvalidInput,
        "WOML_DATABASE_QUERY_NOT_READ_ONLY",
        "db.query accepts only a read-only PostgreSQL statement; use db.execute for writes.",
        false,
        false,
      );
    }
    return postgres_failure(
      CapabilityFailureKind::ServiceRejected,
      if read {
        "WOML_DATABASE_QUERY_FAILED"
      } else {
        "WOML_DATABASE_STATEMENT_FAILED"
      },
      if read {
        "The PostgreSQL query failed."
      } else {
        "The PostgreSQL statement failed."
      },
      false,
      false,
    );
  }
  postgres_failure(
    CapabilityFailureKind::TransportFailed,
    "WOML_DATABASE_CONNECTION_LOST",
    "The PostgreSQL connection is unavailable or was lost.",
    read,
    write && dispatched,
  )
}

fn postgres_unavailable(retryable: bool, ambiguous: bool) -> CapabilityFailure {
  postgres_failure(
    CapabilityFailureKind::TransportFailed,
    "WOML_DATABASE_UNAVAILABLE",
    "The PostgreSQL connection pool is unavailable.",
    retryable,
    ambiguous,
  )
}

fn postgres_cancelled(ambiguous: bool) -> CapabilityFailure {
  postgres_failure(
    CapabilityFailureKind::Cancelled,
    "WOML_DATABASE_CANCELLED",
    "The PostgreSQL operation was cancelled.",
    false,
    ambiguous,
  )
}

fn postgres_input(message: &str) -> CapabilityFailure {
  postgres_failure(
    CapabilityFailureKind::InvalidInput,
    "WOML_DATABASE_INPUT_INVALID",
    message,
    false,
    false,
  )
}

fn postgres_result(message: &str) -> CapabilityFailure {
  postgres_failure(
    CapabilityFailureKind::InvalidResult,
    "WOML_DATABASE_RESULT_INVALID",
    message,
    false,
    false,
  )
}

fn postgres_limit(code: &str, message: &str) -> CapabilityFailure {
  postgres_failure(
    CapabilityFailureKind::ResultTooLarge,
    code,
    message,
    false,
    false,
  )
}

fn postgres_failure(
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
