//! Nyquest Code Minifier
//! Safe, AST-free code compression for Python, JavaScript/TypeScript, and shell.
//! Uses simple state-machine parsing to handle strings, comments, and docstrings
//! without regex (avoids false positives on comment-like syntax in strings).
//!
//! Design: No external dependencies. Handles common patterns safely.
//! Falls back to original text if anything looks wrong.

/// Detect language from content heuristics or code fence
pub fn detect_language(text: &str) -> Option<&'static str> {
    let trimmed = text.trim();

    // Code fence detection
    if trimmed.starts_with("```python") || trimmed.starts_with("```py") {
        return Some("python");
    }
    if trimmed.starts_with("```javascript")
        || trimmed.starts_with("```js")
        || trimmed.starts_with("```typescript")
        || trimmed.starts_with("```ts")
    {
        return Some("javascript");
    }
    if trimmed.starts_with("```bash")
        || trimmed.starts_with("```sh")
        || trimmed.starts_with("```shell")
    {
        return Some("shell");
    }

    // Content heuristics (require multiple signals)
    let py_signals = [
        trimmed.contains("def ") && trimmed.contains(":\n"),
        trimmed.contains("import "),
        trimmed.contains("class ") && trimmed.contains(":\n"),
        trimmed.contains("    self."),
        trimmed.contains("if __name__"),
    ];
    if py_signals.iter().filter(|&&b| b).count() >= 2 {
        return Some("python");
    }

    let js_signals = [
        trimmed.contains("function ") || trimmed.contains("const ") || trimmed.contains("let "),
        trimmed.contains("=>") || trimmed.contains("async "),
        trimmed.contains("require(") || trimmed.contains("import "),
    ];
    if js_signals.iter().filter(|&&b| b).count() >= 2 {
        return Some("javascript");
    }

    None
}

/// Minify Python code: strip comments, docstrings, excess blank lines, normalize indent
pub fn minify_python(code: &str) -> String {
    let mut result = Vec::new();
    let mut in_docstring = false;
    let mut docstring_char: char = '"';
    let mut blank_count = 0;

    let lines: Vec<&str> = code.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // Handle docstring boundaries
        if in_docstring {
            let delim = if docstring_char == '"' {
                "\"\"\""
            } else {
                "'''"
            };
            if trimmed.contains(delim) && (trimmed.ends_with(delim) || trimmed == delim) {
                in_docstring = false;
            }
            i += 1;
            continue;
        }

        // Detect docstring start
        if trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''") {
            docstring_char = trimmed.chars().next().unwrap();
            let delim = if docstring_char == '"' {
                "\"\"\""
            } else {
                "'''"
            };
            // Check if it's a single-line docstring
            if trimmed.len() > 3 && trimmed[3..].contains(delim) {
                // Single-line docstring — skip
                i += 1;
                continue;
            }
            // Multi-line docstring
            in_docstring = true;
            i += 1;
            continue;
        }

        // Skip pure comment lines (but keep shebangs and type: ignore)
        if trimmed.starts_with('#')
            && !trimmed.starts_with("#!")
            && !trimmed.contains("type:")
            && !trimmed.contains("noqa")
            && !trimmed.contains("pylint")
            && !trimmed.contains("pragma")
            && !trimmed.contains("encoding")
        {
            i += 1;
            continue;
        }

        // Collapse blank lines (max 1 consecutive)
        if trimmed.is_empty() {
            blank_count += 1;
            if blank_count <= 1 {
                result.push(String::new());
            }
            i += 1;
            continue;
        }
        blank_count = 0;

        // Strip trailing inline comments (careful: not in strings)
        let clean_line = strip_inline_comment_python(line);

        result.push(clean_line);
        i += 1;
    }

    // Trim trailing blank lines
    while result.last().map(|s| s.trim().is_empty()).unwrap_or(false) {
        result.pop();
    }

    result.join("\n")
}

/// Strip trailing comments from a Python line, respecting string literals
fn strip_inline_comment_python(line: &str) -> String {
    let mut in_single = false;
    let mut in_double = false;
    let mut prev = '\0';

    for (i, ch) in line.char_indices() {
        if ch == '\'' && !in_double && prev != '\\' {
            in_single = !in_single;
        }
        if ch == '"' && !in_single && prev != '\\' {
            in_double = !in_double;
        }

        if ch == '#' && !in_single && !in_double {
            // Check it's not a type hint or pragma
            let rest = &line[i..];
            if rest.contains("type:") || rest.contains("noqa") || rest.contains("pragma") {
                return line.to_string();
            }
            // Strip the comment, preserve trailing content significance
            let before = line[..i].trim_end();
            if !before.is_empty() {
                return before.to_string();
            }
        }
        prev = ch;
    }

    line.to_string()
}

