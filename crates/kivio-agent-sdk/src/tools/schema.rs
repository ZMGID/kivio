use jsonschema::{
    error::{TypeKind, ValidationErrorKind},
    paths::LocationSegment,
    Retrieve, Uri,
};
use serde_json::Value;

struct NoExternalSchemas;

impl Retrieve for NoExternalSchemas {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err(format!("external schema references are not allowed: {uri}").into())
    }
}

pub fn validate_arguments(schema: &Value, value: &Value) -> Result<(), String> {
    if schema.is_null() {
        return Ok(());
    }
    let validator = jsonschema::options()
        .with_retriever(NoExternalSchemas)
        .build(schema)
        .map_err(|error| format!("invalid arguments schema: {error}"))?;
    validator.validate(value).map_err(|error| {
        let mut path = String::from("arguments");
        for segment in error.instance_path() {
            match segment {
                LocationSegment::Property(key) => {
                    path.push('.');
                    path.push_str(&key);
                }
                LocationSegment::Index(index) => path.push_str(&format!("[{index}]")),
            }
        }
        match error.kind() {
            ValidationErrorKind::Required { property } => format!(
                "{path}.{} is required",
                property.as_str().unwrap_or_default()
            ),
            ValidationErrorKind::AdditionalProperties { unexpected } => format!(
                "{path}.{} is not allowed",
                unexpected.first().map(String::as_str).unwrap_or_default()
            ),
            ValidationErrorKind::Type {
                kind: TypeKind::Single(kind),
            } => format!("{path} must be {kind}"),
            ValidationErrorKind::Type { .. } => format!("{path} has an invalid type"),
            ValidationErrorKind::Enum { .. } => {
                format!("{path} must be one of the declared enum values")
            }
            ValidationErrorKind::AnyOf { .. } => {
                format!("{path} does not match any allowed schema")
            }
            ValidationErrorKind::OneOfMultipleValid { .. }
            | ValidationErrorKind::OneOfNotValid { .. } => {
                format!("{path} must match exactly one allowed schema")
            }
            ValidationErrorKind::Minimum { limit } => format!("{path} must be at least {limit}"),
            ValidationErrorKind::Maximum { limit } => format!("{path} must be at most {limit}"),
            ValidationErrorKind::MaxItems { limit } => {
                format!("{path} must contain at most {limit} items")
            }
            ValidationErrorKind::FalseSchema
                if error
                    .schema_path()
                    .as_str()
                    .ends_with("/additionalProperties") =>
            {
                let key = value
                    .pointer(error.instance_path().as_str())
                    .and_then(Value::as_object)
                    .and_then(|object| object.keys().next());
                match key {
                    Some(key) => format!("{path}.{key} is not allowed"),
                    None => format!("{path}: {error}"),
                }
            }
            _ => format!("{path}: {error}"),
        }
    })
}

pub fn coerce_arguments(schema: &Value, value: Value) -> Value {
    if schema.is_null() {
        return value;
    }
    let Ok(validators) = jsonschema::options()
        .with_retriever(NoExternalSchemas)
        .build_map(schema)
    else {
        return value;
    };
    Coercion { schema, validators }.coerce("#", value, &mut Vec::new())
}

struct Coercion<'a> {
    schema: &'a Value,
    validators: jsonschema::ValidatorMap,
}

