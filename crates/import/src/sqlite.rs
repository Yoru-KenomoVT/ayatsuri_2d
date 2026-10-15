// A read-only SQLite 3 reader, enough to pull whole tables out of a
// database embedded in another file. Rowid tables only (b-tree pages of
// type 0x05 and 0x0d), with overflow chains, as the file format document
// describes them. Every offset is checked; a damaged file gives an error,
// never a panic.

use crate::ImportError;

fn err(msg: impl Into<String>) -> ImportError {
    ImportError::Export(format!("sqlite: {}", msg.into()))
}

/// One cell value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Value {
    pub fn int(&self) -> Option<i64> {
        match self {
            Value::Int(v) => Some(*v),
            Value::Real(v) => Some(*v as i64),
            _ => None,
        }
    }

    pub fn real(&self) -> Option<f64> {
        match self {
            Value::Int(v) => Some(*v as f64),
            Value::Real(v) => Some(*v),
            _ => None,
        }
    }

    pub fn text(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }

    /// Bytes of a blob, or of text stored where a blob was expected.
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Blob(b) => Some(b),
            Value::Text(s) => Some(s.as_bytes()),
            _ => None,
        }
    }
}

/// A table read whole: column names as declared, rows in rowid order.
pub struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

impl Table {
    pub fn col(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.eq_ignore_ascii_case(name))
    }

    /// A row's value by column name.
    pub fn get<'a>(&self, row: &'a [Value], name: &str) -> Option<&'a Value> {
        row.get(self.col(name)?)
    }
}

/// A database held in memory.
pub struct Db<'a> {
    bytes: &'a [u8],
    page_size: usize,
    usable: usize,
    /// Table name, root page, CREATE statement, from sqlite_master.
    master: Vec<(String, u32, String)>,
}

/// Where to stop on a file that lies about its own size.
const MAX_ROWS: usize = 1 << 20;
const MAX_CELL: usize = 64 << 20;

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes([*b.get(at)?, *b.get(at + 1)?, *b.get(at + 2)?, *b.get(at + 3)?]))
}

/// SQLite's variable-length integer: up to nine bytes, big-endian, high
/// bit continues. Returns the value and how many bytes it took.
fn varint(b: &[u8], at: usize) -> Option<(i64, usize)> {
    let mut v: u64 = 0;
    for i in 0..8 {
        let byte = *b.get(at + i)?;
        v = (v << 7) | (byte & 0x7f) as u64;
        if byte & 0x80 == 0 {
            return Some((v as i64, i + 1));
        }
    }
    let last = *b.get(at + 8)?;
    v = (v << 8) | last as u64;
    Some((v as i64, 9))
}

impl<'a> Db<'a> {
    /// Open a database image. `bytes` must start at the file header.
    pub fn open(bytes: &'a [u8]) -> Result<Self, ImportError> {
        if bytes.get(..16) != Some(b"SQLite format 3\0") {
            return Err(err("not an SQLite 3 database"));
        }
        let raw = be16(bytes, 16).ok_or_else(|| err("short header"))?;
        let page_size = if raw == 1 { 65536 } else { raw as usize };
        if !(512..=65536).contains(&page_size) || !page_size.is_power_of_two() {
            return Err(err(format!("page size {page_size}")));
        }
        let reserved = *bytes.get(20).ok_or_else(|| err("short header"))? as usize;
        let usable = page_size.checked_sub(reserved).ok_or_else(|| err("reserved bytes"))?;
        if usable < 480 {
            return Err(err("usable page size too small"));
        }
        let encoding = be32(bytes, 56).unwrap_or(1);
        if encoding != 1 {
            return Err(err("only UTF-8 databases are read"));
        }
        let mut db = Db { bytes, page_size, usable, master: Vec::new() };
        // sqlite_master lives in page 1: type, name, tbl_name, rootpage, sql
        let rows = db.scan(1)?;
        for row in rows {
            let kind = row.first().and_then(Value::text).unwrap_or("");
            let name = row.get(1).and_then(Value::text).unwrap_or("");
            let root = row.get(3).and_then(Value::int).unwrap_or(0);
            let sql = row.get(4).and_then(Value::text).unwrap_or("");
            if kind == "table" && root > 0 {
                db.master.push((name.to_string(), root as u32, sql.to_string()));
            }
        }
        Ok(db)
    }

    /// Whether a table exists.
    pub fn has(&self, table: &str) -> bool {
        self.master.iter().any(|(n, _, _)| n.eq_ignore_ascii_case(table))
    }