/// Minify JavaScript/TypeScript code: strip comments, excess whitespace
pub fn minify_javascript(code: &str) -> String {
    let mut result = Vec::new();
    let mut in_block_comment = false;
    let mut blank_count = 0;

    for line in code.lines() {
        let trimmed = line.trim();

        // Handle block comments
        if in_block_comment {
            if let Some(pos) = trimmed.find("*/") {
                let after = trimmed[pos + 2..].trim();
                if !after.is_empty() {
                    result.push(after.to_string());
                }
                in_block_comment = false;
            }
            continue;
        }

        // Check for block comment start
        if trimmed.starts_with("/*") {
            if let Some(pos) = trimmed[2..].find("*/") {
                // Single-line block comment
                let after = trimmed[pos + 4..].trim();
                if !after.is_empty() {
                    result.push(after.to_string());
                }
                continue;
            }
            in_block_comment = true;
            continue;
        }

        // Skip single-line comments
        if trimmed.starts_with("//") && !trimmed.starts_with("///") {
            continue;
        }

        // Collapse blanks
        if trimmed.is_empty() {
            blank_count += 1;
            if blank_count <= 1 {
                result.push(String::new());
            }
            continue;
        }
        blank_count = 0;

        // Strip trailing // comments (respecting strings)
        let clean = strip_inline_comment_js(line);
        result.push(clean);
    }

    while result.last().map(|s| s.trim().is_empty()).unwrap_or(false) {
        result.pop();
    }

    result.join("\n")
}

fn strip_inline_comment_js(line: &str) -> String {
    let mut in_single = false;
    let mut in_double = false;
    let mut in_template = false;
    let mut prev = '\0';
    let chars: Vec<char> = line.chars().collect();

    for i in 0..chars.len() {
        let ch = chars[i];
        if ch == '\'' && !in_double && !in_template && prev != '\\' {
            in_single = !in_single;
        }
        if ch == '"' && !in_single && !in_template && prev != '\\' {
            in_double = !in_double;
        }
        if ch == '`' && !in_single && !in_double && prev != '\\' {
            in_template = !in_template;
        }

        if ch == '/'
            && i + 1 < chars.len()
            && chars[i + 1] == '/'
            && !in_single
            && !in_double
            && !in_template
        {
            let before = line[..i].trim_end();
            if !before.is_empty() {
                return before.to_string();
            }
            return String::new();
        }
        prev = ch;
    }

    line.to_string()
}

/// Minify shell scripts: strip comments, blank lines
pub fn minify_shell(code: &str) -> String {
    let mut result = Vec::new();
    let mut blank_count = 0;

    for line in code.lines() {
        let trimmed = line.trim();

        // Keep shebangs
        if trimmed.starts_with("#!") {
            result.push(line.to_string());
            continue;
        }

        // Skip pure comment lines
        if trimmed.starts_with('#') {
            continue;
        }

        if trimmed.is_empty() {
            blank_count += 1;
            if blank_count <= 1 {
                result.push(String::new());
            }
            continue;
        }
        blank_count = 0;

        result.push(line.to_string());
    }

    result.join("\n")
}

/// Top-level entry: detect language and apply appropriate minifier
pub fn minify_code_block(text: &str) -> String {
    let lang = detect_language(text);

    // Extract content from code fences if present
    let (code, prefix, suffix) = if text.trim().starts_with("```") {
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() >= 2 {
            let fence_line = lines[0];
            let end_idx = lines.len()
                - if lines.last().map(|l| l.trim()) == Some("```") {
                    1
                } else {
                    0
                };
            let inner = lines[1..end_idx].join("\n");
            (inner, format!("{}\n", fence_line), "\n```".to_string())
        } else {
            return text.to_string();
        }
    } else {
        (text.to_string(), String::new(), String::new())
    };

    let minified = match lang {
        Some("python") => minify_python(&code),
        Some("javascript") => minify_javascript(&code),
        Some("shell") => minify_shell(&code),
        _ => return text.to_string(), // Unknown language, don't touch
    };

    // Only use if shorter
    if minified.len() < code.len() {
        format!("{}{}{}", prefix, minified, suffix)
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_python_docstring_strip() {
        let code = r#"def hello():
    """This is a docstring.
    It spans multiple lines.
    """
    return "world"
"#;
        let result = minify_python(code);
        assert!(!result.contains("docstring"));
        assert!(result.contains("return \"world\""));
    }

    #[test]
    fn test_python_comment_in_string() {
        let code = "msg = \"# this is not a comment\"\n# this IS a comment\nprint(msg)\n";
        let result = minify_python(code);
        assert!(result.contains("not a comment"));
        assert!(!result.contains("this IS a comment"));
    }

    #[test]
    fn test_js_block_comment() {
        let code = r#"/* This is a 
   block comment */
const x = 42; // inline comment
"#;
        let result = minify_javascript(code);
        assert!(!result.contains("block comment"));
        assert!(!result.contains("inline comment"));
        assert!(result.contains("const x = 42;"));
    }
}
