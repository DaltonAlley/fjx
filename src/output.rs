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