impl Coercion<'_> {
    fn is_valid(&self, path: &str, value: &Value) -> bool {
        self.validators
            .get(path)
            .is_some_and(|validator| validator.is_valid(value))
    }

    fn coerce(&self, path: &str, mut value: Value, active: &mut Vec<String>) -> Value {
        if active.iter().any(|seen| seen == path) {
            return value;
        }
        let mut scope = path.strip_prefix('#').unwrap_or(path);
        while !scope.is_empty() {
            if self
                .schema
                .pointer(scope)
                .is_some_and(|node| node.get("$id").is_some())
            {
                return value;
            }
            scope = scope
                .rsplit_once('/')
                .map(|(parent, _)| parent)
                .unwrap_or_default();
        }
        let Some(schema) = self.schema.pointer(path.strip_prefix('#').unwrap_or(path)) else {
            return value;
        };
        active.push(path.to_owned());
        // shortcut: Coercion follows local pointers; extend it when tools use scoped or anchor refs.
        if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
            if reference.starts_with('#') {
                value = self.coerce(reference, value, active);
            }
        }
        if let Some(all_of) = schema.get("allOf").and_then(Value::as_array) {
            for index in 0..all_of.len() {
                value = self.coerce(&format!("{path}/allOf/{index}"), value, active);
            }
        }
        for keyword in ["anyOf", "oneOf"] {
            if let Some(options) = schema.get(keyword).and_then(Value::as_array) {
                let paths: Vec<_> = (0..options.len())
                    .map(|index| format!("{path}/{keyword}/{index}"))
                    .collect();
                if !paths.iter().any(|option| self.is_valid(option, &value)) {
                    for option in paths {
                        let candidate = self.coerce(&option, value.clone(), active);
                        if self.is_valid(&option, &candidate) {
                            value = candidate;
                            break;
                        }
                    }
                }
            }
        }
        let types = match schema.get("type") {
            Some(Value::String(name)) => vec![name.as_str()],
            Some(Value::Array(names)) => names.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        let matches_union = types.len() > 1 && types.iter().any(|name| matches_type(name, &value));
        if !matches_union {
            for name in &types {
                if let Some(candidate) = coerce_primitive(&value, name) {
                    value = candidate;
                    break;
                }
            }
        }
        if types.contains(&"object") {
            if let Some(object) = value.as_object_mut() {
                let properties = schema.get("properties").and_then(Value::as_object);
                if let Some(properties) = properties {
                    let required = schema.get("required").and_then(Value::as_array);
                    for key in properties.keys() {
                        if let Some(child) = object.remove(key) {
                            let child_path = format!("{path}/properties/{}", escape_pointer(key));
                            let is_required = required.is_some_and(|keys| {
                                keys.iter().any(|item| item.as_str() == Some(key))
                            });
                            if child.is_null()
                                && !is_required
                                && !self.is_valid(&child_path, &child)
                            {
                                continue;
                            }
                            object.insert(
                                key.clone(),
                                self.coerce(&child_path, child, &mut Vec::new()),
                            );
                        }
                    }
                }
                if schema
                    .get("additionalProperties")
                    .is_some_and(Value::is_object)
                {
                    let keys: Vec<_> = object
                        .keys()
                        .filter(|key| properties.is_none_or(|props| !props.contains_key(*key)))
                        .cloned()
                        .collect();
                    for key in keys {
                        if let Some(child) = object.remove(&key) {
                            object.insert(
                                key,
                                self.coerce(
                                    &format!("{path}/additionalProperties"),
                                    child,
                                    &mut Vec::new(),
                                ),
                            );
                        }
                    }
                }
            }
        }
        if types.contains(&"array") && schema.get("items").is_some_and(Value::is_object) {
            if let Some(array) = value.as_array_mut() {
                for item in array {
                    *item = self.coerce(
                        &format!("{path}/items"),
                        std::mem::take(item),
                        &mut Vec::new(),
                    );
                }
            }
        }
        active.pop();
        value
    }
}

