//! Deliberately bounded MCP form schema support. Unsupported assertions are rejected at
//! parsing; this same validator owns submission validation and successful wire encoding.
use serde_json::Value;

use super::{AskUserAnswer, AskUserQuestion};

pub(crate) fn supports_value_schema(schema: &Value) -> bool {
    let Some(object) = schema.as_object() else {
        return false;
    };
    let Some(kind @ ("string" | "number" | "integer" | "boolean")) = schema["type"].as_str() else {
        return false;
    };
    for (key, value) in object {
        let supported = match key.as_str() {
            "type" => true,
            "title" | "description" => value.is_string(),
            "default" => true, // annotation only; never insert a value on the user's behalf
            "enum" => value.as_array().is_some_and(|values| {
                (2..=6).contains(&values.len())
                    && values.iter().enumerate().all(|(index, value)| {
                        !values[..index].contains(value) && check_value(schema, value).is_ok()
                    })
            }),
            "minimum" | "maximum" if matches!(kind, "number" | "integer") => {
                value.is_number() && parse_number(&value.to_string(), false).is_ok()
            }
            "minLength" | "maxLength" if kind == "string" => value.as_u64().is_some(),
            _ => false,
        };
        if !supported {
            return false;
        }
    }
    if let (Some(min), Some(max)) = (schema["minimum"].as_f64(), schema["maximum"].as_f64()) {
        if min > max {
            return false;
        }
    }
    if let (Some(min), Some(max)) = (schema["minLength"].as_u64(), schema["maxLength"].as_u64()) {
        if min > max {
            return false;
        }
    }
    true
}

/// A decimal's exact lexical value, independent of exponent spelling and trailing zeros.
/// Comparing this with the serialized f64 catches silent rounding and underflow.
fn decimal_identity(text: &str) -> Option<(bool, String, i64)> {
    let (mantissa, exponent) = text.split_once(['e', 'E']).unwrap_or((text, "0"));
    let negative = mantissa.starts_with('-');
    let mantissa = mantissa.strip_prefix('-').unwrap_or(mantissa);
    let fractional = mantissa.split_once('.').map_or(0, |(_, part)| part.len());
    let mut exponent = exponent
        .parse::<i64>()
        .ok()?
        .checked_sub(fractional as i64)?;
    let mut digits = mantissa
        .replace('.', "")
        .trim_start_matches('0')
        .to_string();
    if digits.is_empty() {
        return Some((false, "0".into(), 0));
    }
    while digits.ends_with('0') {
        digits.pop();
        exponent = exponent.checked_add(1)?;
    }
    Some((negative, digits, exponent))
}

fn parse_number(text: &str, integer: bool) -> Result<Value, String> {
    let text = text.trim();
    let parsed: Value = serde_json::from_str(text).map_err(|_| "Expected a valid JSON number")?;
    let number = parsed
        .as_f64()
        .filter(|number| number.is_finite())
        .ok_or("Expected a finite number")?;
    if integer && number.fract() != 0.0 {
        return Err("Expected an integer".into());
    }
    if number.fract() == 0.0 && number.abs() > 9_007_199_254_740_991.0 {
        return Err("Number exceeds the lossless integer range".into());
    }
    let encoded = serde_json::Number::from_f64(number).ok_or("Expected a finite number")?;
    if decimal_identity(text) != decimal_identity(&encoded.to_string()) {
        return Err("Number cannot be represented without precision loss".into());
    }
    Ok(if integer {
        Value::from(number as i64)
    } else {
        Value::Number(encoded)
    })
}

fn check_value(schema: &Value, value: &Value) -> Result<(), String> {
    match schema["type"].as_str() {
        Some("string") => {
            let text = value.as_str().ok_or("Expected a string")?;
            let length = text.chars().count() as u64;
            if schema["minLength"].as_u64().is_some_and(|min| length < min) {
                return Err("String is shorter than minLength".into());
            }
            if schema["maxLength"].as_u64().is_some_and(|max| length > max) {
                return Err("String is longer than maxLength".into());
            }
        }
        Some(kind @ ("integer" | "number")) => {
            if !value.is_number() {
                return Err("Expected a number".into());
            }
            let number = parse_number(&value.to_string(), kind == "integer")?
                .as_f64()
                .unwrap();
            if schema["minimum"].as_f64().is_some_and(|min| number < min) {
                return Err("Number is below minimum".into());
            }
            if schema["maximum"].as_f64().is_some_and(|max| number > max) {
                return Err("Number is above maximum".into());
            }
        }
        Some("boolean") if value.is_boolean() => {}
        _ => return Err("Unsupported value type".into()),
    }
    if schema["enum"]
        .as_array()
        .is_some_and(|values| !values.contains(value))
    {
        return Err("Value is not a member of enum".into());
    }
    Ok(())
}

pub(crate) fn answer_value(
    question: &AskUserQuestion,
    answer: &AskUserAnswer,
) -> Result<Value, String> {
    let result = (|| {
        let schema = question
            .value_schema
            .as_ref()
            .ok_or("Missing value schema")?;
        if !supports_value_schema(schema) {
            return Err("Unsupported value schema".into());
        }
        let value = if let Some(text) = &answer.custom_text {
            if !question.allow_custom {
                return Err("Custom text is not allowed".into());
            }
            match schema["type"].as_str() {
                Some("string") => Value::String(text.clone()),
                Some("integer") => parse_number(text, true)?,
                Some("number") => parse_number(text, false)?,
                _ => return Err("Custom text is not supported for this type".into()),
            }
        } else {
            if answer.selected_option_ids.len() != 1 {
                return Err("Expected one selected option".into());
            }
            let selected = &answer.selected_option_ids[0];
            if !question.options.iter().any(|option| option.id == *selected) {
                return Err("Unknown selected option".into());
            }
            if let Some(values) = schema["enum"].as_array() {
                selected
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| values.get(index))
                    .cloned()
                    .ok_or("Unknown enum option")?
            } else if schema["type"] == "boolean" {
                match selected.as_str() {
                    "true" => Value::Bool(true),
                    "false" => Value::Bool(false),
                    _ => return Err("Unknown boolean option".into()),
                }
            } else {
                return Err("Expected custom text".into());
            }
        };
        check_value(schema, &value)?;
        Ok(value)
    })();
    result.map_err(|error: String| format!("Question `{}`: {error}", question.id))
}
