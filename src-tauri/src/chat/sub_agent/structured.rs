use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

use super::spawn::CompletionPolicy;

fn check_references(schema: &Value) -> Result<(), String> {
    for key in ["$ref", "$dynamicRef", "$recursiveRef"] {
        if let Some(value) = schema.get(key) {
            let reference = value.as_str().ok_or("Schema references must be strings.")?;
            if !reference.is_empty() && !reference.starts_with('#') {
                return Err("Output schemas may only reference their own document.".into());
            }
        }
    }
    for key in SCHEMA_MAPS {
        if let Some(map) = schema.get(key).and_then(Value::as_object) {
            for value in map
                .values()
                .filter(|value| value.is_object() || value.is_boolean())
            {
                check_references(value)?;
            }
        }
    }
    for key in SCHEMA_NODES {
        if let Some(value) = schema.get(key) {
            if let Some(items) = value.as_array() {
                for item in items {
                    check_references(item)?;
                }
            } else {
                check_references(value)?;
            }
        }
    }
    for key in SCHEMA_ARRAYS {
        if let Some(items) = schema.get(key).and_then(Value::as_array) {
            for item in items {
                check_references(item)?;
            }
        }
    }
    Ok(())
}

const SCHEMA_MAPS: &[&str] = &[
    "properties",
    "patternProperties",
    "$defs",
    "definitions",
    "dependentSchemas",
    "dependencies",
];
const SCHEMA_NODES: &[&str] = &[
    "additionalProperties",
    "additionalItems",
    "items",
    "contains",
    "propertyNames",
    "not",
    "if",
    "then",
    "else",
    "unevaluatedItems",
    "unevaluatedProperties",
    "contentSchema",
];
const SCHEMA_ARRAYS: &[&str] = &["allOf", "anyOf", "oneOf", "prefixItems"];
const JTD_TYPES: &[&str] = &[
    "boolean",
    "string",
    "timestamp",
    "float32",
    "float64",
    "int8",
    "uint8",
    "int16",
    "uint16",
    "int32",
    "uint32",
];

fn is_jtd_document(schema: &Value, allow_number: bool) -> bool {
    let Some(schema) = schema.as_object() else {
        return false;
    };
    schema.iter().all(|(key, value)| match key.as_str() {
        "metadata" => true,
        "nullable" | "additionalProperties" => value.is_boolean(),
        "ref" | "discriminator" => value.is_string(),
        "type" => value
            .as_str()
            .is_some_and(|kind| JTD_TYPES.contains(&kind) || (allow_number && kind == "number")),
        "enum" => value
            .as_array()
            .is_some_and(|values| values.iter().all(Value::is_string)),
        "elements" | "values" => is_jtd_document(value, allow_number),
        "properties" | "optionalProperties" | "definitions" | "mapping" => {
            value.as_object().is_some_and(|map| {
                map.values()
                    .all(|value| is_jtd_document(value, allow_number))
            })
        }
        _ => false,
    })
}

fn convert_jtd(schema: &Value) -> Result<Value, String> {
    // OMP passes `number` through inside explicit JTD forms, but not for format detection.
    if !is_jtd_document(schema, true) {
        return Err("Invalid JTD output schema.".into());
    }
    let forms = ["enum", "elements", "type", "values", "discriminator", "ref"]
        .iter()
        .filter(|key| schema.get(**key).is_some())
        .count()
        + usize::from(
            schema.get("properties").is_some() || schema.get("optionalProperties").is_some(),
        );
    if forms > 1 || (schema.get("mapping").is_some() && schema.get("discriminator").is_none()) {
        return Err("JTD output schemas must declare one schema form.".into());
    }
    let mut converted = if let Some(values) = schema.get("enum") {
        if values.as_array().is_none_or(Vec::is_empty) {
            return Err("JTD enum must not be empty.".into());
        }
        json!({"enum":values})
    } else if let Some(elements) = schema.get("elements") {
        json!({"type":"array","items":convert_jtd(elements)?})
    } else if let Some(kind) = schema.get("type").and_then(Value::as_str) {
        let kind = match kind {
            "timestamp" => "string",
            "float32" | "float64" | "number" => "number",
            "boolean" | "string" => kind,
            _ => "integer",
        };
        json!({"type":kind})
    } else if let Some(values) = schema.get("values") {
        json!({"type":"object","additionalProperties":convert_jtd(values)?})
    } else if schema.get("properties").is_some() || schema.get("optionalProperties").is_some() {
        let mut properties = Map::new();
        let mut required = Vec::new();
        for keyword in ["properties", "optionalProperties"] {
            if let Some(fields) = schema.get(keyword).and_then(Value::as_object) {
                for (key, value) in fields {
                    if properties.contains_key(key) {
                        return Err(format!(
                            "JTD property `{key}` is both required and optional."
                        ));
                    }
                    properties.insert(key.clone(), convert_jtd(value)?);
                    if keyword == "properties" {
                        required.push(key.clone());
                    }
                }
            }
        }
        let mut result =
            json!({"type":"object","properties":properties,"additionalProperties":false});
        if !required.is_empty() {
            result["required"] = json!(required);
        }
        result
    } else if let Some(discriminator) = schema.get("discriminator").and_then(Value::as_str) {
        let mapping = schema
            .get("mapping")
            .and_then(Value::as_object)
            .ok_or("JTD discriminator requires a mapping.")?;
        let mut variants = Vec::new();
        for (tag, value) in mapping {
            let mut variant = convert_jtd(value)?;
            if variant.get("type").and_then(Value::as_str) != Some("object")
                || variant.get("properties").is_none()
            {
                return Err("JTD discriminator mappings must use properties.".into());
            }
            variant["properties"][discriminator] = json!({"const":tag});
            let required = variant
                .as_object_mut()
                .expect("converted object")
                .entry("required")
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .expect("required array");
            if !required.contains(&json!(discriminator)) {
                required.push(json!(discriminator));
            }
            variants.push(variant);
        }
        json!({"oneOf":variants})
    } else if let Some(reference) = schema.get("ref").and_then(Value::as_str) {
        json!({"$ref":format!("#/$defs/{}", reference.replace('~', "~0").replace('/', "~1"))})
    } else {
        json!({})
    };
    if schema.get("nullable") == Some(&Value::Bool(true)) {
        converted = json!({"anyOf":[converted,{"type":"null"}]});
    }
    // Keep local definitions so converted JTD references remain enforceable.
    if let Some(definitions) = schema.get("definitions").and_then(Value::as_object) {
        let definitions = definitions
            .iter()
            .map(|(name, schema)| Ok((name.clone(), convert_jtd(schema)?)))
            .collect::<Result<Map<_, _>, String>>()?;
        converted["$defs"] = Value::Object(definitions);
    }
    Ok(converted)
}