fn escape_pointer(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

fn matches_type(name: &str, value: &Value) -> bool {
    match name {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        _ => true,
    }
}

fn coerce_primitive(value: &Value, name: &str) -> Option<Value> {
    match (name, value) {
        ("number", Value::String(text)) if !text.trim().is_empty() => text
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|number| number.is_finite())
            .map(Value::from),
        ("integer", Value::String(text)) if !text.trim().is_empty() => {
            text.trim().parse::<i64>().ok().map(Value::from)
        }
        ("number" | "integer", Value::Bool(flag)) => Some(Value::from(if *flag { 1 } else { 0 })),
        ("boolean", Value::String(text)) if text == "true" => Some(Value::Bool(true)),
        ("boolean", Value::String(text)) if text == "false" => Some(Value::Bool(false)),
        ("boolean", Value::Number(number)) if number.as_i64() == Some(1) => Some(Value::Bool(true)),
        ("boolean", Value::Number(number)) if number.as_i64() == Some(0) => {
            Some(Value::Bool(false))
        }
        ("string", Value::Number(number)) => Some(Value::String(number.to_string())),
        ("string", Value::Bool(flag)) => Some(Value::String(flag.to_string())),
        ("null", Value::String(text)) if text.is_empty() => Some(Value::Null),
        ("null", Value::Number(number)) if number.as_i64() == Some(0) => Some(Value::Null),
        ("null", Value::Bool(false)) => Some(Value::Null),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::JsonSchema;
    use serde_json::json;

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct Arguments {
        entries: Vec<Entry>,
        optional: Option<Entry>,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct Entry {
        count: u32,
        label: String,
    }

    #[test]
    fn generated_nested_arguments_resolve_refs_for_validation_and_coercion() {
        let schema = serde_json::to_value(schemars::schema_for!(Arguments)).unwrap();
        let valid = json!({"entries": [{"count": 2, "label": "hi"}], "optional": null});
        assert!(validate_arguments(&schema, &valid).is_ok());
        assert!(validate_arguments(
            &schema,
            &json!({"entries": [{"count": "bad", "label": "hi"}]})
        )
        .is_err());
        assert!(
            validate_arguments(&schema, &json!({"entries": [{"count": -1, "label": "hi"}]}))
                .is_err()
        );
        let coerced = coerce_arguments(
            &schema,
            json!({"entries": [{"count": "2", "label": 3}], "optional": null}),
        );
        assert_eq!(
            coerced,
            json!({"entries": [{"count": 2, "label": "3"}], "optional": null})
        );
        assert!(validate_arguments(&schema, &coerced).is_ok());
    }

    #[test]
    fn dynamic_schemas_enforce_all_constraints() {
        let schema = json!({
            "type": "object", "required": ["name", "scores"], "additionalProperties": false,
            "properties": {
                "name": {"type": "string", "minLength": 2, "maxLength": 4, "pattern": "^[A-Z]+$"},
                "scores": {"type": "array", "minItems": 1, "maxItems": 2, "uniqueItems": true, "items": {"type": "number", "minimum": 1, "maximum": 5, "multipleOf": 0.5}},
                "tag": {"const": "ok"}
            }
        });
        assert!(validate_arguments(
            &schema,
            &json!({"name":"AB", "scores":[1, 2.5], "tag":"ok"})
        )
        .is_ok());
        for invalid in [
            json!({"name":"A", "scores":[1]}),
            json!({"name":"ABCDE", "scores":[1]}),
            json!({"name":"ab", "scores":[1]}),
            json!({"name":"AB", "scores":[]}),
            json!({"name":"AB", "scores":[1, 2, 3]}),
            json!({"name":"AB", "scores":[1, 1]}),
            json!({"name":"AB", "scores":[0]}),
            json!({"name":"AB", "scores":[6]}),
            json!({"name":"AB", "scores":[1.2]}),
            json!({"name":"AB", "scores":[1], "tag":"bad"}),
            json!({"name":"AB", "scores":[1], "extra":true}),
        ] {
            assert!(validate_arguments(&schema, &invalid).is_err(), "{invalid}");
        }
        let composed = json!({"allOf":[{"type":"integer"}, {"minimum":2}], "anyOf":[{"maximum":3}, {"minimum":8}]});
        assert!(validate_arguments(&composed, &json!(8)).is_ok());
        for invalid in [json!(1), json!(5), json!("8")] {
            assert!(validate_arguments(&composed, &invalid).is_err());
        }
        let conditional = json!({"if":{"const":1}, "then":false, "else":{"not":{"const":2}}});
        assert!(validate_arguments(&conditional, &json!(3)).is_ok());
        assert!(validate_arguments(&conditional, &json!(1)).is_err());
        assert!(validate_arguments(&conditional, &json!(2)).is_err());
    }

    #[test]
    fn coercion_preserves_null_and_union_semantics() {
        let schema = json!({"type":"object", "required":["required"], "properties":{
            "required":{"type":"integer"}, "optional":{"$ref":"#/$defs/count"},
            "nullable":{"type":["string","null"]}, "flag":{"type":"boolean"},
            "choice":{"anyOf":[{"$ref":"#/$defs/count"},{"type":"boolean"}]}
        }, "$defs":{"count":{"type":"integer", "minimum":1}}});
        let coerced = coerce_arguments(
            &schema,
            json!({"required":null, "optional":null, "nullable":null, "flag":"true", "choice":"3"}),
        );
        assert_eq!(
            coerced,
            json!({"required":null, "nullable":null, "flag":true, "choice":3})
        );
        assert!(validate_arguments(&schema, &coerced).is_err());
        let union = json!({"oneOf":[{"type":"integer"}, {"type":"object"}]});
        assert_eq!(coerce_arguments(&union, json!("12")), json!(12));
        assert_eq!(coerce_arguments(&union, json!({})), json!({}));
        assert_eq!(coerce_arguments(&union, json!("bad")), json!("bad"));
        assert!(validate_arguments(
            &json!({"oneOf":[{"type":"integer"},{"type":"number"}]}),
            &json!(1)
        )
        .is_err());
    }

    #[test]
    fn invalid_schemas_and_external_references_fail_closed() {
        for schema in [
            json!({"$ref":"https://example.invalid/schema"}),
            json!({"$ref":"file:///C:/Windows/win.ini"}),
            json!({"$ref":"#/$defs/missing"}),
            json!({"type":"nonsense"}),
            json!({"minimum":"bad"}),
            json!({"pattern":"["}),
        ] {
            assert!(validate_arguments(&schema, &json!({}))
                .unwrap_err()
                .contains("invalid arguments schema"));
            assert_eq!(coerce_arguments(&schema, json!("5")), json!("5"));
        }
        assert!(validate_arguments(&Value::Null, &json!({"anything":true})).is_ok());
        assert!(validate_arguments(&json!(true), &json!({})).is_ok());
        assert!(validate_arguments(&json!(false), &json!({})).is_err());
    }

    #[test]
    fn recursive_refs_coerce_finite_values_without_looping() {
        let schema = json!({"$ref":"#/$defs/node", "$defs":{"node":{"type":"object", "properties":{
            "count":{"type":"integer"}, "next":{"$ref":"#/$defs/node"}
        }}}});
        let value = coerce_arguments(&schema, json!({"count":"1", "next":{"count":"2"}}));
        assert_eq!(value, json!({"count":1,"next":{"count":2}}));
        assert!(validate_arguments(&schema, &value).is_ok());
        assert_eq!(coerce_arguments(&json!({"$ref":"#"}), json!(1)), json!(1));
    }

    #[test]
    fn existing_errors_keep_argument_paths() {
        assert_eq!(
            validate_arguments(
                &json!({"type":"object","additionalProperties":false}),
                &json!({"extra":true})
            )
            .unwrap_err(),
            "arguments.extra is not allowed"
        );
        assert_eq!(
            validate_arguments(&json!({"type":"object","required":["name"]}), &json!({}))
                .unwrap_err(),
            "arguments.name is required"
        );
        assert_eq!(
            validate_arguments(
                &json!({"type":"array","items":{"type":"integer"}}),
                &json!(["bad"])
            )
            .unwrap_err(),
            "arguments[0] must be integer"
        );
    }

    #[test]
    fn scoped_and_anchor_references_validate_without_guessing_coercion() {
        let schema = json!({
            "$id":"https://example.invalid/root", "$ref":"#/$defs/scoped", "$defs":{
                "scoped":{"$id":"child", "$ref":"#/$defs/count", "$defs":{"count":{"type":"integer","minimum":2}}},
                "count":{"type":"boolean"}
            }
        });
        assert!(validate_arguments(&schema, &json!(2)).is_ok());
        assert!(validate_arguments(&schema, &json!(true)).is_err());
        assert_eq!(coerce_arguments(&schema, json!("2")), json!("2"));
        let anchored =
            json!({"$ref":"#count", "$defs":{"count":{"$anchor":"count", "type":"integer"}}});
        assert!(validate_arguments(&anchored, &json!(2)).is_ok());
        assert!(validate_arguments(&anchored, &json!("bad")).is_err());
        assert_eq!(coerce_arguments(&anchored, json!("2")), json!("2"));
    }
}
