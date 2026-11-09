#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Rust,
    Python,
    JavaScript,
    Go,
    C,
    Plain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Comment,
    String,
    Keyword,
    Number,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

pub fn language_from_extension(extension: &str) -> Language {
    match extension.to_ascii_lowercase().as_str() {
        "rs" => Language::Rust,
        "py" | "pyw" | "pyi" => Language::Python,
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" => Language::JavaScript,
        "go" => Language::Go,
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => Language::C,
        _ => Language::Plain,
    }
}

pub fn highlight_line(line: &str, language: Language) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < line.len() {
        if let Some(end) = line_comment_end(line, index, language) {
            push(&mut tokens, index, end, TokenKind::Comment);
            break;
        }
        if let Some(end) = block_comment_end(line, index, language) {
            push(&mut tokens, index, end, TokenKind::Comment);
            index = end;
            continue;
        }
        if let Some(end) = string_end(line, index, language) {
            push(&mut tokens, index, end, TokenKind::String);
            index = end;
            continue;
        }
        if language != Language::Plain
            && let Some(end) = number_end(line, index)
        {
            push(&mut tokens, index, end, TokenKind::Number);
            index = end;
            continue;
        }
        if let Some(end) = ident_end(line, index) {
            if is_keyword(language, &line[index..end]) {
                push(&mut tokens, index, end, TokenKind::Keyword);
            }
            index = end;
            continue;
        }
        index += utf8_len(line, index);
    }
    tokens
}

fn push(tokens: &mut Vec<Token>, start: usize, end: usize, kind: TokenKind) {
    if end > start {
        tokens.push(Token { start, end, kind });
    }
}

fn line_comment_end(line: &str, index: usize, language: Language) -> Option<usize> {
    let rest = &line[index..];
    let comment = match language {
        Language::Python => rest.starts_with('#'),
        Language::Plain => rest.starts_with("//") || rest.starts_with('#'),
        Language::Rust | Language::JavaScript | Language::Go | Language::C => {
            rest.starts_with("//")
        }
    };
    comment.then_some(line.len())
}

fn block_comment_end(line: &str, index: usize, language: Language) -> Option<usize> {
    if matches!(language, Language::Python) || !line[index..].starts_with("/*") {
        return None;
    }
    match line[index + 2..].find("*/") {
        Some(rel) => Some(index + 2 + rel + 2),
        None => Some(line.len()),
    }
}

fn string_end(line: &str, index: usize, language: Language) -> Option<usize> {
    let rest = &line[index..];
    match language {
        Language::Python => python_string_end(line, index),
        Language::Rust | Language::C => {
            if rest.starts_with('\'') {
                char_end(line, index)
            } else if rest.starts_with('"') {
                Some(quoted_end(line, index + 1, "\"", true))
            } else {
                None
            }
        }
        Language::Go => {
            if rest.starts_with('\'') {
                char_end(line, index)
            } else if rest.starts_with('"') {
                Some(quoted_end(line, index + 1, "\"", true))
            } else if rest.starts_with('`') {
                Some(quoted_end(line, index + 1, "`", false))
            } else {
                None
            }
        }
        Language::JavaScript | Language::Plain => {
            if rest.starts_with('"') {
                Some(quoted_end(line, index + 1, "\"", true))
            } else if rest.starts_with('\'') {
                Some(quoted_end(line, index + 1, "'", true))
            } else if rest.starts_with('`') {
                Some(quoted_end(line, index + 1, "`", true))
            } else {
                None
            }
        }
    }
}

fn python_string_end(line: &str, index: usize) -> Option<usize> {
    let rest = &line[index..];
    if rest.starts_with("\"\"\"") {
        Some(quoted_end(line, index + 3, "\"\"\"", true))
    } else if rest.starts_with("'''") {
        Some(quoted_end(line, index + 3, "'''", true))
    } else if rest.starts_with('"') {
        Some(quoted_end(line, index + 1, "\"", true))
    } else if rest.starts_with('\'') {
        Some(quoted_end(line, index + 1, "'", true))
    } else {
        None
    }
}