fn normalize_schema_node(schema: &Value, root: bool) -> Result<Value, String> {
    if !schema.is_object() {
        return Ok(schema.clone());
    }
    let explicit_jtd = [
        "elements",
        "values",
        "optionalProperties",
        "discriminator",
        "ref",
    ]
    .iter()
    .any(|key| schema.get(key).is_some());
    let root_jtd = root
        && schema.get("properties").is_some()
        && schema.get("type").is_none()
        && is_jtd_document(schema, false);
    let jtd_type = schema
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| JTD_TYPES.contains(&kind) && !matches!(kind, "boolean" | "string"));
    if explicit_jtd || root_jtd || jtd_type {
        return convert_jtd(schema);
    }
    let mut normalized = schema.clone();
    for keyword in SCHEMA_MAPS {
        if let Some(map) = normalized.get_mut(*keyword).and_then(Value::as_object_mut) {
            for value in map.values_mut() {
                *value = normalize_schema_node(value, false)?;
            }
        }
    }
    for keyword in SCHEMA_NODES.iter().chain(SCHEMA_ARRAYS) {
        if let Some(value) = normalized.get_mut(*keyword) {
            if let Some(items) = value.as_array_mut() {
                for item in items {
                    *item = normalize_schema_node(item, false)?;
                }
            } else {
                *value = normalize_schema_node(value, false)?;
            }
        }
    }
    Ok(normalized)
}

pub(crate) fn normalize_schema(schema: &Value) -> Result<Value, String> {
    let parsed;
    let schema = if let Some(text) = schema.as_str() {
        parsed = serde_json::from_str(text)
            .map_err(|error| format!("Invalid output schema JSON: {error}"))?;
        &parsed
    } else {
        schema
    };
    if !schema.is_object() && !schema.is_boolean() {
        return Err("Output schema must be a JSON Schema or JTD object or boolean.".into());
    }
    let schema = normalize_schema_node(schema, true)?;
    check_references(&schema)?;
    jsonschema::validator_for(&schema)
        .map_err(|error| format!("Invalid output schema: {error}"))?;
    Ok(schema)
}

pub(crate) fn validate_schema(schema: &Value) -> Result<(), String> {
    normalize_schema(schema).map(|_| ())
}

pub(crate) fn validate_result(schema: &Value, value: &Value) -> Result<(), String> {
    let schema = normalize_schema(schema)?;
    validate_json_result(&schema, value)
}

fn validate_json_result(schema: &Value, value: &Value) -> Result<(), String> {
    let validator = jsonschema::validator_for(schema)
        .map_err(|error| format!("Invalid output schema: {error}"))?;
    let errors: Vec<_> = validator
        .iter_errors(value)
        .take(8)
        .map(|error| error.to_string())
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn referenced_schema<'a>(schema: &'a Value, root: &'a Value) -> Option<&'a Value> {
    fn anchor<'a>(schema: &'a Value, name: &str) -> Option<&'a Value> {
        if schema.get("$anchor").and_then(Value::as_str) == Some(name)
            || schema.get("$dynamicAnchor").and_then(Value::as_str) == Some(name)
        {
            return Some(schema);
        }
        for keyword in SCHEMA_MAPS {
            if let Some(map) = schema.get(*keyword).and_then(Value::as_object) {
                for schema in map.values() {
                    if let Some(target) = anchor(schema, name) {
                        return Some(target);
                    }
                }
            }
        }
        for keyword in SCHEMA_NODES.iter().chain(SCHEMA_ARRAYS) {
            if let Some(schema) = schema.get(*keyword) {
                if let Some(items) = schema.as_array() {
                    for schema in items {
                        if let Some(target) = anchor(schema, name) {
                            return Some(target);
                        }
                    }
                } else if let Some(target) = anchor(schema, name) {
                    return Some(target);
                }
            }
        }
        None
    }
    schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|reference| {
            if reference.is_empty() {
                Some(root)
            } else {
                reference.strip_prefix('#').and_then(|reference| {
                    if reference.is_empty() || reference.starts_with('/') {
                        root.pointer(reference)
                    } else {
                        anchor(root, reference)
                    }
                })
            }
        })
}

fn resolve_schema<'a>(mut schema: &'a Value, root: &'a Value) -> &'a Value {
    for _ in 0..64 {
        let Some(target) = referenced_schema(schema, root) else {
            break;
        };
        schema = target;
    }
    schema
}

fn validate_subschema(schema: &Value, root: &Value, value: &Value) -> Result<(), String> {
    fn rebase(schema: &Value) -> Value {
        let mut schema = schema.clone();
        for keyword in ["$ref", "$dynamicRef", "$recursiveRef"] {
            if let Some(reference) = schema.get_mut(keyword) {
                if let Some(pointer) = reference
                    .as_str()
                    .and_then(|reference| reference.strip_prefix("#/"))
                {
                    *reference = json!(format!("#/$defs/__output/{pointer}"));
                } else if matches!(reference.as_str(), Some("#" | "")) {
                    *reference = json!("#/$defs/__output");
                }
            }
        }
        for keyword in SCHEMA_MAPS {
            if let Some(map) = schema.get_mut(*keyword).and_then(Value::as_object_mut) {
                for value in map.values_mut() {
                    *value = rebase(value);
                }
            }
        }
        for keyword in SCHEMA_NODES.iter().chain(SCHEMA_ARRAYS) {
            if let Some(value) = schema.get_mut(*keyword) {
                if let Some(items) = value.as_array_mut() {
                    for item in items {
                        *item = rebase(item);
                    }
                } else {
                    *value = rebase(value);
                }
            }
        }
        schema
    }
    let wrapper = json!({"$defs":{"__output":rebase(root)},"allOf":[rebase(schema)]});
    validate_json_result(&wrapper, value)
}

