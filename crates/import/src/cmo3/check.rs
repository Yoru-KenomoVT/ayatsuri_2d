// A strict reading of a written project. Cubism Editor loads main.xml as
// one linked object graph and refuses a file over a single broken link,
// so every project the exporter writes is checked here before it leaves:
// references, counts and values by the rules the Editor's own files keep,
// and every object's shape against schema.txt, the shapes found in a
// project saved by the Editor (fields each class always has, the value
// types each field takes, the attributes, what each list holds).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::xml::Doc;

const LISTS: [&str; 5] = ["carray_list", "array_list", "hash_map", "linked_map", "linked_set"];
const ARRAYS: [&str; 6] =
    ["float-array", "double-array", "int-array", "short-array", "byte-array", "long-array"];

/// Attributes the graph itself uses, not part of any class.
fn graph_attr(k: &str) -> bool {
    matches!(k, "xs.n" | "xs.id" | "xs.idx" | "xs.ref")
}

#[derive(Default)]
struct Class {
    seen: usize,
    attrs: BTreeMap<String, usize>,
    fields: BTreeMap<String, (usize, BTreeSet<String>)>,
}

/// The object shapes of a project saved by the Editor.
#[derive(Default)]
pub struct Schema {
    classes: BTreeMap<String, Class>,
    items: BTreeMap<String, BTreeSet<String>>,
}

const SCHEMA: &str = include_str!("schema.txt");

impl Schema {
    pub fn builtin() -> Self {
        let mut s = Self::parse(SCHEMA);
        // 5.3's alpha blend on a mesh, which the sample project predates
        if let Some(c) = s.classes.get_mut("CArtMeshSource") {
            c.fields
                .insert("alphaComposition".into(), (0, ["AlphaComposition".to_string()].into()));
        }
        s.classes.insert(
            "AlphaComposition".into(),
            Class { seen: 1, attrs: [("v".to_string(), 1)].into(), ..Class::default() },
        );
        s
    }

    /// Learn the shapes from a project's graph.
    pub fn learn(doc: &Doc) -> Self {
        let mut s = Schema::default();
        let ctx = contexts(doc);
        for (i, n) in doc.nodes.iter().enumerate() {
            if n.attr("xs.ref").is_some() || matches!(n.tag.as_str(), "root" | "shared" | "main") {
                continue;
            }
            let c = s.classes.entry(n.tag.clone()).or_default();
            c.seen += 1;
            for (k, _) in &n.attrs {
                if !graph_attr(k) {
                    *c.attrs.entry(k.clone()).or_default() += 1;
                }
            }
            let mut named = BTreeSet::new();
            for &k in &n.children {
                let Some(kn) = doc.node(k) else { continue };
                match kn.attr("xs.n") {
                    Some(f) => {
                        let e = c.fields.entry(f.to_string()).or_default();
                        if named.insert(f) {
                            e.0 += 1;
                        }
                        e.1.insert(kn.tag.clone());
                    }
                    None if LISTS.contains(&n.tag.as_str()) => {
                        if let Some(at) = ctx.get(i) {
                            s.items.entry(at.clone()).or_default().insert(kn.tag.clone());
                        }
                    }
                    None => {}
                }
            }
        }
        s
    }

    #[cfg(test)]
    pub fn write(&self) -> String {
        let mut out = String::new();
        for (tag, c) in &self.classes {
            out.push_str(&format!("class {tag} {}\n", c.seen));
            for (a, n) in &c.attrs {
                out.push_str(&format!(" attr {a} {n}\n"));
            }
            for (f, (n, tags)) in &c.fields {
                let tags: Vec<&str> = tags.iter().map(String::as_str).collect();
                out.push_str(&format!(" field {f} {n} {}\n", tags.join(" ")));
            }
        }
        for (at, tags) in &self.items {
            let tags: Vec<&str> = tags.iter().map(String::as_str).collect();
            out.push_str(&format!("items {at} {}\n", tags.join(" ")));
        }
        out
    }