fn quoted_end(line: &str, mut index: usize, close: &str, escapes: bool) -> usize {
    while index < line.len() {
        if escapes && line[index..].starts_with('\\') {
            index += 1;
            if index < line.len() {
                index += utf8_len(line, index);
            }
            continue;
        }
        if line[index..].starts_with(close) {
            return index + close.len();
        }
        index += utf8_len(line, index);
    }
    line.len()
}

fn char_end(line: &str, index: usize) -> Option<usize> {
    let rest = &line[index + 1..];
    let mut chars = rest.char_indices();
    let (_, first) = chars.next()?;
    if first == '\'' || first == '\n' || first == '\r' {
        return None;
    }
    let after = if first == '\\' {
        let (offset, ch) = chars.next()?;
        index + 1 + offset + ch.len_utf8()
    } else {
        index + 1 + first.len_utf8()
    };
    line[after..].starts_with('\'').then_some(after + 1)
}

fn number_end(line: &str, index: usize) -> Option<usize> {
    let bytes = line.as_bytes();
    if index >= bytes.len() {
        return None;
    }
    let mut cursor = index;
    if bytes[cursor] == b'.' {
        if cursor + 1 < bytes.len() && bytes[cursor + 1].is_ascii_digit() {
            cursor += 1;
        } else {
            return None;
        }
    } else if bytes[cursor].is_ascii_digit() {
        if bytes[cursor] == b'0' && cursor + 1 < bytes.len() {
            let base = bytes[cursor + 1];
            let digit = |byte: u8| match base {
                b'x' | b'X' => byte.is_ascii_hexdigit() || byte == b'_',
                b'b' | b'B' => matches!(byte, b'0' | b'1' | b'_'),
                b'o' | b'O' => matches!(byte, b'0'..=b'7' | b'_'),
                _ => false,
            };
            if matches!(base, b'x' | b'X' | b'b' | b'B' | b'o' | b'O')
                && cursor + 2 < bytes.len()
                && digit(bytes[cursor + 2])
                && bytes[cursor + 2] != b'_'
            {
                cursor += 2;
                while cursor < bytes.len() && digit(bytes[cursor]) {
                    cursor += 1;
                }
                return Some(cursor);
            }
        }
    } else {
        return None;
    }
    cursor = take_digits(bytes, cursor);
    if cursor < bytes.len()
        && bytes[cursor] == b'.'
        && cursor + 1 < bytes.len()
        && bytes[cursor + 1].is_ascii_digit()
        && !line[index..].starts_with('.')
    {
        cursor += 1;
        cursor = take_digits(bytes, cursor);
    }
    if cursor < bytes.len() && matches!(bytes[cursor], b'e' | b'E') {
        let mut exponent = cursor + 1;
        if exponent < bytes.len() && matches!(bytes[exponent], b'+' | b'-') {
            exponent += 1;
        }
        if exponent < bytes.len() && bytes[exponent].is_ascii_digit() {
            cursor = take_digits(bytes, exponent);
        }
    }
    (cursor > index).then_some(cursor)
}

fn take_digits(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b'_') {
        index += 1;
    }
    index
}

fn ident_end(line: &str, index: usize) -> Option<usize> {
    let bytes = line.as_bytes();
    if index >= bytes.len() || !is_ident_start(bytes[index]) {
        return None;
    }
    let mut cursor = index + 1;
    while cursor < bytes.len() && is_ident_continue(bytes[cursor]) {
        cursor += 1;
    }
    Some(cursor)
}

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_ident_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn utf8_len(line: &str, index: usize) -> usize {
    line[index..]
        .chars()
        .next()
        .map(|ch| ch.len_utf8())
        .unwrap_or(1)
}

