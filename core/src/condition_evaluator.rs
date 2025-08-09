//! Condition evaluation for control flow in Node-Cronflow Core Engine
//! 
//! This module provides condition evaluation functionality for if/else control flow,
//! including parsing condition expressions and evaluating them against workflow context.

use crate::error::{CoreError, CoreResult};
use crate::models::{ConditionResult, StepResult};
use crate::context::Context;
use serde_json::Value;
use std::collections::HashMap;

/// Condition evaluator for workflow control flow
pub struct ConditionEvaluator {
    /// Workflow context for evaluation
    context: Context,
    /// Completed step results for reference
    completed_steps: HashMap<String, StepResult>,
}

impl ConditionEvaluator {
    /// Create a new condition evaluator
    pub fn new(context: Context, completed_steps: Vec<StepResult>) -> Self {
        let mut steps_map = HashMap::new();
        for step_result in completed_steps {
            steps_map.insert(step_result.step_id.clone(), step_result);
        }
        
        Self {
            context,
            completed_steps: steps_map,
        }
    }
    
    /// Evaluate a condition expression
    pub fn evaluate_condition(&self, condition_expr: &str) -> CoreResult<ConditionResult> {
        log::debug!("Evaluating condition: {}", condition_expr);
        
        let parsed_condition = self.parse_condition_expression(condition_expr)?;
        
        // Evaluate the parsed condition
        let result = self.evaluate_parsed_condition(&parsed_condition)?;
        
        log::debug!("Condition evaluation result: {}", result.met);
        Ok(result)
    }
    
    /// Parse a condition expression into evaluable components
    fn parse_condition_expression(&self, expr: &str) -> CoreResult<ParsedCondition> {
        // Remove whitespace and normalize
        let expr = expr.trim();
        
        // Handle simple boolean expressions
        if expr == "true" {
            return Ok(ParsedCondition::Boolean(true));
        }
        if expr == "false" {
            return Ok(ParsedCondition::Boolean(false));
        }
        
        // Handle context references (ctx.payload.field, ctx.last.field, etc.)
        if expr.starts_with("ctx.") {
            return self.parse_context_reference(expr);
        }
        
        // Handle step result references (ctx.steps.step_id.output.field)
        if expr.starts_with("ctx.steps.") {
            return self.parse_step_reference(expr);
        }
        
        // Handle comparison expressions (field > value, field == value, etc.)
        if self.contains_comparison_operator(expr) {
            return self.parse_comparison_expression(expr);
        }
        
        Ok(ParsedCondition::FieldReference(expr.to_string()))
    }
    
    /// Parse context reference expressions
    fn parse_context_reference(&self, expr: &str) -> CoreResult<ParsedCondition> {
        // Handle ctx.payload.field
        if expr.starts_with("ctx.payload.") {
            let field_path = &expr[13..]; // Remove "ctx.payload."
            return Ok(ParsedCondition::PayloadField(field_path.to_string()));
        }
        
        // Handle ctx.last.field
        if expr.starts_with("ctx.last.") {
            let field_path = &expr[9..]; // Remove "ctx.last."
            return Ok(ParsedCondition::LastStepField(field_path.to_string()));
        }
        
        Err(CoreError::Validation(format!("Unsupported context reference: {}", expr)))
    }
    
    /// Parse step result reference expressions
    fn parse_step_reference(&self, expr: &str) -> CoreResult<ParsedCondition> {
        // Handle ctx.steps.step_id.output.field
        let parts: Vec<&str> = expr.split('.').collect();
        if parts.len() >= 4 && parts[0] == "ctx" && parts[1] == "steps" {
            let step_id = parts[2];
            let field_type = parts[3];
            
            match field_type {
                "output" => {
                    if parts.len() >= 5 {
                        let field_path = parts[4..].join(".");
                        return Ok(ParsedCondition::StepOutput(step_id.to_string(), field_path));
                    } else {
                        return Ok(ParsedCondition::StepOutput(step_id.to_string(), "".to_string()));
                    }
                },
                "error" => {
                    return Ok(ParsedCondition::StepError(step_id.to_string()));
                },
                "status" => {
                    return Ok(ParsedCondition::StepStatus(step_id.to_string()));
                },
                _ => {
                    return Err(CoreError::Validation(format!("Unsupported step field: {}", field_type)));
                }
            }
        }
        
        Err(CoreError::Validation(format!("Invalid step reference format: {}", expr)))
    }
    
