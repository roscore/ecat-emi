//! Minimal S-expression tokenizer + iterative parser (no recursion, so deep input cannot overflow).

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Atom(String, u32),
    List(Vec<Node>, u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub line: u32,
    pub msg: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.msg)
    }
}

pub fn err<T>(line: u32, msg: impl Into<String>) -> Result<T, ParseError> {
    Err(ParseError { line, msg: msg.into() })
}

const MAX_DEPTH: usize = 512;

/// Parse exactly one top-level list (trailing whitespace allowed).
pub fn parse(src: &str) -> Result<Node, ParseError> {
    let b = src.as_bytes();
    let mut i = 0;
    let mut line = 1u32;
    // Stack of open lists: (children, line where opened).
    let mut stack: Vec<(Vec<Node>, u32)> = Vec::new();
    let mut root: Option<Node> = None;
    while i < b.len() {
        let c = b[i];
        match c {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b' ' | b'\t' | b'\r' => i += 1,
            b'(' => {
                if root.is_some() {
                    return err(line, "content after the top-level list");
                }
                if stack.len() >= MAX_DEPTH {
                    return err(line, "nesting too deep");
                }
                stack.push((Vec::new(), line));
                i += 1;
            }
            b')' => {
                let Some((kids, l0)) = stack.pop() else { return err(line, "unbalanced ')'") };
                let node = Node::List(kids, l0);
                match stack.last_mut() {
                    Some(top) => top.0.push(node),
                    None => root = Some(node),
                }
                i += 1;
            }
            b'"' => {
                let l0 = line;
                i += 1;
                let mut s = Vec::new();
                loop {
                    let Some(&ch) = b.get(i) else { return err(l0, "unterminated string") };
                    i += 1;
                    match ch {
                        b'"' => break,
                        b'\\' => {
                            let Some(&e) = b.get(i) else { return err(l0, "unterminated string") };
                            i += 1;
                            s.push(match e {
                                b'n' => b'\n',
                                b't' => b'\t',
                                other => other,
                            });
                        }
                        b'\n' => {
                            line += 1;
                            s.push(ch);
                        }
                        _ => s.push(ch),
                    }
                }
                let atom = Node::Atom(String::from_utf8_lossy(&s).into_owned(), l0);
                match stack.last_mut() {
                    Some(top) => top.0.push(atom),
                    None => return err(l0, "atom outside a list"),
                }
            }
            _ => {
                let st = i;
                while i < b.len() && !matches!(b[i], b' ' | b'\t' | b'\r' | b'\n' | b'(' | b')' | b'"') {
                    i += 1;
                }
                let atom = Node::Atom(String::from_utf8_lossy(&b[st..i]).into_owned(), line);
                match stack.last_mut() {
                    Some(top) => top.0.push(atom),
                    None => return err(line, "atom outside a list"),
                }
            }
        }
    }
    if let Some((_, l0)) = stack.last() {
        return err(*l0, "unclosed '('");
    }
    root.ok_or(ParseError { line, msg: "empty input".into() })
}

impl Node {
    pub fn line(&self) -> u32 {
        match self {
            Node::Atom(_, l) | Node::List(_, l) => *l,
        }
    }
    pub fn atom(&self) -> Option<&str> {
        match self {
            Node::Atom(s, _) => Some(s),
            _ => None,
        }
    }
    pub fn items(&self) -> &[Node] {
        match self {
            Node::List(v, _) => v,
            _ => &[],
        }
    }
    /// Head symbol of a list.
    pub fn head(&self) -> Option<&str> {
        self.items().first().and_then(Node::atom)
    }
    /// Child lists with the given head.
    pub fn children<'a>(&'a self, head: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.items().iter().filter(move |n| n.head() == Some(head))
    }
    pub fn child(&self, head: &str) -> Option<&Node> {
        self.items().iter().find(|n| n.head() == Some(head))
    }
    /// The k-th element (0 = head).
    pub fn arg(&self, k: usize) -> Option<&Node> {
        self.items().get(k)
    }
    pub fn arg_str(&self, k: usize) -> Option<&str> {
        self.arg(k).and_then(Node::atom)
    }
    pub fn num(&self, k: usize) -> Result<f64, ParseError> {
        let a = self.arg_str(k).ok_or(ParseError { line: self.line(), msg: format!("missing number in {:?}", self.head()) })?;
        a.parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or(ParseError { line: self.line(), msg: format!("bad number '{a}'") })
    }
    /// Numbers of a child: `(at 1 2 90)` → [1, 2, 90].
    pub fn nums(&self) -> Result<Vec<f64>, ParseError> {
        (1..self.items().len()).map(|k| self.num(k)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_lists_strings_and_lines() {
        let n = parse("(a 1 \"x y\"\n (b (c \"q\\\"\")))").unwrap();
        assert_eq!(n.head(), Some("a"));
        assert_eq!(n.arg_str(2), Some("x y"));
        let b = n.child("b").unwrap();
        assert_eq!(b.line(), 2);
        assert_eq!(b.child("c").unwrap().arg_str(1), Some("q\""));
    }

    #[test]
    fn malformed_input_errors_with_line() {
        assert_eq!(parse("(a\n(b)").unwrap_err().line, 1);
        assert_eq!(parse("(a)\n)").unwrap_err().line, 2);
        assert!(parse("(a \"open").is_err());
        assert!(parse("").is_err());
        let deep = "(".repeat(100_000);
        assert!(parse(&deep).is_err());
    }
}
