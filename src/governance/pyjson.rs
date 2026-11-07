//! Python-compatible JSON rendering: `json.dumps(value, indent, sort_keys=True, ensure_ascii=True)`.
//! The tracker page must stay byte-identical to CPython's output, so this reproduces it exactly.

use std::fmt::{self, Write};

use lgwks_std::json::{Map, Number, Value};

use super::pystr::Classes;

/// Renders `value` as CPython's `json.dumps` would with `sort_keys=True` and `ensure_ascii=True`.
/// `indent` of `None` writes `, ` and `: ` separators; `Some(n)` puts each item on its own line,
/// indented `n` spaces per depth.
pub(crate) fn dumps(value: &Value, indent: Option<usize>) -> String {
    let mut out = String::new();
    if write_value(&mut out, value, indent, 0).is_err() {
        return String::new();
    }
    out
}

/// Appends one value at `depth` nesting levels.
fn write_value(
    out: &mut String,
    value: &Value,
    indent: Option<usize>,
    depth: usize,
) -> fmt::Result {
    match *value {
        Value::Null => out.write_str("null"),
        Value::Bool(true) => out.write_str("true"),
        Value::Bool(false) => out.write_str("false"),
        Value::Number(ref number) => write_number(out, number),
        Value::String(ref text) => write_string(out, text.as_str()),
        Value::Array(ref items) => write_array(out, items, indent, depth),
        Value::Object(ref members) => write_object(out, members, indent, depth),
        _ => out.write_str("null"),
    }
}

/// Appends an array; an empty one is `[]`.
fn write_array(
    out: &mut String,
    items: &[Value],
    indent: Option<usize>,
    depth: usize,
) -> fmt::Result {
    if items.is_empty() {
        return out.write_str("[]");
    }
    out.write_char('[')?;
    for (index, item) in items.iter().enumerate() {
        separate(out, index, indent, depth)?;
        write_value(out, item, indent, depth.saturating_add(1))?;
    }
    close(out, indent, depth)?;
    out.write_char(']')
}

/// Appends an object with its keys in sorted order; an empty one is `{}`.
fn write_object(
    out: &mut String,
    members: &Map,
    indent: Option<usize>,
    depth: usize,
) -> fmt::Result {
    if members.is_empty() {
        return out.write_str("{}");
    }
    out.write_char('{')?;
    for (index, (key, member)) in members.iter().enumerate() {
        separate(out, index, indent, depth)?;
        write_string(out, key)?;
        out.write_str(": ")?;
        write_value(out, member, indent, depth.saturating_add(1))?;
    }
    close(out, indent, depth)?;
    out.write_char('}')
}

/// Writes the separator before item `index` and the line break for an indented layout.
fn separate(out: &mut String, index: usize, indent: Option<usize>, depth: usize) -> fmt::Result {
    if index > 0 {
        out.write_char(',')?;
        if indent.is_none() {
            out.write_char(' ')?;
        }
    }
    match indent {
        Some(width) => {
            out.write_char('\n')?;
            push_spaces(out, width.saturating_mul(depth.saturating_add(1)))
        }
        None => Ok(()),
    }
}

/// Writes the newline and indentation before a closing bracket of an indented layout.
fn close(out: &mut String, indent: Option<usize>, depth: usize) -> fmt::Result {
    match indent {
        Some(width) => {
            out.write_char('\n')?;
            push_spaces(out, width.saturating_mul(depth))
        }
        None => Ok(()),
    }
}

/// Appends `count` spaces.
fn push_spaces(out: &mut String, count: usize) -> fmt::Result {
    for _ in 0..count {
        out.write_char(' ')?;
    }
    Ok(())
}

/// Appends a number: integers as digits, floats by Python's `repr`.
fn write_number(out: &mut String, number: &Number) -> fmt::Result {
    if let Some(integer) = number.as_u64() {
        write!(out, "{integer}")
    } else if let Some(integer) = number.as_i64() {
        write!(out, "{integer}")
    } else if let Some(float) = number.as_f64() {
        out.write_str(&float_repr(float))
    } else {
        out.write_str("null")
    }
}

/// Python's `repr(float)`: shortest round-trip digits, fixed notation when the decimal exponent
/// is in `-4..16`, otherwise `d.ddde+NN`.
pub(crate) fn float_repr(float: f64) -> String {
    if float == 0.0 {
        return if float.is_sign_negative() {
            String::from("-0.0")
        } else {
            String::from("0.0")
        };
    }
    let sign = if float.is_sign_negative() { "-" } else { "" };
    let scientific = format!("{:e}", float.abs());
    let Some((mantissa, exponent_text)) = scientific.split_once('e') else {
        return format!("{sign}{}", float.abs());
    };
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let Ok(exponent) = exponent_text.parse::<i32>() else {
        return format!("{sign}{}", float.abs());
    };
    let body = if (-4..16).contains(&exponent) {
        fixed(&digits, exponent)
    } else {
        scientific_form(&digits, exponent)
    };
    format!("{sign}{body}")
}