    /// Parse comparison expressions
    fn parse_comparison_expression(&self, expr: &str) -> CoreResult<ParsedCondition> {
        let operators = ["==", "!=", ">=", "<=", ">", "<"];
        
        for op in &operators {
            if expr.contains(op) {
                let parts: Vec<&str> = expr.split(op).collect();
                if parts.len() == 2 {
                    let left = parts[0].trim();
                    let right = parts[1].trim();
                    
                    return Ok(ParsedCondition::Comparison(
                        left.to_string(),
                        op.to_string(),
                        right.to_string()
                    ));
                }
            }
        }
        
        Err(CoreError::Validation(format!("Invalid comparison expression: {}", expr)))
    }
    
    /// Check if expression contains comparison operators
    fn contains_comparison_operator(&self, expr: &str) -> bool {
        let operators = ["==", "!=", ">=", "<=", ">", "<"];
        operators.iter().any(|op| expr.contains(op))
    }
    
    /// Evaluate a parsed condition
    fn evaluate_parsed_condition(&self, condition: &ParsedCondition) -> CoreResult<ConditionResult> {
        match condition {
            ParsedCondition::Boolean(value) => {
                Ok(ConditionResult::success(*value))
            },
            ParsedCondition::PayloadField(field_path) => {
                let value = self.get_payload_field(field_path)?;
                Ok(ConditionResult::success(self.is_truthy(&value)))
            },
            ParsedCondition::LastStepField(field_path) => {
                let value = self.get_last_step_field(field_path)?;
                Ok(ConditionResult::success(self.is_truthy(&value)))
            },
            ParsedCondition::StepOutput(step_id, field_path) => {
                let value = self.get_step_output_field(step_id, field_path)?;
                Ok(ConditionResult::success(self.is_truthy(&value)))
            },
            ParsedCondition::StepError(step_id) => {
                let has_error = self.get_step_error(step_id)?;
                Ok(ConditionResult::success(has_error))
            },
            ParsedCondition::StepStatus(step_id) => {
                let status = self.get_step_status(step_id)?;
                Ok(ConditionResult::success(status == "completed"))
            },
            ParsedCondition::FieldReference(field_path) => {
                let value = self.get_field_reference(field_path)?;
                Ok(ConditionResult::success(self.is_truthy(&value)))
            },
            ParsedCondition::Comparison(left, op, right) => {
                let result = self.evaluate_comparison(left, op, right)?;
                Ok(ConditionResult::success(result))
            },
        }
    }
    
    /// Get a field from the payload
    fn get_payload_field(&self, field_path: &str) -> CoreResult<Value> {
        let payload = &self.context.payload;
        self.get_nested_field(payload, field_path)
    }
    
    /// Get a field from the last step result
    fn get_last_step_field(&self, field_path: &str) -> CoreResult<Value> {
        // In the future, this should access the actual last step result
        if field_path.is_empty() {
            Ok(Value::Null)
        } else {
            // Try to get from the most recent step result
            let completed_steps: Vec<&StepResult> = self.completed_steps.values().collect();
            if let Some(last_step) = completed_steps.last() {
                if let Some(output) = &last_step.output {
                    self.get_nested_field(output, field_path)
                } else {
                    Ok(Value::Null)
                }
            } else {
                Ok(Value::Null)
            }
        }
    }
    
    /// Get a field from a step's output
    fn get_step_output_field(&self, step_id: &str, field_path: &str) -> CoreResult<Value> {
        if let Some(step_result) = self.completed_steps.get(step_id) {
            if let Some(output) = &step_result.output {
                if field_path.is_empty() {
                    Ok(output.clone())
                } else {
                    self.get_nested_field(output, field_path)
                }
            } else {
                Ok(Value::Null)
            }
        } else {
            Ok(Value::Null)
        }
    }
    
    /// Check if a step has an error
    fn get_step_error(&self, step_id: &str) -> CoreResult<bool> {
        if let Some(step_result) = self.completed_steps.get(step_id) {
            Ok(step_result.error.is_some())
        } else {
            Ok(false)
        }
    }
    
    /// Get a step's status
    fn get_step_status(&self, step_id: &str) -> CoreResult<String> {
        if let Some(step_result) = self.completed_steps.get(step_id) {
            Ok(step_result.status.as_str().to_string())
        } else {
            Ok("pending".to_string())
        }
    }
    