    fn parse(text: &str) -> Self {
        let mut s = Schema::default();
        let mut cur: Option<String> = None;
        for line in text.lines() {
            let mut w = line.split_ascii_whitespace();
            match (w.next(), w.next()) {
                (Some("class"), Some(tag)) => {
                    let seen = w.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                    s.classes.insert(tag.to_string(), Class { seen, ..Class::default() });
                    cur = Some(tag.to_string());
                }
                (Some("attr"), Some(a)) => {
                    let n = w.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                    if let Some(c) = cur.as_ref().and_then(|t| s.classes.get_mut(t)) {
                        c.attrs.insert(a.to_string(), n);
                    }
                }
                (Some("field"), Some(f)) => {
                    let n = w.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                    let tags = w.map(str::to_string).collect();
                    if let Some(c) = cur.as_ref().and_then(|t| s.classes.get_mut(t)) {
                        c.fields.insert(f.to_string(), (n, tags));
                    }
                }
                (Some("items"), Some(at)) => {
                    s.items.insert(at.to_string(), w.map(str::to_string).collect());
                }
                _ => {}
            }
        }
        s
    }
}

/// Where each element sits, for knowing what a list may hold: a field is
/// `Owner.field`, a list item is its list's place plus `[]`.
fn contexts(doc: &Doc) -> Vec<String> {
    let mut ctx = vec![String::new(); doc.nodes.len()];
    // children always come after their parent, so one forward pass works
    for (i, n) in doc.nodes.iter().enumerate() {
        let here = match ctx.get(i) {
            Some(c) if !c.is_empty() => c.clone(),
            _ => n.tag.clone(),
        };
        let list = LISTS.contains(&n.tag.as_str());
        for &k in &n.children {
            let Some(kn) = doc.node(k) else { continue };
            let at = match kn.attr("xs.n") {
                Some(f) => format!("{}.{f}", n.tag),
                None if list => format!("{here}[]"),
                None => kn.tag.clone(),
            };
            if let Some(slot) = ctx.get_mut(k) {
                *slot = at;
            }
        }
        if let Some(slot) = ctx.get_mut(i) {
            *slot = here;
        }
    }
    ctx
}