fn normalize_output(value: &Value, schema: &Value, root: &Value, depth: usize) -> Value {
    if depth > 64 {
        return value.clone();
    }
    let mut value = referenced_schema(schema, root).map_or_else(
        || value.clone(),
        |target| normalize_output(value, target, root, depth + 1),
    );
    if let Some(branches) = schema
        .get("anyOf")
        .and_then(Value::as_array)
        .filter(|branches| branches.len() == 2)
    {
        if let Some(other) = branches
            .iter()
            .position(|branch| branch.get("type").and_then(Value::as_str) == Some("null"))
        {
            return normalize_output(&value, &branches[1 - other], root, depth + 1);
        }
    }
    if let Some(branches) = schema.get("allOf").and_then(Value::as_array) {
        for branch in branches {
            value = normalize_output(&value, branch, root, depth + 1);
        }
    }
    for keyword in ["oneOf", "anyOf"] {
        if let Some(branches) = schema.get(keyword).and_then(Value::as_array) {
            if validate_subschema(schema, root, &value).is_ok() {
                return value;
            }
            for branch in branches {
                let candidate = normalize_output(&value, branch, root, depth + 1);
                if candidate != value && validate_subschema(schema, root, &candidate).is_ok() {
                    return candidate;
                }
            }
        }
    }
    if let Some(items) = value.as_array_mut() {
        for (index, item) in items.iter_mut().enumerate() {
            if let Some(item_schema) = schema
                .get("prefixItems")
                .and_then(Value::as_array)
                .and_then(|items| items.get(index))
                .or_else(|| schema.get("items"))
            {
                *item = normalize_output(item, item_schema, root, depth + 1);
            }
        }
    }
    if let (Some(object), Some(properties)) = (
        value.as_object_mut(),
        schema.get("properties").and_then(Value::as_object),
    ) {
        for (key, property_schema) in properties {
            let Some(property) = object.get(key) else {
                continue;
            };
            let required = schema
                .get("required")
                .and_then(Value::as_array)
                .is_some_and(|required| required.iter().any(|field| field.as_str() == Some(key)));
            if property.is_null()
                && !required
                && validate_subschema(property_schema, root, &Value::Null).is_err()
            {
                object.remove(key);
            } else {
                object.insert(
                    key.clone(),
                    normalize_output(property, property_schema, root, depth + 1),
                );
            }
        }
    }
    value
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct YieldInput {
    #[serde(default, rename = "type")]
    pub kind: Option<Value>,
    #[serde(default)]
    pub data: Option<Value>,
    #[serde(default)]
    pub use_last_turn: bool,
    #[serde(default, skip_deserializing)]
    pub schema_overridden: bool,
}

impl YieldInput {
    fn labels(&self) -> Result<Vec<&str>, String> {
        match &self.kind {
            None => Ok(Vec::new()),
            Some(Value::String(_)) => Ok(Vec::new()),
            Some(Value::Array(labels))
                if !labels.is_empty() && labels.iter().all(Value::is_string) =>
            {
                Ok(labels
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|label| !label.is_empty())
                    .collect())
            }
            _ => Err("Yield type must be a string or non-empty list of strings.".into()),
        }
    }

    fn incremental(&self) -> bool {
        self.kind
            .as_ref()
            .and_then(Value::as_array)
            .is_some_and(|labels| !labels.is_empty())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StructuredResult {
    pub data: Value,
    pub schema_overridden: bool,
    pub error: Option<String>,
}

#[derive(Default)]
pub(crate) struct YieldAssembly {
    epoch: u64,
    entries: Vec<(u64, YieldInput)>,
    schema_failures: usize,
}

impl YieldAssembly {
    pub(crate) fn require_updated_submission(&mut self) {
        self.epoch = self.epoch.saturating_add(1);
    }

    pub(crate) fn has_section(&self, label: &str) -> bool {
        self.entries.iter().any(|(_, entry)| {
            entry
                .kind
                .as_ref()
                .and_then(Value::as_array)
                .is_some_and(|labels| labels.iter().any(|value| value.as_str() == Some(label)))
        })
    }

    pub(crate) fn submit_workpool(
        &mut self,
        key: usize,
        data: Value,
        items: &[String],
        policy: &CompletionPolicy,
    ) -> Result<bool, String> {
        let id = key
            .checked_sub(1)
            .and_then(|index| items.get(index))
            .ok_or("Unknown workpool item key")?;
        if self.has_section(id) {
            return Err(format!("Workpool item {key} was already submitted"));
        }
        self.submit(
            YieldInput {
                kind: Some(serde_json::json!([id])),
                data: Some(data),
                use_last_turn: false,
                schema_overridden: false,
            },
            policy,
            "",
        )?;
        if !items.iter().all(|id| self.has_section(id)) {
            return Ok(false);
        }
        self.submit(
            YieldInput {
                kind: Some(serde_json::json!("result")),
                data: None,
                use_last_turn: false,
                schema_overridden: false,
            },
            policy,
            "",
        )
    }

    pub(crate) fn submit(
        &mut self,
        mut input: YieldInput,
        policy: &CompletionPolicy,
        last_text: &str,
    ) -> Result<bool, String> {
        input.data = input.data.filter(|data| !data.is_null());
        let labels = input.labels()?;
        let terminal = !input.incremental();
        let CompletionPolicy::Yield { schema, .. } = policy else {
            return Err("This worker has no yield contract.".into());
        };
        let schema = schema.as_ref().map(normalize_schema).transpose()?;
        if !terminal {
            if let Some(schema) = &schema {
                for label in &labels {
                    if !known_section(schema, schema, label, 0) {
                        return Err(format!("Unknown incremental yield section `{label}`."));
                    }
                }
            }
        }
        let has_sections = self.entries.iter().any(|(_, entry)| entry.incremental());
        if input.data.is_none() && input.kind.is_none() && !input.use_last_turn {
            return Err("Yield must contain data or a typed last-turn result.".into());
        }
        let use_last_turn = input.use_last_turn || (input.data.is_none() && input.kind.is_some());
        if use_last_turn && (!terminal || !has_sections) && last_text.trim().is_empty() {
            return Err("Yield used the last assistant turn, but that turn contains no text. Put the result in data.".into());
        }
        if use_last_turn
            && terminal
            && !has_sections
            && schema.as_ref().is_some_and(|schema| schema.is_object())
        {
            return Err("A schema-bound task requires data matching its schema instead of a raw last-turn result.".into());
        }
        if !use_last_turn {
            if let (Some(schema), Some(data)) = (&schema, &input.data) {
                let section = if labels.len() == 1 {
                    section_schema(schema, labels[0])
                } else {
                    None
                };
                let normalize = |value: &Value| {
                    if terminal {
                        normalize_output(value, schema, schema, 0)
                    } else {
                        section.map_or_else(
                            || value.clone(),
                            |section| normalize_output(value, section, schema, 0),
                        )
                    }
                };
                let validate = |value: &Value| {
                    if terminal {
                        validate_json_result(schema, value)
                    } else {
                        labels.iter().try_for_each(|label| {
                            section_schema(schema, label).map_or(Ok(()), |section| {
                                validate_subschema(section, schema, value)
                            })
                        })
                    }
                };
                let mut data = normalize(data);
                let mut validation = validate(&data);
                if validation.is_err() {
                    if let Some(parsed) = data
                        .as_str()
                        .and_then(|text| serde_json::from_str::<Value>(text).ok())
                        .filter(|value| !value.is_null())
                    {
                        let parsed = normalize(&parsed);
                        if validate(&parsed).is_ok() {
                            data = parsed;
                            validation = Ok(());
                        }
                    }
                }
                input.data = Some(data);
                if let Err(error) = validation {
                    self.schema_failures += 1;
                    if self.schema_failures <= 3 {
                        return Err(format!("Yield does not match schema: {error}. Retry with corrected data. Schema retries remaining before override: {}.", 3 - self.schema_failures));
                    }
                    self.schema_failures = 0;
                    input.schema_overridden = true;
                } else {
                    self.schema_failures = 0;
                }
            }
        }
        self.entries.push((self.epoch, input));
        Ok(terminal)
    }

    pub(crate) fn has_current_terminal(&self) -> bool {
        self.entries
            .iter()
            .rev()
            .any(|(epoch, entry)| *epoch == self.epoch && !entry.incremental())
    }

    pub(crate) fn finish(
        &self,
        policy: &CompletionPolicy,
        last_text: &str,
    ) -> Result<StructuredResult, String> {
        let CompletionPolicy::Yield { schema, strict } = policy else {
            return Err("This worker has no yield contract.".into());
        };
        if self.entries.is_empty() {
            return Err("Worker did not submit a yield result.".into());
        }
        if self.entries.iter().all(|(epoch, _)| *epoch != self.epoch) {
            return Err(
                "Yield result predates newer child results. Submit an updated result.".into(),
            );
        }
        let schema = schema.as_ref().map(normalize_schema).transpose()?;
        let terminal = self
            .entries
            .iter()
            .rev()
            .find(|(epoch, item)| *epoch == self.epoch && !item.incremental());
        let mut sections = Map::new();
        let shapes = schema.as_ref().map(section_shapes).unwrap_or_default();
        let mut overridden = false;
        let mut scalar_overrides = BTreeMap::new();
        let mut missing_data = false;
        for (_, entry) in &self.entries {
            if !entry.incremental() {
                continue;
            }
            let value = if entry.use_last_turn || entry.data.is_none() {
                Value::String(last_text.to_string())
            } else {
                entry.data.clone().unwrap_or(Value::Null)
            };
            missing_data |= value.is_null()
                || ((entry.use_last_turn || entry.data.is_none()) && last_text.is_empty());
            for label in entry.labels()? {
                let value = value.clone();
                match shapes.get(label).copied().flatten() {
                    Some(true) => {
                        sections
                            .entry(label)
                            .or_insert_with(|| Value::Array(Vec::new()))
                            .as_array_mut()
                            .expect("array section")
                            .push(value);
                        overridden |= entry.schema_overridden;
                    }
                    Some(false) => {
                        sections.insert(label.to_string(), value);
                        scalar_overrides.insert(label.to_string(), entry.schema_overridden);
                    }
                    None => {
                        if let Some(current) = sections.get_mut(label) {
                            if let Some(items) = current.as_array_mut() {
                                items.push(value);
                            } else {
                                *current = Value::Array(vec![current.clone(), value]);
                            }
                        } else {
                            sections.insert(label.to_string(), value);
                        }
                        overridden |= entry.schema_overridden;
                    }
                }
            }
        }
        let mut raw_text = false;
        let mut data = if let Some((_, entry)) = terminal.filter(|(_, entry)| entry.data.is_some())
        {
            overridden = entry.schema_overridden;
            missing_data = entry.data.as_ref().is_none_or(Value::is_null);
            raw_text = entry.use_last_turn;
            if entry.use_last_turn {
                Value::String(last_text.to_string())
            } else {
                entry.data.clone().unwrap_or(Value::Null)
            }
        } else if !sections.is_empty() {
            overridden |= scalar_overrides.values().any(|value| *value);
            Value::Object(sections)
        } else if terminal.is_some_and(|(_, item)| item.use_last_turn || item.kind.is_some()) {
            raw_text = true;
            missing_data = last_text.is_empty();
            Value::String(last_text.to_string())
        } else {
            missing_data = true;
            Value::Null
        };
        if !raw_text {
            if let Some(parsed) = data
                .as_str()
                .map(str::trim)
                .filter(|text| text.starts_with(['{', '[']))
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
            {
                data = parsed;
            }
        }
        if *strict {
            if let Some(schema) = &schema {
                data = normalize_output(&data, schema, schema, 0);
            }
        }
        let error = if missing_data {
            Some("Yield result has no data.".into())
        } else {
            schema
                .as_ref()
                .and_then(|schema| validate_json_result(schema, &data).err())
                .or_else(|| {
                    overridden.then(|| {
                        "Yield schema validation was overridden after repeated failures.".into()
                    })
                })
        };
        if let Some(error) = &error {
            if *strict || (!overridden && !missing_data) {
                return Err(error.clone());
            }
        }
        Ok(StructuredResult {
            data,
            schema_overridden: overridden,
            error,
        })
    }
}

fn section_schema<'a>(schema: &'a Value, label: &str) -> Option<&'a Value> {
    let property = schema
        .get("properties")
        .and_then(|properties| properties.get(label))
        .or_else(|| resolve_schema(schema, schema).get("properties")?.get(label))?;
    let property = resolve_schema(property, schema);
    if property.get("type").and_then(Value::as_str) == Some("array") {
        property
            .get("items")
            .filter(|schema| !schema.is_null())
            .or(Some(property))
    } else {
        Some(property)
    }
}

