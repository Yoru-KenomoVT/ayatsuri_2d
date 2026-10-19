// The object graph a Cubism Editor project is written as, built in memory
// and then serialised the way main.xml has it: every element is an object
// or a field (`xs.n` names the field), an object pointed at from more than
// one place is defined once under `<shared>` with an `xs.id` and referred
// to everywhere else with `xs.ref`, and `<main>` holds the model. Lists
// carry their `count`; empty maps say their keys are strings.

use std::fmt::Write as _;

/// A node of the graph: tag, attributes, text and children. A child is
/// either a named field or a list item.
pub struct Node {
    pub tag: &'static str,
    pub attrs: Vec<(&'static str, String)>,
    pub text: String,
    pub kids: Vec<(Option<&'static str>, usize)>,
    /// Defined under `<shared>` even if only one place points at it:
    /// guids and the sources the importer looks up by id.
    pub share: bool,
}

#[derive(Default)]
pub struct Tree {
    pub nodes: Vec<Node>,
}

const LISTS: [&str; 5] = ["carray_list", "array_list", "hash_map", "linked_map", "linked_set"];

impl Tree {
    pub fn obj(&mut self, tag: &'static str) -> usize {
        self.nodes.push(Node {
            tag,
            attrs: Vec::new(),
            text: String::new(),
            kids: Vec::new(),
            share: false,
        });
        self.nodes.len() - 1
    }

    /// An object with an attribute set.
    pub fn obj_with(&mut self, tag: &'static str, attrs: &[(&'static str, String)]) -> usize {
        let n = self.obj(tag);
        if let Some(node) = self.nodes.get_mut(n) {
            node.attrs.extend(attrs.iter().cloned());
        }
        n
    }

    pub fn share(&mut self, n: usize) -> usize {
        if let Some(node) = self.nodes.get_mut(n) {
            node.share = true;
        }
        n
    }

    /// `child` becomes the field `name` of `parent`.
    pub fn set(&mut self, parent: usize, name: &'static str, child: usize) {
        if let Some(node) = self.nodes.get_mut(parent) {
            node.kids.push((Some(name), child));
        }
    }

    /// `child` is appended to the list `parent`.
    pub fn push(&mut self, parent: usize, child: usize) {
        if let Some(node) = self.nodes.get_mut(parent) {
            node.kids.push((None, child));
        }
    }

    /// A new field holding text: `<tag xs.n="name">text</tag>`.
    pub fn leaf(&mut self, parent: usize, tag: &'static str, name: &'static str, text: String) {
        let n = self.obj(tag);
        if let Some(node) = self.nodes.get_mut(n) {
            node.text = text;
        }
        self.set(parent, name, n);
    }

    pub fn s(&mut self, parent: usize, name: &'static str, text: &str) {
        self.leaf(parent, "s", name, text.to_string());
    }

    pub fn i(&mut self, parent: usize, name: &'static str, v: i64) {
        self.leaf(parent, "i", name, v.to_string());
    }

    pub fn l(&mut self, parent: usize, name: &'static str, v: i64) {
        self.leaf(parent, "l", name, v.to_string());
    }

    pub fn f(&mut self, parent: usize, name: &'static str, v: f32) {
        self.leaf(parent, "f", name, fmt_f(v));
    }

    pub fn b(&mut self, parent: usize, name: &'static str, v: bool) {
        self.leaf(parent, "b", name, if v { "true".into() } else { "false".into() });
    }

    pub fn null(&mut self, parent: usize, name: &'static str) {
        let n = self.obj("null");
        self.set(parent, name, n);
    }

    /// An enum field: `<Tag xs.n="name" v="VALUE"/>`.
    pub fn enum_v(&mut self, parent: usize, tag: &'static str, name: &'static str, v: &str) {
        let n = self.obj_with(tag, &[("v", v.to_string())]);
        self.set(parent, name, n);
    }

    /// A new list as the field `name` of `parent`; items go in with `push`.
    pub fn list(&mut self, parent: usize, tag: &'static str, name: &'static str) -> usize {
        let n = self.obj(tag);
        self.set(parent, name, n);
        n
    }