    /// Get a field reference (fallback method)
    fn get_field_reference(&self, field_path: &str) -> CoreResult<Value> {
        // Try payload first
        if let Ok(value) = self.get_payload_field(field_path) {
            if value != Value::Null {
                return Ok(value);
            }
        }
        
        // Try last step
        if let Ok(value) = self.get_last_step_field(field_path) {
            if value != Value::Null {
                return Ok(value);
            }
        }
        
        Ok(Value::Null)
    }
    
    /// Evaluate a comparison expression
    fn evaluate_comparison(&self, left: &str, op: &str, right: &str) -> CoreResult<bool> {
        let left_value = if left.starts_with("ctx.") {
            let parsed = self.parse_context_reference(left)?;
            match parsed {
                ParsedCondition::PayloadField(field_path) => self.get_payload_field(&field_path)?,
                ParsedCondition::LastStepField(field_path) => self.get_last_step_field(&field_path)?,
                _ => return Err(CoreError::Validation("Unsupported left operand in comparison".to_string())),
            }
        } else {
            // Try to parse as number or string
            if let Ok(num) = left.parse::<f64>() {
                Value::Number(serde_json::Number::from_f64(num).unwrap_or_else(|| serde_json::Number::from(0)))
            } else {
                Value::String(left.to_string())
            }
        };
        
        let right_value = if right.starts_with("ctx.") {
            let parsed = self.parse_context_reference(right)?;
            match parsed {
                ParsedCondition::PayloadField(field_path) => self.get_payload_field(&field_path)?,
                ParsedCondition::LastStepField(field_path) => self.get_last_step_field(&field_path)?,
                _ => return Err(CoreError::Validation("Unsupported right operand in comparison".to_string())),
            }
        } else {
            // Try to parse as number or string
            if let Ok(num) = right.parse::<f64>() {
                Value::Number(serde_json::Number::from_f64(num).unwrap_or_else(|| serde_json::Number::from(0)))
            } else {
                Value::String(right.to_string())
            }
        };
        
        // Perform comparison
        match op {
            "==" => Ok(left_value == right_value),
            "!=" => Ok(left_value != right_value),
            ">" => self.compare_values(&left_value, &right_value, |a, b| a > b),
            "<" => self.compare_values(&left_value, &right_value, |a, b| a < b),
            ">=" => self.compare_values(&left_value, &right_value, |a, b| a >= b),
            "<=" => self.compare_values(&left_value, &right_value, |a, b| a <= b),
            _ => Err(CoreError::Validation(format!("Unsupported comparison operator: {}", op))),
        }
    }
    
    /// Compare two values using a comparison function
    fn compare_values<F>(&self, left: &Value, right: &Value, compare_fn: F) -> CoreResult<bool>
    where
        F: Fn(f64, f64) -> bool,
    {
        let left_num = self.value_to_number(left)?;
        let right_num = self.value_to_number(right)?;
        Ok(compare_fn(left_num, right_num))
    }
    
    /// Convert a JSON value to a number
    fn value_to_number(&self, value: &Value) -> CoreResult<f64> {
        match value {
            Value::Number(n) => n.as_f64().ok_or_else(|| {
                CoreError::Validation("Invalid number value".to_string())
            }),
            Value::String(s) => s.parse::<f64>().map_err(|_| {
                CoreError::Validation(format!("Cannot convert string '{}' to number", s))
            }),
            _ => Err(CoreError::Validation("Cannot convert value to number".to_string())),
        }
    }
    
    /// Get a nested field from a JSON value
    fn get_nested_field(&self, value: &Value, field_path: &str) -> CoreResult<Value> {
        let parts: Vec<&str> = field_path.split('.').collect();
        let mut current = value;
        
        for part in parts {
            match current {
                Value::Object(map) => {
                    if let Some(field_value) = map.get(part) {
                        current = field_value;
                    } else {
                        return Ok(Value::Null);
                    }
                },
                _ => {
                    return Ok(Value::Null);
                }
            }
        }
        
        Ok(current.clone())
    }
    
    /// Check if a value is truthy
    fn is_truthy(&self, value: &Value) -> bool {
        match value {
            Value::Bool(b) => *b,
            Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
            Value::String(s) => !s.is_empty(),
            Value::Array(arr) => !arr.is_empty(),
            Value::Object(obj) => !obj.is_empty(),
            Value::Null => false,
        }
    }
}

/// Parsed condition representation
#[derive(Debug, Clone)]
enum ParsedCondition {
    Boolean(bool),
    PayloadField(String),
    LastStepField(String),
    StepOutput(String, String),
    StepError(String),
    StepStatus(String),
    FieldReference(String),
    Comparison(String, String, String),
} 