fn closed_conjuncts<'a>(schema: &'a Value, root: &'a Value, depth: usize) -> Vec<&'a Value> {
    if depth > 64 {
        return Vec::new();
    }
    let mut closed = Vec::new();
    if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
        closed.push(schema);
    }
    if let Some(target) = referenced_schema(schema, root) {
        closed.extend(closed_conjuncts(target, root, depth + 1));
    }
    if let Some(branches) = schema.get("allOf").and_then(Value::as_array) {
        for branch in branches {
            closed.extend(closed_conjuncts(branch, root, depth + 1));
        }
    }
    closed
}

fn accepts_label(schema: &Value, label: &str) -> bool {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|properties| properties.contains_key(label))
        || schema
            .get("patternProperties")
            .and_then(Value::as_object)
            .is_some_and(|patterns| {
                patterns.keys().any(|pattern| {
                    validate_json_result(&json!({"pattern":pattern}), &json!(label)).is_ok()
                })
            })
        || schema.get("additionalProperties") != Some(&Value::Bool(false))
}

fn known_section(schema: &Value, root: &Value, label: &str, depth: usize) -> bool {
    if depth > 64 {
        return true;
    }
    if referenced_schema(schema, root)
        .is_some_and(|target| !known_section(target, root, label, depth + 1))
    {
        return false;
    }
    if schema.get("additionalProperties") == Some(&Value::Bool(false))
        && !accepts_label(schema, label)
    {
        return false;
    }
    for keyword in ["oneOf", "anyOf"] {
        if let Some(branches) = schema
            .get(keyword)
            .and_then(Value::as_array)
            .filter(|branches| !branches.is_empty())
        {
            let variants: Vec<_> = branches
                .iter()
                .map(|branch| closed_conjuncts(branch, root, depth + 1))
                .collect();
            if variants.iter().all(|variant| !variant.is_empty())
                && !variants
                    .iter()
                    .any(|variant| variant.iter().all(|schema| accepts_label(schema, label)))
            {
                return false;
            }
        }
    }
    schema
        .get("allOf")
        .and_then(Value::as_array)
        .is_none_or(|branches| {
            branches
                .iter()
                .all(|branch| known_section(branch, root, label, depth + 1))
        })
}