    /// A numeric array field with its count: `<float-array count="n">..</float-array>`.
    pub fn floats(&mut self, parent: usize, name: &'static str, v: &[f32]) {
        let mut text = String::with_capacity(v.len() * 10);
        for (i, x) in v.iter().enumerate() {
            if i > 0 {
                text.push(' ');
            }
            text.push_str(&fmt_f(*x));
        }
        let n = self.obj_with("float-array", &[("count", v.len().to_string())]);
        if let Some(node) = self.nodes.get_mut(n) {
            node.text = text;
        }
        self.set(parent, name, n);
    }

    pub fn ints(&mut self, parent: usize, tag: &'static str, name: &'static str, v: &[i64]) {
        let text = v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" ");
        let n = self.obj_with(tag, &[("count", v.len().to_string())]);
        if let Some(node) = self.nodes.get_mut(n) {
            node.text = text;
        }
        self.set(parent, name, n);
    }

    /// A guid object: `<Tag uuid=".." note=".."/>`, always shared.
    pub fn guid(&mut self, tag: &'static str, uuid: &str, note: &str) -> usize {
        let n = self.obj_with(tag, &[("uuid", uuid.to_string()), ("note", note.to_string())]);
        self.share(n)
    }

    /// A 2D affine as Cubism writes it.
    pub fn affine(&mut self, parent: usize, name: &'static str, m: [f32; 6]) {
        let [m00, m01, m02, m10, m11, m12] = m;
        let n = self.obj_with(
            "CAffine",
            &[
                ("m00", fmt_f(m00)),
                ("m01", fmt_f(m01)),
                ("m02", fmt_f(m02)),
                ("m10", fmt_f(m10)),
                ("m11", fmt_f(m11)),
                ("m12", fmt_f(m12)),
            ],
        );
        self.set(parent, name, n);
    }

    pub fn identity(&mut self, parent: usize, name: &'static str) {
        self.affine(parent, name, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
    }

    /// The whole document: the processing instructions, then `shared` with
    /// every object referred to more than once, then `main` with the root.
    pub fn emit(&self, head: &str, main: usize) -> String {
        // first pass: how many places point at each node, and the order
        // they are first met in
        let mut hits = vec![0u32; self.nodes.len()];
        let mut order: Vec<usize> = Vec::new();
        let mut stack = vec![main];
        while let Some(n) = stack.pop() {
            let Some(h) = hits.get_mut(n) else { continue };
            *h += 1;
            if *h > 1 {
                continue;
            }
            order.push(n);
            if let Some(node) = self.nodes.get(n) {
                stack.extend(node.kids.iter().rev().map(|(_, c)| *c));
            }
        }
        let mut ids: Vec<Option<usize>> = vec![None; self.nodes.len()];
        let mut shared: Vec<usize> = Vec::new();
        for &n in &order {
            let share = self.nodes.get(n).is_some_and(|x| x.share);
            if share || hits.get(n).copied().unwrap_or(0) > 1 {
                if let Some(slot) = ids.get_mut(n) {
                    *slot = Some(shared.len());
                }
                shared.push(n);
            }
        }
        let mut idx: Vec<usize> = vec![0; self.nodes.len()];
        for (serial, &n) in order.iter().enumerate() {
            if let Some(slot) = idx.get_mut(n) {
                *slot = serial + 1;
            }
        }
        let mut out = String::with_capacity(1 << 20);
        out.push_str(head);
        out.push_str("<root fileFormatVersion=\"502030002\">\n<shared>\n");
        for &n in &shared {
            self.write(&mut out, n, None, true, &ids, &idx, 1);
        }
        out.push_str("</shared>\n<main>\n");
        self.write(&mut out, main, None, true, &ids, &idx, 1);
        out.push_str("</main>\n</root>\n");
        out
    }

    /// One element: a reference when the node is shared and this is not
    /// its definition, otherwise the node with its children.
    fn write(
        &self,
        out: &mut String,
        n: usize,
        field: Option<&str>,
        define: bool,
        ids: &[Option<usize>],
        idx: &[usize],
        depth: usize,
    ) {
        let Some(node) = self.nodes.get(n) else { return };
        let id = ids.get(n).copied().flatten();
        for _ in 0..depth.min(64) {
            out.push_str("  ");
        }
        out.push('<');
        out.push_str(node.tag);
        if let Some(f) = field {
            out.push_str(" xs.n=\"");
            esc(out, f);
            out.push('"');
        }
        if let (Some(id), false) = (id, define) {
            let _ = writeln!(out, " xs.ref=\"#{id}\"/>");
            return;
        }
        for (k, v) in &node.attrs {
            out.push(' ');
            out.push_str(k);
            out.push_str("=\"");
            esc(out, v);
            out.push('"');
        }
        if LISTS.contains(&node.tag) {
            let _ = write!(out, " count=\"{}\"", node.kids.len());
            if node.kids.is_empty()
                && (node.tag == "hash_map" || node.tag == "linked_map")
                && !node.attrs.iter().any(|(k, _)| *k == "keyType")
            {
                out.push_str(" keyType=\"string\"");
            }
        }
        if let Some(id) = id {
            let _ = write!(out, " xs.id=\"#{id}\" xs.idx=\"{}\"", idx.get(n).copied().unwrap_or(0));
        }
        if node.kids.is_empty() && node.text.is_empty() {
            out.push_str("/>\n");
            return;
        }
        out.push('>');
        esc(out, &node.text);
        if !node.kids.is_empty() {
            out.push('\n');
            for (f, c) in &node.kids {
                self.write(out, *c, *f, false, ids, idx, depth + 1);
            }
            for _ in 0..depth.min(64) {
                out.push_str("  ");
            }
        }
        out.push_str("</");
        out.push_str(node.tag);
        out.push_str(">\n");
    }
}

