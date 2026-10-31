//! String-aware comment stripper and strict recursive-descent JSON parser for Resilio Sync v3.

#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Object(Vec<(String, JsonValue)>),
    Array(Vec<JsonValue>),
    Str(String),
    Number(i64),
    Bool(bool),
    Null,
}

impl JsonValue {
    pub fn as_object(&self) -> Option<&[(String, JsonValue)]> {
        match self {
            Self::Object(e) => Some(e),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Number(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

pub fn strip_json_comments(input: &str) -> Result<String, String> {
    let bytes = input.as_bytes();
    let (mut out, mut i, mut in_str) = (Vec::with_capacity(bytes.len()), 0, false);
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            out.push(b);
            if b == b'\\' && i + 1 < bytes.len() {
                out.push(bytes[i + 1]);
                i += 2;
                continue;
            }
            in_str = b != b'"';
            i += 1;
        } else if (b == b'/' && bytes.get(i + 1) == Some(&b'/')) || b == b'#' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            let mut closed = false;
            while i + 1 < bytes.len() {
                if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    i += 2;
                    closed = true;
                    break;
                }
                if bytes[i] == b'\n' {
                    out.push(b'\n');
                }
                i += 1;
            }
            if !closed {
                return Err("Unterminated /* ... */ block comment in sync.conf".to_string());
            }
        } else {
            in_str = b == b'"';
            out.push(b);
            i += 1;
        }
    }
    if in_str {
        return Err("Unterminated string literal in sync.conf".to_string());
    }
    String::from_utf8(out).map_err(|e| format!("Invalid UTF-8 in sync.conf: {e}"))
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn peek(&mut self) -> Option<u8> {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
        self.bytes.get(self.pos).copied()
    }

    fn parse_value(&mut self) -> Result<JsonValue, String> {
        if self.depth > 64 {
            return Err("JSON nesting depth exceeds maximum limit (64)".to_string());
        }
        let tok = self.peek();
        let rest = &self.bytes[self.pos..];
        let (n, val) = match tok {
            Some(b'{') => return self.parse_object(),
            Some(b'[') => return self.parse_array(),
            Some(b'"') => return self.parse_string().map(JsonValue::Str),
            Some(b't') if rest.starts_with(b"true") => (4, JsonValue::Bool(true)),
            Some(b'f') if rest.starts_with(b"false") => (5, JsonValue::Bool(false)),
            Some(b'n') if rest.starts_with(b"null") => (4, JsonValue::Null),
            Some(b'-' | b'0'..=b'9') => return self.parse_number(),
            Some(c) => return Err(format!("Unexpected token '{}' at {}", c as char, self.pos)),
            None => return Err("Unexpected end of JSON input".to_string()),
        };
        self.pos += n;
        Ok(val)
    }

    fn parse_string(&mut self) -> Result<String, String> {
        if self.peek() != Some(b'"') {
            return Err(format!("Expected '\"' at byte {}", self.pos));
        }
        self.pos += 1;
        let mut out = Vec::new();
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            self.pos += 1;
            if b == b'"' {
                return String::from_utf8(out).map_err(|e| format!("Invalid UTF-8 string: {e}"));
            } else if b == b'\\' {
                let esc = *self.bytes.get(self.pos).ok_or("Untruncated escape")?;
                self.pos += 1;
                match esc {
                    b'"' | b'\\' | b'/' => out.push(esc),
                    b'b' | b'f' => out.push(if esc == b'b' { 0x08 } else { 0x0C }),
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
                    b'u' => {
                        let hex = self.bytes.get(self.pos..self.pos + 4).ok_or("Bad \\u")?;
                        let ch = std::str::from_utf8(hex)
                            .ok()
                            .and_then(|s| u32::from_str_radix(s, 16).ok())
                            .and_then(char::from_u32)
                            .ok_or("Invalid \\uXXXX escape")?;
                        self.pos += 4;
                        out.extend_from_slice(ch.to_string().as_bytes());
                    }
                    _ => return Err(format!("Unsupported JSON escape '\\{}'", esc as char)),
                }
            } else if b < 0x20 {
                return Err("Unescaped control character in JSON string".to_string());
            } else {
                out.push(b);
            }
        }
        Err("Unterminated JSON string".to_string())
    }

    fn step_delim(&mut self, close: u8, kind: &str) -> Result<bool, String> {
        match self.peek() {
            Some(b',') => {
                self.pos += 1;
                if self.peek() == Some(close) {
                    return Err(format!("Trailing comma in JSON {kind} is invalid"));
                }
                Ok(true)
            }
            Some(c) if c == close => {
                self.pos += 1;
                Ok(false)
            }
            _ => Err(format!("Expected ',' or closing delimiter in JSON {kind}")),
        }
    }

    fn parse_object(&mut self) -> Result<JsonValue, String> {
        self.pos += 1;
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(JsonValue::Object(Vec::new()));
        }
        self.depth += 1;
        let mut entries = Vec::new();
        loop {
            let key = self.parse_string()?;
            if entries.iter().any(|(k, _)| k == &key) {
                return Err(format!("Duplicate JSON object key '{key}'"));
            }
            if self.peek() != Some(b':') {
                return Err(format!("Expected ':' after key '{key}'"));
            }
            self.pos += 1;
            entries.push((key, self.parse_value()?));
            if !self.step_delim(b'}', "object")? {
                self.depth -= 1;
                return Ok(JsonValue::Object(entries));
            }
        }
    }

    fn parse_array(&mut self) -> Result<JsonValue, String> {
        self.pos += 1;
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(JsonValue::Array(Vec::new()));
        }
        self.depth += 1;
        let mut items = Vec::new();
        loop {
            items.push(self.parse_value()?);
            if !self.step_delim(b']', "array")? {
                self.depth -= 1;
                return Ok(JsonValue::Array(items));
            }
        }
    }

    fn parse_number(&mut self) -> Result<JsonValue, String> {
        let start = self.pos;
        if self.bytes.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }
        let digit_start = self.pos;
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        let digits = &self.bytes[digit_start..self.pos];
        if digits.len() > 1 && digits[0] == b'0' {
            return Err("Leading zeros are not permitted in JSON integers".to_string());
        }
        let s = std::str::from_utf8(&self.bytes[start..self.pos]).map_err(|_| "Bad number")?;
        s.parse::<i64>()
            .map(JsonValue::Number)
            .map_err(|e| format!("Invalid integer '{s}': {e}"))
    }
}

pub fn parse_sync_conf_json(raw: &str) -> Result<JsonValue, String> {
    let stripped = strip_json_comments(raw)?;
    let bytes = stripped.as_bytes();
    let mut parser = Parser {
        bytes,
        pos: 0,
        depth: 0,
    };
    let root = parser.parse_value()?;
    if parser.peek().is_some() {
        return Err(format!("Unexpected trailing data at byte {}", parser.pos));
    }
    Ok(root)
}