fn section_shapes(schema: &Value) -> BTreeMap<String, Option<bool>> {
    fn array_typed(schema: &Value, root: &Value, depth: usize) -> bool {
        if depth > 64 {
            return false;
        }
        schema.get("type").is_some_and(|kind| {
            kind == "array"
                || kind
                    .as_array()
                    .is_some_and(|items| items.iter().any(|kind| kind == "array"))
        }) || referenced_schema(schema, root)
            .is_some_and(|target| array_typed(target, root, depth + 1))
            || ["allOf", "anyOf", "oneOf"].iter().any(|keyword| {
                schema
                    .get(keyword)
                    .and_then(Value::as_array)
                    .is_some_and(|branches| {
                        branches
                            .iter()
                            .any(|branch| array_typed(branch, root, depth + 1))
                    })
            })
    }
    fn collect(
        schema: &Value,
        root: &Value,
        result: &mut BTreeMap<String, Option<bool>>,
        depth: usize,
    ) {
        if depth > 64 {
            return;
        }
        if let Some(target) = referenced_schema(schema, root) {
            collect(target, root, result, depth + 1);
        }
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            for (label, shape) in properties {
                let array = array_typed(shape, root, depth + 1);
                result
                    .entry(label.clone())
                    .and_modify(|existing| {
                        if *existing != Some(array) {
                            *existing = None;
                        }
                    })
                    .or_insert(Some(array));
            }
        }
        for keyword in ["allOf", "anyOf", "oneOf"] {
            if let Some(branches) = schema.get(keyword).and_then(Value::as_array) {
                for branch in branches {
                    collect(branch, root, result, depth + 1);
                }
            }
        }
    }
    let mut shapes = BTreeMap::new();
    collect(schema, schema, &mut shapes, 0);
    shapes
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn input(value: Value) -> YieldInput {
        serde_json::from_value(value).unwrap()
    }
    fn policy(schema: Value, strict: bool) -> CompletionPolicy {
        CompletionPolicy::Yield {
            schema: Some(schema),
            strict,
        }
    }

    #[test]
    fn validates_constraints_refs_and_invalid_schemas() {
        let schema = json!({"type":"object","required":["n"],"properties":{"n":{"$ref":"#/$defs/n"}},"$defs":{"n":{"type":"integer","minimum":2}},"additionalProperties":false});
        assert!(validate_schema(&schema).is_ok());
        assert!(validate_result(&schema, &json!({"n":2})).is_ok());
        assert!(validate_result(&schema, &json!({"n":1})).is_err());
        assert!(validate_result(&schema, &json!({"n":2,"extra":1})).is_err());
        assert!(validate_schema(&json!({"type":"unknown"})).is_err());
        assert!(validate_schema(&json!({"$ref":"https://example.org/schema"})).is_err());
        assert!(validate_schema(&json!({"properties":{"$ref":{"type":"string"}},"const":{"$ref":"https://example.org/data"}})).is_ok());
    }

    #[test]
    fn incremental_arrays_accumulate_scalars_replace_terminal_data_wins() {
        let policy = policy(
            json!({"type":"object","properties":{"items":{"type":"array","items":{}},"summary":{"type":"string"}}}),
            true,
        );
        let mut assembly = YieldAssembly::default();
        for entry in [
            json!({"type":["items"],"data":1}),
            json!({"type":["summary"],"data":"old"}),
            json!({"type":["summary"],"data":"new"}),
            json!({"type":"result"}),
        ] {
            assembly.submit(input(entry), &policy, "").unwrap();
        }
        assert_eq!(
            assembly.finish(&policy, "").unwrap().data,
            json!({"items":[1],"summary":"new"})
        );
        assembly
            .submit(
                input(json!({"type":"result","data":{"summary":"final"}})),
                &policy,
                "",
            )
            .unwrap();
        assert_eq!(
            assembly.finish(&policy, "").unwrap().data,
            json!({"summary":"final"})
        );
    }

    #[test]
    fn new_child_results_require_updated_submission_and_errors_are_observable() {
        let mut assembly = YieldAssembly::default();
        let strict = policy(json!({"type":"integer"}), true);
        assert!(assembly
            .submit(input(json!({"data":"bad"})), &strict, "")
            .is_err());
        assert!(!assembly.has_current_terminal());
        let permissive = policy(json!({"type":"integer"}), false);
        for _ in 0..2 {
            assert!(assembly
                .submit(input(json!({"data":"bad"})), &permissive, "")
                .is_err());
        }
        assembly
            .submit(input(json!({"data":"bad"})), &permissive, "")
            .unwrap();
        assert!(assembly.finish(&permissive, "").unwrap().error.is_some());
        assembly.require_updated_submission();
        assert!(!assembly.has_current_terminal());
        assert!(assembly.finish(&permissive, "").is_err());
        assembly
            .submit(input(json!({"data":2})), &strict, "")
            .unwrap();
        assert_eq!(assembly.finish(&strict, "").unwrap().data, json!(2));
    }

    #[test]
    fn incoming_advice_or_messages_require_a_new_terminal_submission() {
        let policy = policy(json!({"type":"string"}), true);
        let mut assembly = YieldAssembly::default();
        assembly
            .submit(input(json!({"data":"before advice"})), &policy, "")
            .unwrap();
        assert!(assembly.has_current_terminal());
        assembly.require_updated_submission();
        assert!(!assembly.has_current_terminal());
        assert!(assembly.finish(&policy, "").is_err());
        assembly
            .submit(input(json!({"data":"after advice"})), &policy, "")
            .unwrap();
        assert!(assembly.has_current_terminal());
        assert_eq!(
            assembly.finish(&policy, "").unwrap().data,
            json!("after advice")
        );
    }

    #[test]
    fn workpool_yields_preserve_batch_order_reject_duplicates_and_finish_automatically() {
        let items = vec!["pool#2".to_string(), "pool#10".to_string()];
        let policy = policy(
            json!({"type":"object","properties":{"pool#2":{},"pool#10":{}},"required":items,"additionalProperties":false}),
            true,
        );
        let mut assembly = YieldAssembly::default();
        assert!(assembly
            .submit_workpool(0, json!(1), &items, &policy)
            .is_err());
        assert!(!assembly
            .submit_workpool(2, json!({"value":42}), &items, &policy)
            .unwrap());
        assert!(assembly.has_section("pool#10"));
        assert!(!assembly.has_current_terminal());
        assert!(assembly
            .submit_workpool(2, json!("duplicate"), &items, &policy)
            .is_err());
        assert!(assembly
            .submit_workpool(1, json!({"error":"invalid input"}), &items, &policy)
            .unwrap());
        assert!(assembly.has_current_terminal());
        assert_eq!(
            assembly.finish(&policy, "").unwrap().data,
            json!({"pool#2":{"error":"invalid input"},"pool#10":{"value":42}})
        );
    }

    #[test]
    fn jtd_and_json_string_declarations_enforce_their_constraints() {
        let schema = json!({"properties":{"items":{"elements":{"type":"int32"}}},"optionalProperties":{"comment":{"type":"string","nullable":true}}});
        assert!(validate_result(&schema, &json!({"items":[1],"comment":null})).is_ok());
        assert!(validate_result(&schema, &json!({"items":["wrong"]})).is_err());
        assert!(validate_result(&schema, &json!({"comment":"missing items"})).is_err());
        assert!(validate_result(&json!(schema.to_string()), &json!({"items":["wrong"]})).is_err());
    }

    #[test]
    fn omp_numeric_shorthand_preserves_fractional_values_and_json_schema_constraints() {
        let schema = json!({"properties":{"confidence":{"type":"number"}},"optionalProperties":{"scores":{"elements":{"type":"number"}}}});
        assert!(validate_result(&schema, &json!({"confidence":0.75,"scores":[1.25]})).is_ok());
        for invalid in [
            json!({}),
            json!({"confidence":"0.75"}),
            json!({"confidence":0.75,"scores":["wrong"]}),
            json!({"confidence":0.75,"extra":1}),
        ] {
            assert!(validate_result(&schema, &invalid).is_err());
        }
        let json_schema = json!({"properties":{"score":{"type":"number","minimum":0.5}}});
        assert!(validate_result(&json_schema, &json!({})).is_ok());
        assert!(validate_result(&json_schema, &json!({"score":0.25})).is_err());
        assert!(validate_result(&json_schema, &json!({"score":0.75})).is_ok());
        assert!(validate_result(
            &json!({"properties":{"score":{"type":"number"}}}),
            &json!({})
        )
        .is_ok());
        for invalid in [
            json!({"elements":{"type":"unknown"}}),
            json!({"elements":{"type":"number","elements":{}}}),
            json!({"properties":{"score":{"type":"number"}},"optionalProperties":{"score":{"type":"number"}}}),
            json!({"optionalProperties":{"score":{"type":"number","minimum":0.5}}}),
        ] {
            assert!(validate_schema(&invalid).is_err());
        }
    }

    #[test]
    fn strict_provider_nulls_are_removed_from_optional_fields() {
        let schema = json!({"type":"object","properties":{"items":{"type":"array","items":{"type":"object","properties":{"name":{"type":"string"},"note":{"type":"string"}},"required":["name"],"additionalProperties":false}}},"required":["items"]});
        let mut assembly = YieldAssembly::default();
        assembly
            .submit(
                input(json!({"data":{"items":[{"name":"ok","note":null}]}})),
                &policy(schema, true),
                "",
            )
            .unwrap();
    }

    #[test]
    fn closed_schemas_reject_unknown_sections_before_assembly() {
        let mut assembly = YieldAssembly::default();
        let policy = policy(
            json!({"type":"object","properties":{"known":{"type":"string"}},"additionalProperties":false}),
            false,
        );
        assert!(assembly
            .submit(input(json!({"type":["unknown"],"data":1})), &policy, "")
            .is_err());
    }

    #[test]
    fn permissive_does_not_silently_accept_an_ordinary_schema_failure() {
        let mut assembly = YieldAssembly::default();
        assert!(assembly
            .submit(
                input(json!({"data":"bad"})),
                &policy(json!({"type":"integer"}), false),
                ""
            )
            .is_err());
    }

    #[test]
    fn converts_jtd_forms_local_definitions_and_nested_json_schema_nodes() {
        let schema = json!({"definitions":{"Label/Name":{"enum":["yes","no"]}},"properties":{"labels":{"values":{"ref":"Label/Name"}},"measure":{"type":"float64"},"time":{"type":"timestamp"}}});
        assert!(validate_result(
            &schema,
            &json!({"labels":{"a":"yes"},"measure":1.5,"time":"text"})
        )
        .is_ok());
        assert!(validate_result(
            &schema,
            &json!({"labels":{"a":"maybe"},"measure":1.5,"time":"text"})
        )
        .is_err());
        assert!(
            validate_result(&schema, &json!({"labels":{},"measure":"bad","time":"text"})).is_err()
        );
        let nested = json!({"type":"object","properties":{"items":{"elements":{"type":"int8"}},"required":{"const":{"$ref":"https://example.org/data"}}},"required":["items"]});
        assert!(validate_result(
            &nested,
            &json!({"items":[1],"required":{"$ref":"https://example.org/data"}})
        )
        .is_ok());
        assert!(validate_result(&nested, &json!({"items":["bad"]})).is_err());
        assert!(validate_schema(&json!({"elements":4})).is_err());
        assert!(
            validate_schema(&json!({"type":"object","properties":{"invalid":{"elements":4}}}))
                .is_err()
        );
        assert!(validate_schema(&json!({"elements":{},"type":"string"})).is_err());
        assert!(validate_schema(&json!("{invalid}")).is_err());
    }

    #[test]
    fn json_schema_ambiguity_keeps_constraints_and_annotation_data() {
        let schema = json!({"properties":{"name":{"type":"string","minLength":3}},"required":["name"],"metadata":{"$ref":"https://example.org/data"}});
        assert!(validate_result(&schema, &json!({"name":"long"})).is_ok());
        assert!(validate_result(&schema, &json!({"name":"x"})).is_err());
        for keyword in [
            "properties",
            "patternProperties",
            "$defs",
            "definitions",
            "dependentSchemas",
        ] {
            assert!(validate_schema(
                &json!({keyword:{"external":{"$ref":"https://example.org/schema"}}})
            )
            .is_err());
        }
        for keyword in [
            "not",
            "items",
            "additionalProperties",
            "if",
            "then",
            "else",
            "contains",
            "unevaluatedItems",
        ] {
            assert!(
                validate_schema(&json!({keyword:{"$ref":"https://example.org/schema"}})).is_err()
            );
        }
        assert!(validate_schema(&json!({"const":{"$ref":"https://example.org/data"},"examples":[{"$dynamicRef":"remote"}]})).is_ok());
    }

    #[test]
    fn jtd_discriminator_and_json_unions_gate_only_fully_closed_variants() {
        let schema = json!({"discriminator":"kind","mapping":{"a":{"properties":{"first":{"type":"string"}}},"b":{"properties":{"second":{"elements":{"type":"string"}}}}}});
        assert!(validate_result(&schema, &json!({"kind":"a","first":"yes"})).is_ok());
        assert!(validate_result(&schema, &json!({"kind":"a","second":[]})).is_err());
        let normalized = normalize_schema(&schema).unwrap();
        assert!(known_section(&normalized, &normalized, "first", 0));
        assert!(known_section(&normalized, &normalized, "second", 0));
        assert!(!known_section(&normalized, &normalized, "unknown", 0));
        assert_eq!(section_shapes(&normalized).get("second"), Some(&Some(true)));
        let open = json!({"anyOf":[{"properties":{"known":{}},"additionalProperties":false},{"type":"object"}]});
        assert!(known_section(&open, &open, "unknown", 0));
        let intersection = json!({"allOf":[{"properties":{"a":{},"both":{}},"additionalProperties":false},{"properties":{"b":{},"both":{}},"additionalProperties":false}]});
        assert!(known_section(&intersection, &intersection, "both", 0));
        assert!(!known_section(&intersection, &intersection, "a", 0));
        let pattern =
            json!({"patternProperties":{"^entry_(?=ok)":{}},"additionalProperties":false});
        assert!(known_section(&pattern, &pattern, "entry_ok", 0));
        assert!(!known_section(&pattern, &pattern, "entry_bad", 0));
    }

    #[test]
    fn normalizes_optional_nulls_with_local_refs_and_unions_without_erasing_valid_nulls() {
        let schema = json!({"$ref":"#/$defs/root","$defs":{"root":{"type":"object","properties":{"required":{"type":"string"},"nullable":{"type":["null","string"]},"optional":{"$ref":"#/$defs/name"},"nested":{"anyOf":[{"type":"object","properties":{"a":{"type":"string"}},"additionalProperties":false},{"type":"object","properties":{"b":{"type":"integer"}},"required":["b"],"additionalProperties":false}]}},"required":["required"],"additionalProperties":false},"name":{"type":"string"}}});
        let policy = policy(schema.clone(), true);
        let mut assembly = YieldAssembly::default();
        assembly.submit(input(json!({"data":{"required":"yes","nullable":null,"optional":null,"nested":{"a":null}}})), &policy, "").unwrap();
        assert_eq!(
            assembly.finish(&policy, "").unwrap().data,
            json!({"required":"yes","nullable":null,"nested":{}})
        );
        assert!(assembly
            .submit(input(json!({"data":{"required":null}})), &policy, "")
            .is_err());
        assert!(validate_result(&json!({"type":"null"}), &Value::Null).is_ok());
        assert!(assembly
            .submit(input(json!({"data":null})), &policy, "")
            .is_err());
    }

    #[test]
    fn incremental_sections_validate_recover_json_strings_and_normalize_array_items() {
        let schema = json!({"$ref":"#/$defs/root","$defs":{"root":{"type":"object","properties":{"items":{"type":"array","items":{"$ref":"#/$defs/item"}},"count":{"type":"integer"}},"required":["items","count"],"additionalProperties":false},"item":{"type":"object","properties":{"name":{"type":"string"},"note":{"type":"string"}},"required":["name"],"additionalProperties":false}}});
        let policy = policy(schema, true);
        let mut assembly = YieldAssembly::default();
        assert!(assembly
            .submit(input(json!({"type":["unknown"],"data":{}})), &policy, "")
            .is_err());
        assembly
            .submit(
                input(json!({"type":["items"],"data":"{\"name\":\"yes\",\"note\":null}"})),
                &policy,
                "",
            )
            .unwrap();
        assembly
            .submit(input(json!({"type":["count"],"data":"2"})), &policy, "")
            .unwrap();
        assembly
            .submit(input(json!({"type":"result"})), &policy, "")
            .unwrap();
        assert_eq!(
            assembly.finish(&policy, "").unwrap().data,
            json!({"items":[{"name":"yes"}],"count":2})
        );
    }

    #[test]
    fn local_anchors_preserve_section_validation_and_null_normalization() {
        let schema = json!({"$defs":{"closed":{"$anchor":"closed","type":"object","properties":{"value":{"$ref":"#text"}},"additionalProperties":false},"text":{"$anchor":"text","type":"string"}},"$ref":"#closed"});
        let policy = policy(schema, true);
        let mut assembly = YieldAssembly::default();
        assert!(assembly
            .submit(input(json!({"type":["unknown"],"data":1})), &policy, "")
            .is_err());
        assembly
            .submit(input(json!({"data":{"value":null}})), &policy, "")
            .unwrap();
        assert_eq!(assembly.finish(&policy, "").unwrap().data, json!({}));
    }

    #[test]
    fn reference_siblings_keep_their_constraints_and_section_declarations() {
        let schema = json!({"$ref":"#/$defs/root","$defs":{"root":{"type":"object"}},"properties":{"value":{"type":"string"}},"additionalProperties":false});
        let policy = policy(schema, true);
        let mut assembly = YieldAssembly::default();
        assert!(assembly
            .submit(input(json!({"type":["unknown"],"data":1})), &policy, "")
            .is_err());
        assembly
            .submit(input(json!({"data":{"value":null}})), &policy, "")
            .unwrap();
        assert_eq!(assembly.finish(&policy, "").unwrap().data, json!({}));
    }

    #[test]
    fn empty_references_stay_within_the_document_and_recursive_values_normalize() {
        let schema = json!({"type":"object","properties":{"next":{"$ref":""},"name":{"type":"string"}},"additionalProperties":false});
        assert!(validate_schema(&schema).is_ok());
        let policy = policy(schema, true);
        let mut assembly = YieldAssembly::default();
        assembly
            .submit(input(json!({"data":{"next":{"name":null}}})), &policy, "")
            .unwrap();
        assert_eq!(
            assembly.finish(&policy, "").unwrap().data,
            json!({"next":{}})
        );
    }

    #[test]
    fn retries_reset_and_override_is_internal_with_strict_finalization() {
        let permissive = policy(json!({"type":"integer"}), false);
        let strict = policy(json!({"type":"integer"}), true);
        let mut assembly = YieldAssembly::default();
        assert!(serde_json::from_value::<YieldInput>(
            json!({"data":"bad","schemaOverridden":true})
        )
        .is_err());
        for _ in 0..3 {
            assert!(assembly
                .submit(input(json!({"data":"bad"})), &permissive, "")
                .is_err());
        }
        assembly
            .submit(input(json!({"data":1})), &permissive, "")
            .unwrap();
        for _ in 0..3 {
            assert!(assembly
                .submit(input(json!({"data":"bad"})), &permissive, "")
                .is_err());
        }
        assembly
            .submit(input(json!({"data":"bad"})), &permissive, "")
            .unwrap();
        let overridden = assembly.finish(&permissive, "").unwrap();
        assert!(overridden.schema_overridden && overridden.error.is_some());
        assert!(assembly.finish(&strict, "").is_err());
        assembly
            .submit(input(json!({"data":2})), &permissive, "")
            .unwrap();
        let corrected = assembly.finish(&permissive, "").unwrap();
        assert!(!corrected.schema_overridden && corrected.error.is_none());
    }

    #[test]
    fn scalar_replacement_clears_override_but_array_provenance_accumulates() {
        let policy = policy(
            json!({"type":"object","properties":{"value":{"type":"string"},"items":{"type":"array","items":{"type":"integer"}}}}),
            false,
        );
        let mut assembly = YieldAssembly::default();
        for _ in 0..3 {
            assert!(assembly
                .submit(input(json!({"type":["value"],"data":1})), &policy, "")
                .is_err());
        }
        assembly
            .submit(input(json!({"type":["value"],"data":1})), &policy, "")
            .unwrap();
        assembly
            .submit(input(json!({"type":["value"],"data":"fixed"})), &policy, "")
            .unwrap();
        assert!(!assembly.finish(&policy, "").unwrap().schema_overridden);
        for _ in 0..3 {
            assert!(assembly
                .submit(input(json!({"type":["items"],"data":"bad"})), &policy, "")
                .is_err());
        }
        assembly
            .submit(input(json!({"type":["items"],"data":"bad"})), &policy, "")
            .unwrap();
        assembly
            .submit(input(json!({"type":["items"],"data":1})), &policy, "")
            .unwrap();
        assert!(assembly.finish(&policy, "").unwrap().schema_overridden);
    }

    #[test]
    fn undeclared_array_values_append_and_mixed_sections_keep_the_open_merge() {
        let policy = policy(json!({}), false);
        let mut assembly = YieldAssembly::default();
        assembly
            .submit(input(json!({"type":["x"],"data":[1,2]})), &policy, "")
            .unwrap();
        assembly
            .submit(input(json!({"type":["x"],"data":3})), &policy, "")
            .unwrap();
        assert_eq!(
            assembly.finish(&policy, "").unwrap().data,
            json!({"x":[1,2,3]})
        );
        let schema = json!({"anyOf":[{"properties":{"mixed":{"type":"array"}}},{"properties":{"mixed":{"type":"string"}}}]});
        assert_eq!(section_shapes(&schema).get("mixed"), Some(&None));
        let schema =
            json!({"properties":{"nullable":{"anyOf":[{"type":"null"},{"type":"array"}]}}});
        assert_eq!(section_shapes(&schema).get("nullable"), Some(&Some(true)));
    }

    #[test]
    fn scalar_json_strings_are_recovered_only_when_the_raw_schema_rejects_them() {
        let string = policy(json!({"type":"string"}), true);
        for text in ["42", "null", "false", "\"quoted\""] {
            let mut assembly = YieldAssembly::default();
            assembly
                .submit(input(json!({"data":text})), &string, "")
                .unwrap();
            assert_eq!(assembly.finish(&string, "").unwrap().data, json!(text));
        }
        let boolean = policy(json!({"type":"boolean"}), true);
        let mut assembly = YieldAssembly::default();
        assembly
            .submit(input(json!({"data":"false"})), &boolean, "")
            .unwrap();
        assert_eq!(assembly.finish(&boolean, "").unwrap().data, json!(false));
        let null = policy(json!({"type":"null"}), true);
        let mut assembly = YieldAssembly::default();
        assert!(assembly
            .submit(input(json!({"data":"null"})), &null, "")
            .is_err());
    }
}
