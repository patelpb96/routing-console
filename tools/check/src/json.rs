//! A small RFC 8259 validator: is this byte string exactly one JSON value (plus whitespace)?

const MAX_DEPTH: usize = 512;

/// `Ok(())` when `text` is one complete JSON value surrounded only by JSON whitespace.
pub fn validate(text: &[u8]) -> Result<(), String> {
    if std::str::from_utf8(text).is_err() {
        return Err("not valid UTF-8".into());
    }
    let mut p = Parser { b: text, i: 0 };
    p.ws();
    p.value(0)?;
    p.ws();
    if p.i != p.b.len() {
        return Err(format!(
            "unexpected text after the value at byte {}",
            p.i + 1
        ));
    }
    Ok(())
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn err<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("{what} at byte {}", self.i + 1))
    }

    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn value(&mut self, depth: usize) -> Result<(), String> {
        if depth > MAX_DEPTH {
            return self.err("nesting too deep");
        }
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string(),
            Some(b't') => self.literal(b"true"),
            Some(b'f') => self.literal(b"false"),
            Some(b'n') => self.literal(b"null"),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => self.err("unexpected character"),
            None => self.err("unexpected end of input"),
        }
    }

    fn literal(&mut self, word: &[u8]) -> Result<(), String> {
        if self.b[self.i..].starts_with(word) {
            self.i += word.len();
            Ok(())
        } else {
            self.err("invalid literal")
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.i;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.i += 1;
        }
        self.i - start
    }

    fn number(&mut self) -> Result<(), String> {
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        match self.peek() {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return self.err("invalid number"),
        }
        if self.peek() == Some(b'.') {
            self.i += 1;
            if self.digits() == 0 {
                return self.err("invalid fraction");
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if self.digits() == 0 {
                return self.err("invalid exponent");
            }
        }
        Ok(())
    }

    fn string(&mut self) -> Result<(), String> {
        self.i += 1; // opening quote
        loop {
            match self.peek() {
                None => return self.err("unterminated string"),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(());
                }
                Some(b'\\') => {
                    self.i += 1;
                    match self.peek() {
                        Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => self.i += 1,
                        Some(b'u') => {
                            self.i += 1;
                            for _ in 0..4 {
                                if !matches!(self.peek(), Some(c) if c.is_ascii_hexdigit()) {
                                    return self.err("invalid \\u escape");
                                }
                                self.i += 1;
                            }
                        }
                        _ => return self.err("invalid escape"),
                    }
                }
                Some(c) if c < 0x20 => return self.err("control character in string"),
                Some(_) => self.i += 1,
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<(), String> {
        self.i += 1;
        self.ws();
        if self.peek() == Some(b']') {
            self.i += 1;
            return Ok(());
        }
        loop {
            self.ws();
            self.value(depth + 1)?;
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(());
                }
                _ => return self.err("expected , or ]"),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<(), String> {
        self.i += 1;
        self.ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Ok(());
        }
        loop {
            self.ws();
            if self.peek() != Some(b'"') {
                return self.err("expected a string key");
            }
            self.string()?;
            self.ws();
            if self.peek() != Some(b':') {
                return self.err("expected :");
            }
            self.i += 1;
            self.ws();
            self.value(depth + 1)?;
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(());
                }
                _ => return self.err("expected , or }"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::validate;

    fn ok(s: &str) -> bool {
        validate(s.as_bytes()).is_ok()
    }

    #[test]
    fn accepts_valid_values() {
        for s in [
            "{}",
            "[]",
            "42",
            "-0.5e+10",
            "\"text\"",
            "true",
            "null",
            " {\"a\": [1, 2, {\"b\": null}], \"c\": \"\\u00e9\\n\"} ",
            "{\"emoji\":\"\u{1F600}\"}",
        ] {
            assert!(ok(s), "{s}");
        }
    }

    #[test]
    fn rejects_invalid_values() {
        for s in [
            "",
            "{",
            "{}}",
            "{} {}",
            "[1,]",
            "{\"a\":1,}",
            "{'a':1}",
            "01",
            "1.",
            ".5",
            "+1",
            "1e",
            "tru",
            "NaN",
            "\"a\tb\"",
            "\"\\x\"",
            "\"\\u12\"",
            "{a:1}",
            "\u{feff}{}",
        ] {
            assert!(!ok(s), "{s:?}");
        }
        assert!(validate(&[b'"', 0xff, b'"']).is_err());
    }

    #[test]
    fn deep_nesting_is_bounded() {
        let deep = "[".repeat(10_000) + &"]".repeat(10_000);
        assert!(!ok(&deep));
        let fine = "[".repeat(100) + &"]".repeat(100);
        assert!(ok(&fine));
    }
}