fn is_keyword(language: Language, word: &str) -> bool {
    match language {
        Language::Rust => matches!(
            word,
            "as" | "async"
                | "await"
                | "break"
                | "const"
                | "continue"
                | "crate"
                | "dyn"
                | "else"
                | "enum"
                | "extern"
                | "false"
                | "fn"
                | "for"
                | "if"
                | "impl"
                | "in"
                | "let"
                | "loop"
                | "match"
                | "mod"
                | "move"
                | "mut"
                | "pub"
                | "ref"
                | "return"
                | "self"
                | "Self"
                | "static"
                | "struct"
                | "super"
                | "trait"
                | "true"
                | "type"
                | "unsafe"
                | "use"
                | "where"
                | "while"
        ),
        Language::Python => matches!(
            word,
            "False"
                | "None"
                | "True"
                | "and"
                | "as"
                | "assert"
                | "async"
                | "await"
                | "break"
                | "class"
                | "continue"
                | "def"
                | "del"
                | "elif"
                | "else"
                | "except"
                | "finally"
                | "for"
                | "from"
                | "global"
                | "if"
                | "import"
                | "in"
                | "is"
                | "lambda"
                | "nonlocal"
                | "not"
                | "or"
                | "pass"
                | "raise"
                | "return"
                | "try"
                | "while"
                | "with"
                | "yield"
        ),
        Language::JavaScript => matches!(
            word,
            "as" | "async"
                | "await"
                | "break"
                | "case"
                | "catch"
                | "class"
                | "const"
                | "continue"
                | "default"
                | "else"
                | "enum"
                | "export"
                | "extends"
                | "false"
                | "finally"
                | "for"
                | "from"
                | "function"
                | "if"
                | "implements"
                | "import"
                | "in"
                | "interface"
                | "let"
                | "new"
                | "null"
                | "of"
                | "return"
                | "static"
                | "super"
                | "switch"
                | "this"
                | "throw"
                | "true"
                | "try"
                | "type"
                | "typeof"
                | "undefined"
                | "var"
                | "void"
                | "while"
                | "yield"
        ),
        Language::Go => matches!(
            word,
            "break"
                | "case"
                | "chan"
                | "const"
                | "continue"
                | "default"
                | "defer"
                | "else"
                | "fallthrough"
                | "false"
                | "for"
                | "func"
                | "go"
                | "goto"
                | "if"
                | "import"
                | "interface"
                | "map"
                | "nil"
                | "package"
                | "range"
                | "return"
                | "select"
                | "struct"
                | "switch"
                | "true"
                | "type"
                | "var"
        ),
        Language::C => matches!(
            word,
            "auto"
                | "bool"
                | "break"
                | "case"
                | "catch"
                | "char"
                | "class"
                | "const"
                | "continue"
                | "default"
                | "delete"
                | "do"
                | "double"
                | "else"
                | "enum"
                | "explicit"
                | "extern"
                | "false"
                | "float"
                | "for"
                | "friend"
                | "goto"
                | "if"
                | "inline"
                | "int"
                | "long"
                | "namespace"
                | "new"
                | "nullptr"
                | "operator"
                | "private"
                | "protected"
                | "public"
                | "return"
                | "short"
                | "signed"
                | "sizeof"
                | "static"
                | "struct"
                | "switch"
                | "template"
                | "this"
                | "throw"
                | "true"
                | "try"
                | "typedef"
                | "typename"
                | "union"
                | "unsigned"
                | "using"
                | "virtual"
                | "void"
                | "volatile"
                | "while"
        ),
        Language::Plain => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_keyword_and_string_do_not_overlap() {
        let line = r#"let name = "hyperion";"#;
        let tokens = highlight_line(line, Language::Rust);
        let keyword = tokens
            .iter()
            .find(|token| {
                token.kind == TokenKind::Keyword && &line[token.start..token.end] == "let"
            })
            .expect("keyword");
        let string = tokens
            .iter()
            .find(|token| token.kind == TokenKind::String)
            .expect("string");
        assert_eq!(keyword.kind, TokenKind::Keyword);
        assert_eq!(string.kind, TokenKind::String);
        assert_ne!(keyword.kind, string.kind);
        assert_eq!(&line[string.start..string.end], "\"hyperion\"");
        assert!(keyword.end <= string.start);
    }
}
