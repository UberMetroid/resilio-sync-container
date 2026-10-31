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
            JsonValue::Object(entries) => Some(entries),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            JsonValue::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            JsonValue::Number(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            JsonValue::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

pub fn strip_json_comments(input: &str) -> Result<String, String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let mut in_str = false;

    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            out.push(b);
            if b == b'\\' && i + 1 < bytes.len() {
                out.push(bytes[i + 1]);
                i += 2;
                continue;
            } else if b == b'"' {
                in_str = false;
            }
            i += 1;
        } else if b == b'"' {
            in_str = true;
            out.push(b);
            i += 1;
        } else if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
            i += 2;
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
}

impl<'a> Parser<'a> {
    fn peek(&mut self) -> Option<u8> {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
        self.bytes.get(self.pos).copied()
    }

    fn parse_value(&mut self) -> Result<JsonValue, String> {
        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => self.parse_string().map(JsonValue::Str),
            Some(b't') if self.bytes[self.pos..].starts_with(b"true") => {
                self.pos += 4;
                Ok(JsonValue::Bool(true))
            }
            Some(b'f') if self.bytes[self.pos..].starts_with(b"false") => {
                self.pos += 5;
                Ok(JsonValue::Bool(false))
            }
            Some(b'n') if self.bytes[self.pos..].starts_with(b"null") => {
                self.pos += 4;
                Ok(JsonValue::Null)
            }
            Some(b'-') | Some(b'0'..=b'9') => self.parse_number(),
            Some(c) => Err(format!(
                "Unexpected token '{}' at byte {}",
                c as char, self.pos
            )),
            None => Err("Unexpected end of JSON input".to_string()),
        }
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
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
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

    fn parse_object(&mut self) -> Result<JsonValue, String> {
        self.pos += 1;
        let mut entries: Vec<(String, JsonValue)> = Vec::new();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(JsonValue::Object(entries));
        }
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
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    if self.peek() == Some(b'}') {
                        return Err("Trailing comma in JSON object is invalid".to_string());
                    }
                }
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(JsonValue::Object(entries));
                }
                _ => return Err("Expected ',' or '}' in JSON object".to_string()),
            }
        }
    }

    fn parse_array(&mut self) -> Result<JsonValue, String> {
        self.pos += 1;
        let mut items = Vec::new();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(JsonValue::Array(items));
        }
        loop {
            items.push(self.parse_value()?);
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    if self.peek() == Some(b']') {
                        return Err("Trailing comma in JSON array is invalid".to_string());
                    }
                }
                Some(b']') => {
                    self.pos += 1;
                    return Ok(JsonValue::Array(items));
                }
                _ => return Err("Expected ',' or ']' in JSON array".to_string()),
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
    let mut parser = Parser {
        bytes: stripped.as_bytes(),
        pos: 0,
    };
    let root = parser.parse_value()?;
    if parser.peek().is_some() {
        return Err(format!("Unexpected trailing data at byte {}", parser.pos));
    }
    Ok(root)
}