/// Problems found, each with how often it came up, worst first.
pub fn check(doc: &Doc, schema: &Schema, files: &[&str]) -> Vec<String> {
    let mut found: HashMap<String, usize> = HashMap::new();
    let mut bad = |msg: String| *found.entry(msg).or_default() += 1;

    // the document
    let root = doc.node(0);
    let top: Vec<&str> = root
        .map(|r| r.children.iter().filter_map(|&c| doc.node(c)).map(|c| c.tag.as_str()).collect())
        .unwrap_or_default();
    if root.map(|r| r.tag.as_str()) != Some("root") || top != ["shared", "main"] {
        bad("the document is not <root> holding <shared> then <main>".into());
    }

    // definitions: unique, numbered #0.. in document order, each with an
    // index of its own
    let mut ids: HashMap<&str, usize> = HashMap::new();
    let mut idxs = BTreeSet::new();
    let mut next = 0u64;
    for (i, n) in doc.nodes.iter().enumerate() {
        let Some(id) = n.attr("xs.id") else {
            if n.attr("xs.idx").is_some() {
                bad(format!("<{}> has an xs.idx without an xs.id", n.tag));
            }
            continue;
        };
        if ids.insert(id, i).is_some() {
            bad(format!("xs.id {id} is defined twice"));
        }
        if id.strip_prefix('#').and_then(|v| v.parse::<u64>().ok()) != Some(next) {
            bad(format!("xs.id {id} is out of sequence (expected #{next})"));
        }
        next += 1;
        match n.attr("xs.idx").and_then(|v| v.parse::<u64>().ok()) {
            Some(x) if !idxs.insert(x) => bad(format!("xs.idx {x} is used twice")),
            Some(_) => {}
            None => bad(format!("<{}> {id} has no xs.idx", n.tag)),
        }
    }

    for n in &doc.nodes {
        let tag = n.tag.as_str();
        // references: to something defined, of the same class, and empty
        if let Some(r) = n.attr("xs.ref") {
            match ids.get(r).and_then(|&t| doc.node(t)) {
                None => bad(format!("<{tag}> refers to {r}, which is not defined")),
                Some(t) if t.tag != n.tag => {
                    bad(format!("<{tag}> refers to a <{}>", t.tag));
                }
                Some(_) => {}
            }
            if !n.children.is_empty() || !n.text.trim().is_empty() {
                bad(format!("a reference <{tag}> has content"));
            }
            if n.attrs.iter().any(|(k, _)| !matches!(k.as_str(), "xs.n" | "xs.ref")) {
                bad(format!("a reference <{tag}> carries attributes"));
            }
            continue;
        }

        let mut names = BTreeSet::new();
        for k in n.children.iter().filter_map(|&k| doc.node(k)) {
            if let Some(f) = k.attr("xs.n")
                && !names.insert(f)
            {
                bad(format!("<{tag}> has its field {f} twice"));
            }
        }

        // counts and values
        if LISTS.contains(&tag) {
            match n.attr("count").and_then(|v| v.parse::<usize>().ok()) {
                Some(c) if c == n.children.len() => {}
                Some(c) => bad(format!("<{tag}> says {c} items and holds {}", n.children.len())),
                None if tag == "hash_map" || tag == "linked_map" => {}
                None => bad(format!("<{tag}> has no count")),
            }
        }
        if ARRAYS.contains(&tag) {
            let vals: Vec<&str> = n.text.split_ascii_whitespace().collect();
            match n.attr("count").and_then(|v| v.parse::<usize>().ok()) {
                Some(c) if c == vals.len() => {}
                Some(c) => bad(format!("<{tag}> says {c} values and holds {}", vals.len())),
                None => bad(format!("<{tag}> has no count")),
            }
            let float = tag == "float-array" || tag == "double-array";
            let ok = vals.iter().all(|v| {
                if float {
                    v.parse::<f64>().is_ok_and(f64::is_finite)
                } else {
                    v.parse::<i64>().is_ok()
                }
            });
            if !ok {
                bad(format!("<{tag}> holds a value that is not a number"));
            }
        }
        let v = n.text.trim();
        let ok = match tag {
            "i" | "l" => v.parse::<i64>().is_ok(),
            "f" | "d" => v.parse::<f64>().is_ok_and(f64::is_finite),
            "b" => v == "true" || v == "false",
            _ => true,
        };
        if !ok {
            bad(format!("<{tag}> holds {v:?}"));
        }
        if tag == "file"
            && let Some(p) = n.attr("path")
            && !files.contains(&p)
        {
            bad(format!("<file> points at {p}, which is not in the archive"));
        }
    }

    // shapes against the Editor's
    let mine = Schema::learn(doc);
    for (tag, c) in &mine.classes {
        let Some(want) = schema.classes.get(tag) else {
            bad(format!("<{tag}> is not a class the Editor writes"));
            continue;
        };
        for (a, &n) in &want.attrs {
            let have = c.attrs.get(a).copied().unwrap_or(0);
            if n == want.seen && have < c.seen {
                bad(format!("<{tag}> is missing its {a} attribute"));
            }
        }
        for a in c.attrs.keys() {
            if !want.attrs.contains_key(a) {
                bad(format!("<{tag}> has an attribute {a} the Editor never writes"));
            }
        }
        for (f, (n, _)) in &want.fields {
            let have = c.fields.get(f).map(|x| x.0).unwrap_or(0);
            if *n == want.seen && have < c.seen {
                bad(format!("<{tag}> is missing its field {f}"));
            }
        }
        for (f, (_, tags)) in &c.fields {
            let Some((_, allowed)) = want.fields.get(f) else {
                bad(format!("<{tag}> has a field {f} the Editor never writes"));
                continue;
            };
            for t in tags.difference(allowed) {
                bad(format!("<{tag}>.{f} holds a <{t}>, the Editor writes {allowed:?}"));
            }
        }
    }
    for (at, tags) in &mine.items {
        match schema.items.get(at) {
            None => bad(format!("a list at {at} the Editor never writes")),
            Some(allowed) => {
                for t in tags.difference(allowed) {
                    bad(format!("{at} holds a <{t}>, the Editor writes {allowed:?}"));
                }
            }
        }
    }

    let mut out: Vec<(String, usize)> = found.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    out.into_iter().map(|(m, n)| if n > 1 { format!("{m} (x{n})") } else { m }).collect()
}
