// Cubism Editor's main.xml is a serialized object graph: every element is
// an object or a field (`xs.n` names the field), `xs.id` defines an object
// others point at with `xs.ref`, and guids may be written inline (`uuid`)
// or by reference. This is a small DOM over it with the lookups the
// importer needs; nothing is recursive, so a hostile file cannot blow the
// stack.

use std::collections::HashMap;

use quick_xml::events::Event;

use crate::ImportError;

fn err(msg: impl Into<String>) -> ImportError {
    ImportError::Export(format!("cmo3: {}", msg.into()))
}

pub struct Node {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub children: Vec<usize>,
}

impl Node {
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

pub struct Doc {
    pub nodes: Vec<Node>,
    ids: HashMap<String, usize>,
    by_tag: HashMap<String, Vec<usize>>,
}

const MAX_NODES: usize = 8_000_000;

impl Doc {
    pub fn parse(xml: &[u8]) -> Result<Self, ImportError> {
        let mut reader = quick_xml::Reader::from_reader(xml);
        reader.config_mut().trim_text(true);
        let mut nodes: Vec<Node> = Vec::new();
        let mut stack: Vec<usize> = Vec::new();
        let mut buf = Vec::new();
        loop {
            let ev = reader.read_event_into(&mut buf).map_err(|e| err(format!("main.xml: {e}")))?;
            match ev {
                Event::Start(ref e) | Event::Empty(ref e) => {
                    if nodes.len() >= MAX_NODES || stack.len() > 512 {
                        return Err(err("main.xml too large or too deep"));
                    }
                    let tag = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                    let mut attrs = Vec::new();
                    for a in e.attributes().flatten() {
                        let k = String::from_utf8_lossy(a.key.as_ref()).into_owned();
                        let v = a
                            .decoded_and_normalized_value(
                                quick_xml::XmlVersion::Implicit1_0,
                                reader.decoder(),
                            )
                            .map(|v| v.into_owned())
                            .unwrap_or_default();
                        attrs.push((k, v));
                    }
                    let idx = nodes.len();
                    let parent = stack.last().copied();
                    nodes.push(Node { tag, attrs, text: String::new(), children: Vec::new() });
                    if let Some(p) = parent
                        && let Some(pn) = nodes.get_mut(p)
                    {
                        pn.children.push(idx);
                    }
                    if matches!(ev, Event::Start(_)) {
                        stack.push(idx);
                    }
                }
                Event::End(_) => {
                    stack.pop();
                }
                Event::Text(ref t) => {
                    if let (Some(&cur), Ok(raw)) = (stack.last(), t.decode())
                        && let Some(n) = nodes.get_mut(cur)
                    {
                        match quick_xml::escape::unescape(&raw) {
                            Ok(s) => n.text.push_str(&s),
                            Err(_) => n.text.push_str(&raw),
                        }
                    }
                }
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        let mut ids = HashMap::new();
        let mut by_tag: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, n) in nodes.iter().enumerate() {
            if let Some(id) = n.attr("xs.id") {
                ids.entry(id.to_string()).or_insert(i);
            }
            by_tag.entry(n.tag.clone()).or_default().push(i);
        }
        Ok(Doc { nodes, ids, by_tag })
    }

    pub fn node(&self, i: usize) -> Option<&Node> {
        self.nodes.get(i)
    }

    /// Every element with this tag, document order.
    pub fn all(&self, tag: &str) -> &[usize] {
        self.by_tag.get(tag).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Follow `xs.ref` until an element that is defined in place.
    pub fn deref(&self, mut i: usize) -> Option<usize> {
        for _ in 0..64 {
            let n = self.node(i)?;
            match n.attr("xs.ref") {
                Some(r) => i = *self.ids.get(r)?,
                None => return Some(i),
            }
        }
        None
    }

    /// The child field named `name` (`xs.n`), dereferenced.
    pub fn field(&self, i: usize, name: &str) -> Option<usize> {
        let n = self.node(i)?;
        let c = n
            .children
            .iter()
            .copied()
            .find(|&c| self.node(c).and_then(|cn| cn.attr("xs.n")) == Some(name))?;
        self.deref(c)
    }

    /// The child field itself, not dereferenced (to read its `xs.ref`).
    pub fn field_raw(&self, i: usize, name: &str) -> Option<usize> {
        let n = self.node(i)?;
        n.children
            .iter()
            .copied()
            .find(|&c| self.node(c).and_then(|cn| cn.attr("xs.n")) == Some(name))
    }

    /// A field looked up through the chain of `super` blocks.
    pub fn walk_super(&self, i: usize, name: &str) -> Option<usize> {
        let mut at = Some(i);
        for _ in 0..32 {
            let cur = at?;
            if let Some(hit) = self.field(cur, name) {
                return Some(hit);
            }
            at = self.field_raw(cur, "super").and_then(|s| self.deref(s));
        }
        None
    }

    pub fn text(&self, i: usize) -> &str {
        self.node(i).map(|n| n.text.as_str()).unwrap_or("")
    }

    pub fn attr<'a>(&'a self, i: usize, key: &str) -> Option<&'a str> {
        self.node(i)?.attr(key)
    }

    pub fn floats(&self, i: usize) -> Vec<f32> {
        self.text(i).split_ascii_whitespace().filter_map(|v| v.parse().ok()).collect()
    }

    pub fn ints(&self, i: usize) -> Vec<i64> {
        self.text(i).split_ascii_whitespace().filter_map(|v| v.parse().ok()).collect()
    }

    pub fn float_field(&self, i: usize, name: &str) -> Option<f32> {
        self.text(self.field(i, name)?).trim().parse().ok()
    }

    pub fn int_field(&self, i: usize, name: &str) -> Option<i64> {
        self.text(self.field(i, name)?).trim().parse().ok()
    }

    pub fn bool_field(&self, i: usize, name: &str, default: bool) -> bool {
        match self.field(i, name).map(|f| self.text(f).trim()) {
            Some("true") => true,
            Some("false") => false,
            _ => default,
        }
    }

    pub fn str_field(&self, i: usize, name: &str) -> String {
        self.field(i, name).map(|f| self.text(f).to_string()).unwrap_or_default()
    }

    /// A guid element's uuid, written inline or reached by reference.
    pub fn uuid_of(&self, i: usize) -> Option<String> {
        let d = self.deref(i)?;
        self.attr(d, "uuid").map(str::to_string)
    }

    /// A guid field's uuid.
    pub fn uuid_field(&self, i: usize, name: &str) -> Option<String> {
        self.uuid_of(self.field_raw(i, name)?)
    }

    /// The `xs.ref` target name of a field, or its own `xs.id` when it is
    /// defined in place: the key Cubism uses to tie copies to originals.
    pub fn ref_key(&self, i: usize) -> Option<String> {
        let n = self.node(i)?;
        n.attr("xs.ref").or_else(|| n.attr("xs.id")).map(str::to_string)
    }

    /// Children of a list element, each dereferenced.
    pub fn items(&self, i: usize) -> Vec<usize> {
        self.node(i)
            .map(|n| n.children.iter().filter_map(|&c| self.deref(c)).collect())
            .unwrap_or_default()
    }

    /// Children of a list element as they are written (for their refs).
    pub fn items_raw(&self, i: usize) -> &[usize] {
        self.node(i).map(|n| n.children.as_slice()).unwrap_or(&[])
    }
}
