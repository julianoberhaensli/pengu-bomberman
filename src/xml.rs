//! The small XML subset of the original XMLSocket protocol: one element per frame, attributes in quotes.

const MAX_DEPTH: usize = 6;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Element>,
    pub text: String,
}

impl Element {
    pub fn new(name: &str) -> Self {
        Self { name: name.to_string(), ..Default::default() }
    }

    pub fn attr(mut self, key: &str, value: impl ToString) -> Self {
        self.attrs.push((key.to_string(), value.to_string()));
        self
    }

    pub fn child(mut self, c: Element) -> Self {
        self.children.push(c);
        self
    }

    pub fn text(mut self, t: &str) -> Self {
        self.text = t.to_string();
        self
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub fn to_xml(&self) -> String {
        let mut s = format!("<{}", self.name);
        for (k, v) in &self.attrs {
            s.push_str(&format!(" {k}=\"{}\"", escape(v)));
        }
        if self.children.is_empty() && self.text.is_empty() {
            s.push_str("/>");
        } else {
            s.push('>');
            s.push_str(&escape(&self.text));
            for c in &self.children {
                s.push_str(&c.to_xml());
            }
            s.push_str(&format!("</{}>", self.name));
        }
        s
    }

    /// Parses exactly one element (surrounding whitespace allowed).
    pub fn parse(src: &str) -> Option<Element> {
        let mut p = Parser { src, i: 0 };
        p.skip_ws();
        let e = p.element(0)?;
        p.skip_ws();
        (p.i == src.len()).then_some(e)
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

struct Parser<'a> {
    src: &'a str,
    i: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.src.as_bytes().get(self.i).copied()
    }

    fn rest(&self) -> &'a str {
        &self.src[self.i..]
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.i += 1;
        }
    }

    fn name(&mut self) -> Option<&'a str> {
        let start = self.i;
        while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b':' | b'.')) {
            self.i += 1;
        }
        (self.i > start).then(|| &self.src[start..self.i])
    }

    fn expect(&mut self, c: u8) -> Option<()> {
        (self.peek() == Some(c)).then(|| self.i += 1)
    }

    fn element(&mut self, depth: usize) -> Option<Element> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.expect(b'<')?;
        let mut e = Element::new(self.name()?);
        loop {
            self.skip_ws();
            match self.peek()? {
                b'/' => {
                    self.i += 1;
                    self.expect(b'>')?;
                    return Some(e);
                }
                b'>' => {
                    self.i += 1;
                    break;
                }
                _ => {
                    let key = self.name()?;
                    self.skip_ws();
                    self.expect(b'=')?;
                    self.skip_ws();
                    let quote = self.peek()?;
                    if quote != b'"' && quote != b'\'' {
                        return None;
                    }
                    self.i += 1;
                    let end = self.rest().find(quote as char)?;
                    e.attrs.push((key.to_string(), unescape(&self.rest()[..end])));
                    self.i += end + 1;
                }
            }
        }
        let mut text = String::new();
        loop {
            let lt = self.rest().find('<')?;
            text.push_str(&self.rest()[..lt]);
            self.i += lt;
            if self.rest().starts_with("</") {
                self.i += 2;
                if self.name()? != e.name {
                    return None;
                }
                self.skip_ws();
                self.expect(b'>')?;
                e.text = unescape(text.trim());
                return Some(e);
            }
            e.children.push(self.element(depth + 1)?);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_numeric_names_and_extra_spaces_like_the_original_messages() {
        let e = Element::parse("<12 x=\"120\"  y=\"40\" c=\"0100\"/>\n").unwrap();
        assert_eq!(e.name, "12");
        assert_eq!(e.get("x"), Some("120"));
        assert_eq!(e.get("y"), Some("40"));
        assert_eq!(e.get("c"), Some("0100"));
        assert_eq!(e.get("z"), None);
        assert!(Element::parse("<14 p=\"20\" f=\"40\" />").is_some());
    }

    #[test]
    fn parses_children() {
        let e = Element::parse("<userList><user name=\"a\"/><user name=\"b\" skill=\"1/2/3\"></user></userList>").unwrap();
        assert_eq!(e.children.len(), 2);
        assert_eq!(e.children[1].get("skill"), Some("1/2/3"));
    }

    #[test]
    fn roundtrip_escapes_special_characters() {
        let e = Element::new("msgAll").attr("name", "a\"b").attr("msg", "x < y & z");
        let xml = e.to_xml();
        assert!(!xml.contains("x < y"));
        assert_eq!(Element::parse(&xml).unwrap(), e);
    }

    #[test]
    fn rejects_malformed_input() {
        for bad in ["", "x", "<a", "<a b=1/>", "<a></b>", "<a/><b/>", "<a b=\"1/>", "<>", "<a><b></a>"] {
            assert!(Element::parse(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn nesting_depth_is_limited() {
        let deep = "<a>".repeat(50) + &"</a>".repeat(50);
        assert!(Element::parse(&deep).is_none());
    }
}
