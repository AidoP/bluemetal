//! Generation of the JSON specifications of custom targets, which rustc uses in place of a builtin target.

use std::path::PathBuf;

use crate::project::{CustomTarget, Data};

/// The specification of a custom target.
pub fn path(name: &str) -> PathBuf {
    PathBuf::from(format!("target/targets/{name}.json"))
}

/// Returns the specification of a target, from the specification of the builtin target it inherits, if any, with its
/// fields replaced or added.
pub fn spec(target: &CustomTarget, inherited: Option<&str>) -> Result<String, &'static str> {
    let mut fields = match inherited {
        Some(json) => object_fields(json).ok_or("invalid target specification from rustc")?,
        None => Vec::new(),
    };
    for (name, data) in &target.fields {
        let value = json(data);
        match fields.iter_mut().find(|(field, _)| field == name) {
            Some((_, inherited)) => *inherited = value,
            None => fields.push((name.clone(), value)),
        }
    }
    let fields: Vec<_> = fields.iter().map(|(name, value)| format!("  {}: {value}", crate::rust_analyzer::string(name))).collect();
    Ok(format!("{{\n{}\n}}\n", fields.join(",\n")))
}

/// Returns a value as JSON.
fn json(data: &Data) -> String {
    match data {
        Data::None => "null".into(),
        Data::Str(s) => crate::rust_analyzer::string(s),
        Data::Bool(b) => b.to_string(),
        Data::Unsigned(i) => i.to_string(),
        Data::Signed(i) => i.to_string(),
        Data::List(items) => {
            let items: Vec<_> = items.iter().map(json).collect();
            format!("[{}]", items.join(", "))
        },
    }
}

/// Split a JSON object into its fields, as names and the JSON of their values, which are kept as they are.
fn object_fields(json: &str) -> Option<Vec<(String, String)>> {
    let mut rest = json.trim().strip_prefix('{')?.trim_start();
    let mut fields = Vec::new();
    if let Some(after) = rest.strip_prefix('}') {
        return after.trim().is_empty().then_some(fields);
    }
    loop {
        // the names of specification fields do not need escapes
        let name_end = rest.strip_prefix('"')?.find('"')?;
        let name = rest[1..1 + name_end].to_string();
        rest = rest[name_end + 2..].trim_start().strip_prefix(':')?.trim_start();
        let len = value_len(rest)?;
        fields.push((name, rest[..len].trim_end().to_string()));
        rest = rest[len..].trim_start();
        match rest.as_bytes().first()? {
            b',' => rest = rest[1..].trim_start(),
            b'}' => return rest[1..].trim().is_empty().then_some(fields),
            _ => return None,
        }
    }
}

/// Returns the length of the JSON value at the start of `s`.
fn value_len(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if in_string => i += 1,
            b'"' => in_string = !in_string,
            _ if in_string => (),
            b'{' | b'[' => depth += 1,
            b'}' | b']' if depth == 0 => return Some(i),
            b'}' | b']' => depth -= 1,
            b',' if depth == 0 => return Some(i),
            _ => (),
        }
        i += 1;
    }
    (depth == 0 && !in_string).then_some(bytes.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields() {
        let json = r#"{
  "arch": "riscv64",
  "metadata": { "description": "a, \"b\" {", "tier": 2 },
  "list": [1, [2, 3]],
  "flag": true
}"#;
        assert_eq!(object_fields(json).unwrap(), [
            ("arch".to_string(), "\"riscv64\"".to_string()),
            ("metadata".into(), r#"{ "description": "a, \"b\" {", "tier": 2 }"#.into()),
            ("list".into(), "[1, [2, 3]]".into()),
            ("flag".into(), "true".into()),
        ]);
        assert_eq!(object_fields("{}").unwrap(), []);
        assert!(object_fields("{\"a\": 1").is_none());
        assert!(object_fields("[]").is_none());
    }
}
