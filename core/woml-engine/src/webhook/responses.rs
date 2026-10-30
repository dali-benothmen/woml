//! Private webhook responses.

use crate::webhook::WebhookRoute;
use crate::webhook::WebhookRuntimeState;
use actix_web::http::header;
use actix_web::http::StatusCode;
use actix_web::HttpRequest;
use actix_web::HttpResponse;
use jsonschema::error::ValidationErrorKind;
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::webhook) struct WebhookAcceptedResponse {
  pub(in crate::webhook) run_id: String,
  pub(in crate::webhook) status: &'static str,
  pub(in crate::webhook) duplicate: bool,
}

#[derive(Debug, Serialize)]
pub(in crate::webhook) struct WebhookErrorResponse {
  pub(in crate::webhook) error: WebhookErrorBody,
}

#[derive(Debug, Serialize)]
pub(in crate::webhook) struct WebhookErrorBody {
  pub(in crate::webhook) code: &'static str,
  pub(in crate::webhook) message: &'static str,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub(in crate::webhook) issues: Option<Vec<WebhookSchemaIssue>>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(in crate::webhook) struct WebhookSchemaIssue {
  pub(in crate::webhook) path: String,
  pub(in crate::webhook) message: &'static str,
}

pub(in crate::webhook) fn inbound_trace_context(
  request: &HttpRequest,
) -> crate::InboundTraceContext {
  let mut traceparents = request.headers().get_all("traceparent");
  let traceparent = match (traceparents.next(), traceparents.next()) {
    (None, None) => None,
    (Some(value), None) => match value.to_str() {
      Ok(value) => Some(value),
      Err(_) => return crate::InboundTraceContext::Invalid,
    },
    _ => return crate::InboundTraceContext::Invalid,
  };
  let mut tracestate = String::new();
  for value in request.headers().get_all("tracestate") {
    let Ok(value) = value.to_str() else {
      return crate::InboundTraceContext::Invalid;
    };
    if !tracestate.is_empty() {
      tracestate.push(',');
    }
    tracestate.push_str(value);
    if tracestate.len() > crate::MAX_TRACESTATE_BYTES {
      return crate::InboundTraceContext::Invalid;
    }
  }
  crate::parse_w3c_trace_context(
    traceparent,
    (!tracestate.is_empty()).then_some(tracestate.as_str()),
  )
}

pub(in crate::webhook) fn has_json_content_type(request: &HttpRequest) -> bool {
  request
    .headers()
    .get(header::CONTENT_TYPE)
    .and_then(|value| value.to_str().ok())
    .and_then(|value| value.split(';').next())
    .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case("application/json"))
}

pub(in crate::webhook) fn schema_issue(
  error: jsonschema::ValidationError<'_>,
) -> WebhookSchemaIssue {
  let mut path = error.instance_path().to_string();
  let message = match error.kind() {
    ValidationErrorKind::Required { property } => {
      if let Some(property) = property.as_str() {
        path.push('/');
        path.push_str(&escape_json_pointer(property));
      }
      "Required property is missing."
    }
    ValidationErrorKind::AdditionalProperties { .. }
    | ValidationErrorKind::UnevaluatedProperties { .. } => "Unexpected property is not allowed.",
    ValidationErrorKind::Type { .. } => "Value has the wrong JSON type.",
    ValidationErrorKind::Format { .. } => "String does not match the required format.",
    ValidationErrorKind::Pattern { .. } => "String does not match the required pattern.",
    ValidationErrorKind::MinLength { .. } => "String is shorter than allowed.",
    ValidationErrorKind::MaxLength { .. } => "String is longer than allowed.",
    ValidationErrorKind::Minimum { .. } | ValidationErrorKind::ExclusiveMinimum { .. } => {
      "Number is smaller than allowed."
    }
    ValidationErrorKind::Maximum { .. } | ValidationErrorKind::ExclusiveMaximum { .. } => {
      "Number is larger than allowed."
    }
    _ => "Value does not satisfy the declared schema.",
  };
  WebhookSchemaIssue {
    path: truncate_utf8(path, 2048),
    message,
  }
}

fn escape_json_pointer(value: &str) -> String {
  value.replace('~', "~0").replace('/', "~1")
}

fn truncate_utf8(mut value: String, max_bytes: usize) -> String {
  if value.len() <= max_bytes {
    return value;
  }
  let mut boundary = max_bytes;
  while !value.is_char_boundary(boundary) {
    boundary -= 1;
  }
  value.truncate(boundary);
  value
}

pub(in crate::webhook) fn rejected_response(
  state: &WebhookRuntimeState,
  route: Option<&WebhookRoute>,
  status: StatusCode,
  code: &'static str,
  message: &'static str,
  issues: Option<Vec<WebhookSchemaIssue>>,
) -> HttpResponse {
  state.report_rejected(route, code, message);
  error_response(status, code, message, issues)
}

pub(in crate::webhook) fn error_response(
  status: StatusCode,
  code: &'static str,
  message: &'static str,
  issues: Option<Vec<WebhookSchemaIssue>>,
) -> HttpResponse {
  let mut response = HttpResponse::build(status);
  response.insert_header((header::CACHE_CONTROL, "no-store"));
  if status == StatusCode::METHOD_NOT_ALLOWED {
    response.insert_header((header::ALLOW, "POST"));
  }
  response.json(WebhookErrorResponse {
    error: WebhookErrorBody {
      code,
      message,
      issues,
    },
  })
}