/// Floats the way Java prints them: a decimal point always, no exponent
/// for the sizes a model has.
pub fn fmt_f(v: f32) -> String {
    if !v.is_finite() {
        return "0.0".into();
    }
    if v == v.trunc() && v.abs() < 1e7 { format!("{v:.1}") } else { format!("{v}") }
}

fn esc(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' => {}
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_nodes_define_once_and_refer_elsewhere() {
        let mut t = Tree::default();
        let root = t.obj("CModelSource");
        let g = t.guid("CPartGuid", "abc", "Part");
        let a = t.obj("A");
        let b = t.obj("B");
        t.set(root, "a", a);
        t.set(root, "b", b);
        t.set(a, "guid", g);
        t.set(b, "parentGuid", g);
        let once = t.obj("Once");
        t.s(once, "name", "x<y");
        t.set(a, "once", once);
        let xml = t.emit("<?xml version=\"1.0\"?>\n", root);
        assert_eq!(xml.matches("uuid=\"abc\"").count(), 1, "{xml}");
        assert_eq!(xml.matches("xs.ref=\"#0\"").count(), 2, "{xml}");
        assert!(xml.contains("<s xs.n=\"name\">x&lt;y</s>"));
        assert!(xml.contains("<Once xs.n=\"once\">"), "unshared nodes stay inline: {xml}");
    }

    #[test]
    fn lists_count_their_items_and_empty_maps_say_string_keys() {
        let mut t = Tree::default();
        let root = t.obj("R");
        let l = t.list(root, "carray_list", "items");
        let x = t.obj("X");
        t.push(l, x);
        t.list(root, "hash_map", "opts");
        t.floats(root, "positions", &[1.0, 0.5, -2.25]);
        let xml = t.emit("", root);
        assert!(xml.contains("<carray_list xs.n=\"items\" count=\"1\">"), "{xml}");
        assert!(xml.contains("<hash_map xs.n=\"opts\" count=\"0\" keyType=\"string\"/>"), "{xml}");
        assert!(
            xml.contains("<float-array xs.n=\"positions\" count=\"3\">1.0 0.5 -2.25</float-array>")
        );
    }
}