    /// A whole table. The INTEGER PRIMARY KEY column, if any, carries the
    /// rowid, which the record itself stores as NULL.
    pub fn table(&self, table: &str) -> Result<Table, ImportError> {
        let (_, root, sql) = self
            .master
            .iter()
            .find(|(n, _, _)| n.eq_ignore_ascii_case(table))
            .ok_or_else(|| err(format!("no table {table}")))?;
        let (columns, rowid_col) = columns_of(sql);
        let mut rows = self.scan(*root)?;
        if let Some(k) = rowid_col {
            for row in &mut rows {
                // the rowid was pushed last by scan; move it into its column
                if let Some(id) = row.pop()
                    && let Some(slot) = row.get_mut(k)
                {
                    *slot = id;
                }
            }
        } else {
            for row in &mut rows {
                row.pop();
            }
        }
        for row in &mut rows {
            row.resize(columns.len(), Value::Null);
        }
        Ok(Table { columns, rows })
    }

    fn page(&self, number: u32) -> Result<&'a [u8], ImportError> {
        let n = (number as usize).checked_sub(1).ok_or_else(|| err("page 0"))?;
        let start = n.checked_mul(self.page_size).ok_or_else(|| err("page offset"))?;
        let end = start.checked_add(self.page_size).ok_or_else(|| err("page offset"))?;
        self.bytes.get(start..end).ok_or_else(|| err(format!("page {number} past the end")))
    }

    /// Every record under a table b-tree root, each with its rowid pushed
    /// as a trailing value.
    fn scan(&self, root: u32) -> Result<Vec<Vec<Value>>, ImportError> {
        let mut out = Vec::new();
        let mut stack = vec![(root, 0usize)];
        let mut seen = 0usize;
        while let Some((number, depth)) = stack.pop() {
            if depth > 64 {
                return Err(err("b-tree too deep"));
            }
            seen += 1;
            if seen > 1 << 20 {
                return Err(err("b-tree loops"));
            }
            let page = self.page(number)?;
            // page 1 carries the 100-byte file header first
            let head = if number == 1 { 100 } else { 0 };
            let kind = *page.get(head).ok_or_else(|| err("short page"))?;
            let cells = be16(page, head + 3).ok_or_else(|| err("short page"))? as usize;
            match kind {
                0x05 => {
                    let right = be32(page, head + 8).ok_or_else(|| err("short page"))?;
                    let ptrs = head + 12;
                    // children are pushed right to left so rows come out in order
                    stack.push((right, depth + 1));
                    for i in (0..cells).rev() {
                        let at =
                            be16(page, ptrs + i * 2).ok_or_else(|| err("cell pointer"))? as usize;
                        let child = be32(page, at).ok_or_else(|| err("cell"))?;
                        stack.push((child, depth + 1));
                    }
                }
                0x0d => {
                    let ptrs = head + 8;
                    for i in 0..cells {
                        let at =
                            be16(page, ptrs + i * 2).ok_or_else(|| err("cell pointer"))? as usize;
                        let (len, n1) = varint(page, at).ok_or_else(|| err("cell"))?;
                        let (rowid, n2) = varint(page, at + n1).ok_or_else(|| err("cell"))?;
                        let payload = self.payload(page, at + n1 + n2, len)?;
                        let mut row = record(&payload)?;
                        row.push(Value::Int(rowid));
                        out.push(row);
                        if out.len() > MAX_ROWS {
                            return Err(err("too many rows"));
                        }
                    }
                }
                0x02 | 0x0a => {}
                other => return Err(err(format!("page type {other:#x}"))),
            }
        }
        Ok(out)
    }

    /// A cell's payload, following the overflow chain when it does not fit
    /// in the page. The split follows the format document's table-leaf
    /// formula.
    fn payload(&self, page: &'a [u8], at: usize, len: i64) -> Result<Vec<u8>, ImportError> {
        let total = usize::try_from(len).map_err(|_| err("payload length"))?;
        if total > MAX_CELL {
            return Err(err("payload too large"));
        }
        let u = self.usable;
        let x = u - 35;
        if total <= x {
            return Ok(page.get(at..at + total).ok_or_else(|| err("payload"))?.to_vec());
        }
        let m = ((u - 12) * 32 / 255) - 23;
        let k = m + ((total - m) % (u - 4));
        let local = if k <= x { k } else { m };
        let mut out = page.get(at..at + local).ok_or_else(|| err("payload"))?.to_vec();
        let mut next = be32(page, at + local).ok_or_else(|| err("overflow pointer"))?;
        let mut hops = 0;
        while next != 0 && out.len() < total {
            hops += 1;
            if hops > 1 << 16 {
                return Err(err("overflow chain loops"));
            }
            let op = self.page(next)?;
            next = be32(op, 0).ok_or_else(|| err("overflow page"))?;
            let take = (total - out.len()).min(u - 4);
            out.extend_from_slice(op.get(4..4 + take).ok_or_else(|| err("overflow page"))?);
        }
        if out.len() != total {
            return Err(err("overflow chain short"));
        }
        Ok(out)
    }
}

