use std::fmt::{self, Write};

use serde::Serialize;
use serde_json::Value;

use crate::error::Error;

pub(crate) struct Outcome {
    pub(crate) bytes: Vec<u8>,
    pub(crate) code: u8,
}

pub(crate) struct PlainField<'a>(&'a str);

pub(crate) const fn plain_field(value: &str) -> PlainField<'_> {
    PlainField(value)
}

/// Keep line breaks for human-readable bodies without allowing terminal controls.
pub(crate) fn human_text(value: &str) -> String {
    value
        .split('\n')
        .map(|line| plain_field(line).to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn validate_fields(fields: &[String], allowed: Option<&[&str]>) -> Result<(), Error> {
    if fields.is_empty() {
        return Ok(());
    }
    let allowed = allowed.ok_or_else(|| {
        Error::usage("--fields is available only for typed JSON records; see command help")
    })?;
    for (index, field) in fields.iter().enumerate() {
        if fields[..index].contains(field) {
            return Err(Error::usage(format!("duplicate output field {field:?}")));
        }
        if !allowed.contains(&field.as_str()) {
            return Err(Error::usage(format!(
                "unknown output field {field:?}; available fields: {}",
                allowed.join(",")
            )));
        }
    }
    Ok(())
}

/// Projection happens only after a complete command result, never page by page.
pub(crate) fn project(outcome: Outcome, fields: &[String]) -> Result<Outcome, Error> {
    if fields.is_empty() {
        return Ok(outcome);
    }
    let value: Value = serde_json::from_slice(&outcome.bytes)
        .map_err(|_| Error::data("could not project a non-JSON command result"))?;
    let projected = match value {
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| project_record(value, fields))
                .collect::<Result<_, _>>()?,
        ),
        value => project_record(value, fields)?,
    };
    Ok(Outcome::json(&projected)?.with_code(outcome.code))
}

fn project_record(value: Value, fields: &[String]) -> Result<Value, Error> {
    let Value::Object(mut record) = value else {
        return Err(Error::data("expected a JSON object for field selection"));
    };
    let mut projected = serde_json::Map::new();
    for field in fields {
        // Optional fields remain present as null, so absence never changes shape.
        projected.insert(field.clone(), record.remove(field).unwrap_or(Value::Null));
    }
    Ok(Value::Object(projected))
}

impl fmt::Display for PlainField<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for scalar in self.0.chars() {
            match scalar {
                '\\' => formatter.write_str("\\\\")?,
                '\t' => formatter.write_str("\\t")?,
                '\r' => formatter.write_str("\\r")?,
                '\n' => formatter.write_str("\\n")?,
                scalar if scalar.is_control() => {
                    write!(formatter, "\\u{{{:X}}}", u32::from(scalar))?;
                }
                scalar => formatter.write_char(scalar)?,
            }
        }
        Ok(())
    }
}

impl Outcome {
    pub(crate) fn text(text: impl Into<Vec<u8>>) -> Self {
        Self {
            bytes: text.into(),
            code: 0,
        }
    }

    pub(crate) fn json<T: Serialize>(value: &T) -> Result<Self, Error> {
        let mut bytes = serde_json::to_vec(value)
            .map_err(|error| Error::data(format!("could not encode output: {error}")))?;
        bytes.push(b'\n');
        Ok(Self::text(bytes))
    }

    pub(crate) fn with_code(mut self, code: u8) -> Self {
        self.code = code;
        self
    }

    pub(crate) fn raw_json(value: &Value, compact: bool) -> Result<Self, Error> {
        let mut bytes = if compact {
            serde_json::to_vec(value)
        } else {
            serde_json::to_vec_pretty(value)
        }
        .map_err(|error| Error::data(format!("could not encode output: {error}")))?;
        bytes.push(b'\n');
        Ok(Self::text(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::{Outcome, human_text, project, validate_fields};
    use serde_json::json;

    #[test]
    fn human_bodies_preserve_lines_without_terminal_escapes() {
        assert_eq!(
            human_text("a\nb\r\t\u{1b}[31m\\"),
            "a\nb\\r\\t\\u{1B}[31m\\\\"
        );
    }

    #[test]
    fn projection_preserves_null_arrays_and_failure_exit() {
        let fields = vec!["number".into(), "optional".into()];
        let source = Outcome::json(&json!([{ "number": 3, "body": "long" }]))
            .unwrap_or_else(|error| panic!("{error}"))
            .with_code(1);
        let result = project(source, &fields).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(result.code, 1);
        let parsed: serde_json::Value =
            serde_json::from_slice(&result.bytes).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(parsed, json!([{ "number": 3, "optional": null }]));
        let empty = project(
            Outcome::json(&json!([])).unwrap_or_else(|error| panic!("{error}")),
            &fields,
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(empty.bytes, b"[]\n");
    }

    #[test]
    fn field_validation_precedes_execution() {
        assert!(validate_fields(&["number".into()], Some(&["number"])).is_ok());
        assert!(validate_fields(&["typo".into()], Some(&["number"])).is_err());
        assert!(validate_fields(&["number".into()], None).is_err());
        assert!(validate_fields(&["number".into(), "number".into()], Some(&["number"])).is_err());
    }
}
