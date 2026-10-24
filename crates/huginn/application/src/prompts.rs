//! Lower-trust Markdown tasks. Rendering never performs IO or evaluates values.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const MAX_TEMPLATE_BYTES: usize = 64 * 1024;
pub const MAX_RENDERED_BYTES: usize = 256 * 1024;
pub const MAX_TEMPLATES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptEntry {
    pub name: String,
    pub description: String,
    pub argument_hint: String,
    pub path: PathBuf,
    pub project: bool,
}
#[derive(Debug, Clone, Default)]
pub struct PromptCatalog {
    pub entries: Vec<PromptEntry>,
    pub diagnostics: Vec<String>,
}
/// Runtime owns all filesystem access, including fresh scope checks on invocation.
pub trait PromptTemplates {
    fn discover(&self) -> PromptCatalog;
    fn expand(&self, input: &str) -> Result<Option<String>, String>;
}
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
pub fn invocation(input: &str) -> Result<Option<(&str, &str)>, String> {
    let Some(rest) = input.strip_prefix("/prompts:") else {
        return Ok(None);
    };
    let split = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let (name, args) = rest.split_at(split);
    if !valid_name(name) {
        return Err("Nombre de plantilla inválido".into());
    }
    Ok(Some((name, args.trim_start())))
}

/// Shell-like quoting only: no shell, expansion, comments, or command substitution.
pub fn arguments(input: &str) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut started = false;
    for c in input.chars() {
        if escaped {
            token.push(c);
            escaped = false;
            started = true;
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            escaped = true;
            started = true;
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                token.push(c);
            }
        } else if c == '\'' || c == '"' {
            quote = Some(c);
            started = true;
        } else if c.is_whitespace() {
            if started {
                result.push(std::mem::take(&mut token));
                started = false;
            }
        } else {
            token.push(c);
            started = true;
        }
    }
    if quote.is_some() || escaped {
        return Err("Argumentos con comillas o escape incompletos".into());
    }
    if started {
        result.push(token);
    }
    Ok(result)
}

pub fn render(body: &str, input: &str) -> Result<String, String> {
    if body.len() > MAX_TEMPLATE_BYTES || input.len() > MAX_RENDERED_BYTES {
        return Err("Plantilla o argumentos demasiado grandes".into());
    }
    let args = arguments(input)?;
    let named: BTreeMap<_, _> = args.iter().filter_map(|a| a.split_once('=')).collect();
    let joined = args.join(" ");
    let mut out = String::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            out.push(c);
        } else {
            match chars.peek().copied() {
                Some('$') => {
                    chars.next();
                    out.push('$');
                }
                Some('@') => {
                    chars.next();
                    out.push_str(&joined);
                }
                Some(d @ '1'..='9') => {
                    chars.next();
                    if chars.peek().is_some_and(char::is_ascii_digit) {
                        return Err("Solo se admiten posiciones $1 a $9".into());
                    }
                    out.push_str(
                        args.get(d as usize - '1' as usize)
                            .ok_or_else(|| format!("Falta el argumento ${d}"))?,
                    );
                }
                Some('A'..='Z') => {
                    let mut key = String::new();
                    while chars
                        .peek()
                        .is_some_and(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
                    {
                        key.push(chars.next().expect("peeked character"));
                    }
                    if key == "ARGUMENTS" {
                        out.push_str(&joined);
                    } else {
                        out.push_str(
                            named
                                .get(key.as_str())
                                .ok_or_else(|| format!("Falta el argumento {key}=valor"))?,
                        );
                    }
                }
                _ => out.push('$'),
            }
        }
        if out.len() > MAX_RENDERED_BYTES {
            return Err("Resultado de plantilla demasiado grande".into());
        }
    }
    Ok(out)
}

/// Small strict frontmatter, not a general YAML interpreter.
pub fn metadata(text: &str) -> Result<(String, String, &str), String> {
    let Some(first_end) = text.find('\n') else {
        return Ok((String::new(), String::new(), text));
    };
    if text[..first_end].trim_end_matches('\r') != "---" {
        return Ok((String::new(), String::new(), text));
    }
    let mut description = String::new();
    let mut hint = String::new();
    let mut offset = first_end + 1;
    for line in text[offset..].split_inclusive('\n') {
        offset += line.len();
        let line = line.trim_end_matches(['\r', '\n']);
        if line == "---" {
            return Ok((description, hint, &text[offset..]));
        }
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once(':')
            .ok_or("Metadatos inválidos: se esperaba clave: valor")?;
        let value = value.trim();
        let value = if value.starts_with(['\'', '"']) {
            if value.len() < 2 || !value.ends_with(value.chars().next().unwrap()) {
                return Err("Comillas de metadatos incompletas".into());
            }
            &value[1..value.len() - 1]
        } else {
            value
        };
        match key.trim() {
            "description" => description = value.to_string(),
            "argument-hint" | "argument_hint" => hint = value.to_string(),
            _ => return Err(format!("Metadato no admitido: {key}")),
        }
    }
    Err("Frontmatter sin cierre ---".into())
}

// Keep path types at this boundary; only runtime resolves them.
pub fn project_prompt_root(project: &Path) -> PathBuf {
    project.join(".huginn/prompts")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_literal_expansion() {
        assert_eq!(
            render(
                "Review $1 with focus $2; context: $@",
                "src/lib.rs \"thread safety\""
            )
            .unwrap(),
            "Review src/lib.rs with focus thread safety; context: src/lib.rs thread safety"
        );
        assert_eq!(
            render("$1 $$HOME $ARGUMENTS", "'$(touch nope) $2 café 🦀'").unwrap(),
            "$(touch nope) $2 café 🦀 $HOME $(touch nope) $2 café 🦀"
        );
        assert_eq!(
            render("$FOCUS", "FOCUS='thread safety'").unwrap(),
            "thread safety"
        );
    }
    #[test]
    fn quoting_and_errors() {
        assert_eq!(
            arguments("a '' \"b c\" d\\ e").unwrap(),
            ["a", "", "b c", "d e"]
        );
        assert!(render("$2", "one").is_err());
        assert!(render("$TOKEN", "").is_err());
        assert!(render("$1", "'open").is_err());
        assert!(render("$10", "one").is_err());
        assert!(invocation("/prompts:../secret").is_err());
        assert_eq!(invocation("/login").unwrap(), None);
        assert!(render(&"a".repeat(MAX_TEMPLATE_BYTES + 1), "").is_err());
        assert!(render("$1$1$1$1$1", &"a".repeat(MAX_TEMPLATE_BYTES)).is_err());
    }
    #[test]
    fn frontmatter_and_multiline() {
        assert_eq!(
            metadata("---\r\ndescription: 'Review'\r\nargument-hint: file\r\n---\r\n$1\nnext")
                .unwrap(),
            ("Review".into(), "file".into(), "$1\nnext")
        );
        assert!(metadata("---\nbroken\n---\nbody").is_err());
        assert!(metadata("---\ndescription: missing close\n").is_err());
        assert_eq!(render("α\n$1\n", "'β γ'").unwrap(), "α\nβ γ\n");
    }
}