/// Decode one record: a header of serial types, then the values.
fn record(p: &[u8]) -> Result<Vec<Value>, ImportError> {
    let (hlen, n) = varint(p, 0).ok_or_else(|| err("record header"))?;
    let hlen = usize::try_from(hlen).map_err(|_| err("record header"))?;
    let mut types = Vec::new();
    let mut at = n;
    while at < hlen {
        let (t, k) = varint(p, at).ok_or_else(|| err("record header"))?;
        types.push(t);
        at += k;
    }
    let mut out = Vec::with_capacity(types.len());
    let mut at = hlen;
    for t in types {
        let width = match t {
            0 | 8 | 9 => 0,
            1 => 1,
            2 => 2,
            3 => 3,
            4 => 4,
            5 => 6,
            6 | 7 => 8,
            10 | 11 => return Err(err("reserved serial type")),
            t if t % 2 == 0 => ((t - 12) / 2) as usize,
            t => ((t - 13) / 2) as usize,
        };
        let end = at.checked_add(width).ok_or_else(|| err("record body"))?;
        let bytes = p.get(at..end).ok_or_else(|| err("record body"))?;
        // sign-extend a big-endian integer of `width` bytes
        let int = |b: &[u8]| -> i64 {
            let mut v: i64 = 0;
            for &x in b {
                v = (v << 8) | x as i64;
            }
            let shift = 64 - 8 * b.len() as u32;
            (v << shift) >> shift
        };
        let v = match t {
            0 => Value::Null,
            1..=6 => Value::Int(int(bytes)),
            7 => {
                let arr: [u8; 8] = bytes.try_into().map_err(|_| err("real"))?;
                Value::Real(f64::from_be_bytes(arr))
            }
            8 => Value::Int(0),
            9 => Value::Int(1),
            t if t % 2 == 0 => Value::Blob(bytes.to_vec()),
            _ => Value::Text(String::from_utf8_lossy(bytes).into_owned()),
        };
        at = end;
        out.push(v);
    }
    Ok(out)
}

/// Column names from a CREATE TABLE statement, and which one is the
/// INTEGER PRIMARY KEY (the rowid alias), if any.
fn columns_of(sql: &str) -> (Vec<String>, Option<usize>) {
    let Some(open) = sql.find('(') else { return (Vec::new(), None) };
    let Some(close) = sql.rfind(')') else { return (Vec::new(), None) };
    let body = sql.get(open + 1..close).unwrap_or("");
    let mut cols = Vec::new();
    let mut rowid = None;
    let mut depth = 0i32;
    let mut start = 0usize;
    let parts: Vec<&str> = {
        let mut v = Vec::new();
        for (i, ch) in body.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => depth -= 1,
                ',' if depth == 0 => {
                    v.push(body.get(start..i).unwrap_or(""));
                    start = i + 1;
                }
                _ => {}
            }
        }
        v.push(body.get(start..).unwrap_or(""));
        v
    };
    for part in parts {
        let part = part.trim();
        let mut words = part.split_whitespace();
        let Some(first) = words.next() else { continue };
        let upper = first.to_ascii_uppercase();
        if ["PRIMARY", "UNIQUE", "CHECK", "FOREIGN", "CONSTRAINT"]
            .iter()
            .any(|k| upper.starts_with(k))
        {
            continue;
        }
        let name =
            first.trim_matches(|c| c == '"' || c == '`' || c == '[' || c == ']' || c == '\'');
        let rest = part.get(first.len()..).unwrap_or("").to_ascii_uppercase();
        if rest.contains("INTEGER") && rest.contains("PRIMARY KEY") {
            rowid = Some(cols.len());
        }
        cols.push(name.to_string());
    }
    (cols, rowid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints_decode_as_the_format_says() {
        assert_eq!(varint(&[0x7f], 0), Some((127, 1)));
        assert_eq!(varint(&[0x81, 0x00], 0), Some((128, 2)));
        assert_eq!(varint(&[0xff; 9], 0).map(|v| v.1), Some(9));
        assert_eq!(varint(&[0x80], 0), None);
    }

    #[test]
    fn column_lists_find_the_rowid_alias() {
        let (cols, rowid) = columns_of(
            "CREATE TABLE Layer(_PW_ID INTEGER PRIMARY KEY AUTOINCREMENT, MainId INTEGER, \
             LayerName TEXT, FilterLayerInfo BLOB, UNIQUE(MainId))",
        );
        assert_eq!(cols, ["_PW_ID", "MainId", "LayerName", "FilterLayerInfo"]);
        assert_eq!(rowid, Some(0));
    }

    #[test]
    fn a_record_with_every_serial_type_reads_back() {
        // header: size 8, types null, i8, i16, i24, const 0, text "hi", blob 1
        let mut p = vec![8u8, 0, 1, 2, 3, 8, 17, 14];
        p.extend_from_slice(&[0x7f, 0x01, 0x00, 0xff, 0xff, 0xff, b'h', b'i', 9]);
        let row = record(&p).unwrap();
        assert_eq!(
            row,
            vec![
                Value::Null,
                Value::Int(127),
                Value::Int(256),
                Value::Int(-1),
                Value::Int(0),
                Value::Text("hi".into()),
                Value::Blob(vec![9]),
            ]
        );
    }
}
