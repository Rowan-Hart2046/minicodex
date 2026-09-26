use std::collections::BTreeMap;
pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(BTreeMap<String, Json>),
}

impl Json {
    pub fn object(fields: Vec<(&str, Json)>) -> Self {
        Json::Object(fields.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }
    pub fn parse(source: &str) -> Result<Self> {
        let mut p = Parser {
            bytes: source.as_bytes(),
            pos: 0,
        };
        let value = p.value()?;
        p.space();
        if p.pos == p.bytes.len() {
            Ok(value)
        } else {
            Err("trailing JSON data".into())
        }
    }
    pub fn get(&self, key: &str) -> Result<&Json> {
        match self {
            Json::Object(m) => m
                .get(key)
                .ok_or_else(|| format!("missing JSON field: {key}")),
            _ => Err("expected JSON object".into()),
        }
    }
    pub fn get_str(&self, key: &str) -> Result<&str> {
        self.get(key)?.as_str()
    }
    pub fn as_str(&self) -> Result<&str> {
        if let Json::String(v) = self {
            Ok(v)
        } else {
            Err("expected JSON string".into())
        }
    }
    pub fn as_array(&self) -> Result<&[Json]> {
        if let Json::Array(v) = self {
            Ok(v)
        } else {
            Err("expected JSON array".into())
        }
    }
    pub fn stringify(&self) -> String {
        match self {
            Json::Null => "null".into(),
            Json::Bool(v) => v.to_string(),
            Json::Number(v) => v.to_string(),
            Json::String(v) => format!("\"{}\"", escape(v)),
            Json::Array(v) => format!(
                "[{}]",
                v.iter().map(Json::stringify).collect::<Vec<_>>().join(",")
            ),
            Json::Object(v) => format!(
                "{{{}}}",
                v.iter()
                    .map(|(k, v)| format!("\"{}\":{}", escape(k), v.stringify()))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        }
    }
}

fn escape(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if c < ' ' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl Parser<'_> {
    fn space(&mut self) {
        while self.peek().is_some_and(u8::is_ascii_whitespace) {
            self.pos += 1;
        }
    }
    fn peek(&self) -> Option<&u8> {
        self.bytes.get(self.pos)
    }
    fn take(&mut self) -> Result<u8> {
        let b = *self.peek().ok_or("unexpected end of JSON")?;
        self.pos += 1;
        Ok(b)
    }
    fn expect(&mut self, b: u8) -> Result<()> {
        if self.take()? == b {
            Ok(())
        } else {
            Err(format!("expected '{}'", b as char))
        }
    }
    fn value(&mut self) -> Result<Json> {
        self.space();
        match self.peek().copied() {
            Some(b'n') => {
                self.literal(b"null")?;
                Ok(Json::Null)
            }
            Some(b't') => {
                self.literal(b"true")?;
                Ok(Json::Bool(true))
            }
            Some(b'f') => {
                self.literal(b"false")?;
                Ok(Json::Bool(false))
            }
            Some(b'"') => Ok(Json::String(self.string()?)),
            Some(b'[') => self.array(),
            Some(b'{') => self.object(),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err("invalid JSON value".into()),
        }
    }
    fn literal(&mut self, v: &[u8]) -> Result<()> {
        if self.bytes.get(self.pos..self.pos + v.len()) == Some(v) {
            self.pos += v.len();
            Ok(())
        } else {
            Err("invalid JSON literal".into())
        }
    }
    fn string(&mut self) -> Result<String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            match self.take()? {
                b'"' => return Ok(out),
                b'\\' => match self.take()? {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{08}'),
                    b'f' => out.push('\u{0c}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let hi = self.hex()?;
                        if (0xD800..=0xDBFF).contains(&hi) {
                            self.expect(b'\\')?;
                            self.expect(b'u')?;
                            let lo = self.hex()?;
                            if !(0xDC00..=0xDFFF).contains(&lo) {
                                return Err("invalid Unicode surrogate".into());
                            }
                            out.push(
                                char::from_u32(0x10000 + ((hi - 0xD800) << 10) + lo - 0xDC00)
                                    .ok_or("invalid Unicode")?,
                            );
                        } else {
                            out.push(char::from_u32(hi).ok_or("invalid Unicode")?);
                        }
                    }
                    _ => return Err("invalid JSON escape".into()),
                },
                b if b < 0x20 => return Err("control character in JSON string".into()),
                b if b < 0x80 => out.push(b as char),
                _ => {
                    self.pos -= 1;
                    let rest =
                        std::str::from_utf8(&self.bytes[self.pos..]).map_err(|e| e.to_string())?;
                    let ch = rest.chars().next().ok_or("invalid UTF-8")?;
                    self.pos += ch.len_utf8();
                    out.push(ch);
                }
            }
        }
    }
    fn hex(&mut self) -> Result<u32> {
        let s = self
            .bytes
            .get(self.pos..self.pos + 4)
            .ok_or("short Unicode escape")?;
        self.pos += 4;
        u32::from_str_radix(std::str::from_utf8(s).map_err(|e| e.to_string())?, 16)
            .map_err(|_| "invalid Unicode escape".into())
    }
    fn array(&mut self) -> Result<Json> {
        self.expect(b'[')?;
        let mut v = vec![];
        self.space();
        if self.peek() == Some(&b']') {
            self.pos += 1;
            return Ok(Json::Array(v));
        }
        loop {
            v.push(self.value()?);
            self.space();
            match self.take()? {
                b']' => break,
                b',' => {}
                _ => return Err("expected ',' or ']'".into()),
            }
        }
        Ok(Json::Array(v))
    }
    fn object(&mut self) -> Result<Json> {
        self.expect(b'{')?;
        let mut v = BTreeMap::new();
        self.space();
        if self.peek() == Some(&b'}') {
            self.pos += 1;
            return Ok(Json::Object(v));
        }
        loop {
            self.space();
            let key = self.string()?;
            self.space();
            self.expect(b':')?;
            v.insert(key, self.value()?);
            self.space();
            match self.take()? {
                b'}' => break,
                b',' => {}
                _ => return Err("expected ',' or '}'".into()),
            }
        }
        Ok(Json::Object(v))
    }
    fn number(&mut self) -> Result<Json> {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|b| matches!(b, b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'))
        {
            self.pos += 1;
        }
        let s = std::str::from_utf8(&self.bytes[start..self.pos]).map_err(|e| e.to_string())?;
        Ok(Json::Number(s.parse().map_err(|_| "invalid JSON number")?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip() {
        let v = Json::parse(r#"{"text":"你好\nworld","ok":true,"items":[1,null]}"#).unwrap();
        assert_eq!(Json::parse(&v.stringify()).unwrap(), v);
    }
    #[test]
    fn unicode_escape() {
        assert_eq!(
            Json::parse(r#""\ud83d\ude80""#).unwrap(),
            Json::String("🚀".into())
        );
    }
}