/// Fixed notation for `digits` scaled by `10^exponent`, always with a fractional part. Only called
/// for `exponent` in `-4..16`, so both conversions below succeed; a refusal keeps the digits whole.
fn fixed(digits: &str, exponent: i32) -> String {
    if exponent < 0 {
        let Ok(leading) = usize::try_from(exponent.unsigned_abs().saturating_sub(1)) else {
            return digits.to_owned();
        };
        return format!("0.{}{digits}", "0".repeat(leading));
    }
    let Ok(whole_digits) = usize::try_from(exponent) else {
        return digits.to_owned();
    };
    let point = whole_digits.saturating_add(1);
    if digits.len() <= point {
        let pad = "0".repeat(point.saturating_sub(digits.len()));
        return format!("{digits}{pad}.0");
    }
    let (whole, fraction) = digits.split_at(point);
    format!("{whole}.{fraction}")
}

/// Scientific notation: `d.ddde+NN`, a single digit with no fraction is `de+NN`.
fn scientific_form(digits: &str, exponent: i32) -> String {
    let sign = if exponent < 0 { '-' } else { '+' };
    let magnitude = exponent.unsigned_abs();
    let (lead, rest) = digits.split_at(1);
    let fraction = if rest.is_empty() {
        String::new()
    } else {
        format!(".{rest}")
    };
    format!("{lead}{fraction}e{sign}{magnitude:02}")
}

/// Python `str(value)` for a value `json.load` produced: a string unquoted, anything else as
/// its `repr`.
pub(crate) fn python_str(value: &Value, classes: &Classes) -> String {
    match *value {
        Value::String(ref text) => String::from(text.as_str()),
        _ => python_repr(value, classes),
    }
}

/// Python `repr(value)` for a value `json.load` produced: `None`, `True`, `False`, numbers,
/// quoted strings, `[...]` and `{'key': value}` with the keys in sorted order.
pub(crate) fn python_repr(value: &Value, classes: &Classes) -> String {
    match *value {
        Value::Bool(true) => String::from("True"),
        Value::Bool(false) => String::from("False"),
        Value::Number(ref number) => {
            let mut out = String::new();
            if write_number(&mut out, number).is_err() {
                return String::from("None");
            }
            out
        }
        Value::String(ref text) => classes.repr(text.as_str()),
        Value::Array(ref items) => {
            let parts: Vec<String> = items
                .iter()
                .map(|item| python_repr(item, classes))
                .collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Object(ref members) => {
            let parts: Vec<String> = members
                .iter()
                .map(|(key, member)| {
                    format!("{}: {}", classes.repr(key), python_repr(member, classes))
                })
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
        _ => String::from("None"),
    }
}

/// Appends a JSON string literal with Python's `ensure_ascii` escapes.
fn write_string(out: &mut String, text: &str) -> fmt::Result {
    out.write_char('"')?;
    for unit in text.encode_utf16() {
        match unit {
            0x22 => out.write_str("\\\"")?,
            0x5c => out.write_str("\\\\")?,
            0x0a => out.write_str("\\n")?,
            0x0d => out.write_str("\\r")?,
            0x09 => out.write_str("\\t")?,
            0x08 => out.write_str("\\b")?,
            0x0c => out.write_str("\\f")?,
            0x20..=0x7e => write_printable(out, unit)?,
            _ => write!(out, "\\u{unit:04x}")?,
        }
    }
    out.write_char('"')
}

/// Appends one ASCII unit in `0x20..=0x7e`, the range Python's `ensure_ascii` writes literally
/// (`S_CHAR` in `_json.c`: space to tilde; DEL is escaped). A unit that is no character is
/// written as its escape, as every other unit is.
fn write_printable(out: &mut String, unit: u16) -> fmt::Result {
    match char::from_u32(u32::from(unit)) {
        Some(character) => out.write_char(character),
        None => write!(out, "\\u{unit:04x}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pyjson_floats_match_python_repr() -> Result<(), Box<dyn std::error::Error>> {
        let vectors: [(f64, &str); 13] = [
            (37.5, "37.5"),
            (40.0, "40.0"),
            (1e16, "1e+16"),
            (1e15, "1000000000000000.0"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (1.5e-5, "1.5e-05"),
            (1.2345678901234568e17, "1.2345678901234568e+17"),
            (0.1, "0.1"),
            (-0.0, "-0.0"),
            (1e100, "1e+100"),
            (5e-324, "5e-324"),
            (-2.5, "-2.5"),
        ];
        for (value, expected) in vectors {
            assert_eq!(
                float_repr(value),
                expected,
                "repr of {value:?} must match Python"
            );
        }
        Ok(())
    }

    #[test]
    fn pyjson_strings_match_python_escapes() -> Result<(), Box<dyn std::error::Error>> {
        let vectors: [(&str, &str); 4] = [
            ("é", "\"\\u00e9\""),
            ("😀", "\"\\ud83d\\ude00\""),
            ("\u{1}", "\"\\u0001\""),
            ("\u{7f}~", "\"\\u007f~\""),
        ];
        for (input, expected) in vectors {
            let mut out = String::new();
            write_string(&mut out, input)?;
            assert_eq!(
                out, expected,
                "escape of {input:?} must match Python ensure_ascii"
            );
        }
        Ok(())
    }
}
