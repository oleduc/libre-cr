//! `Tool` trait and dispatcher.

use crate::error::ToolError;
use crate::tools::context::ToolContext;
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub type ToolFuture<'a> = Pin<Box<dyn Future<Output = Result<Value, ToolError>> + Send + 'a>>;

pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn input_schema(&self) -> Value;
    fn call<'a>(&'a self, ctx: Arc<ToolContext>, input: Value) -> ToolFuture<'a>;
}

pub struct ToolRegistry {
    by_name: HashMap<String, Arc<dyn Tool>>,
    order: Vec<String>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            by_name: HashMap::new(),
            order: Vec::new(),
        }
    }

    pub fn register<T: Tool + 'static>(&mut self, tool: T) {
        let name = tool.name().to_string();
        self.order.push(name.clone());
        self.by_name.insert(name, Arc::new(tool));
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.by_name.get(name).cloned()
    }

    pub fn all(&self) -> Vec<Arc<dyn Tool>> {
        self.order
            .iter()
            .filter_map(|n| self.by_name.get(n).cloned())
            .collect()
    }

    pub async fn call(
        &self,
        name: &str,
        ctx: Arc<ToolContext>,
        input: Value,
    ) -> Result<Value, ToolError> {
        let tool = self
            .get(name)
            .ok_or_else(|| ToolError::invalid(format!("unknown tool: {name}")))?;
        validate_against(&tool.input_schema(), &input)?;
        tool.call(ctx, input).await
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Minimal hand-rolled validator: enforces required keys and primitive types.
pub fn validate_against(schema: &Value, input: &Value) -> Result<(), ToolError> {
    let Some(schema_obj) = schema.as_object() else {
        return Ok(());
    };
    let Some(input_obj) = input.as_object() else {
        // Allow null/missing if no properties are required.
        if schema_obj
            .get("required")
            .and_then(|r| r.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false)
        {
            return Err(ToolError::invalid("expected an object"));
        }
        return Ok(());
    };

    if let Some(required) = schema_obj.get("required").and_then(|r| r.as_array()) {
        for k in required {
            if let Some(key) = k.as_str() {
                if !input_obj.contains_key(key) {
                    return Err(ToolError::invalid(format!("missing required field: {key}")));
                }
            }
        }
    }

    if let Some(props) = schema_obj.get("properties").and_then(|p| p.as_object()) {
        for (k, v) in input_obj {
            if let Some(prop_schema) = props.get(k) {
                if let Some(t) = prop_schema.get("type").and_then(|s| s.as_str()) {
                    if !matches_type(t, v) {
                        return Err(ToolError::invalid(format!("field {k:?} expected type {t}")));
                    }
                }
            }
        }
    }
    Ok(())
}

fn matches_type(t: &str, v: &Value) -> bool {
    match t {
        "string" => v.is_string(),
        "number" | "integer" => v.is_number(),
        "boolean" => v.is_boolean(),
        "object" => v.is_object(),
        "array" => v.is_array(),
        "null" => v.is_null(),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn requires_listed_fields() {
        let schema = json!({ "type": "object", "required": ["x"], "properties": {} });
        let bad = json!({});
        assert!(validate_against(&schema, &bad).is_err());
        let good = json!({ "x": 1 });
        assert!(validate_against(&schema, &good).is_ok());
    }

    #[test]
    fn checks_property_types() {
        let schema = json!({
            "type": "object",
            "properties": { "x": { "type": "string" } }
        });
        let bad = json!({ "x": 1 });
        assert!(validate_against(&schema, &bad).is_err());
        let good = json!({ "x": "ok" });
        assert!(validate_against(&schema, &good).is_ok());
    }
}
