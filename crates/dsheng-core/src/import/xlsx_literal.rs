//! XLSX 数值单元格 XML 原始字面量（data-period-import.md §1.1）。
//!
//! 合同：“XLSX数字优先读取XML原始字面量；读取库只给float且不能恢复精度时标
//! `precision_unverified`。禁止以f64中转精确金额。”calamine 只给 f64，本模块
//! 直接从 xlsx 包（ZIP）解出 worksheet XML，按单元格绝对坐标取 `<v>` 原文。
//!
//! - 仅依赖标准库与已有 `regex`：ZIP 中央目录 + RFC 1951 inflate + CRC32 校验；
//! - 只采集数值型单元格（`t` 缺省或 `t="n"`）且**无 `<f>` 公式**者；公式缓存、
//!   共享串、内联串、布尔、错误一律不采；
//! - 任何结构异常（ZIP64、加密、未知压缩法、CRC 不符、超限、非 UTF-8）一律
//!   返回 None / 跳过该 sheet，由调用方回退 `precision_unverified`，绝不猜值。

use regex::Regex;
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

/// 单个 sheet 的数值字面量：(row, col) 0 基绝对坐标（A1=(0,0)）→ `<v>` 原文。
pub type SheetLiterals = HashMap<(u32, u32), String>;

/// 单个包内部件解压上限（防异常/恶意包；业务表远小于此）。
const MAX_PART_BYTES: usize = 256 * 1024 * 1024;

/// 读取工作簿全部 worksheet 的数值字面量；键为 sheet 名（已做 XML 反转义，
/// 与 calamine `sheet_names()` 一致）。包结构不可解析时返回 None。
pub fn read_numeric_literals(path: &Path) -> Option<HashMap<String, SheetLiterals>> {
    let data = std::fs::read(path).ok()?;
    numeric_literals_from_bytes(&data)
}

/// 同 [`read_numeric_literals`]，输入为 xlsx 包字节。
pub fn numeric_literals_from_bytes(data: &[u8]) -> Option<HashMap<String, SheetLiterals>> {
    let zip = ZipIndex::parse(data)?;
    let workbook_part =
        office_document_part(&zip, data).unwrap_or_else(|| "xl/workbook.xml".into());
    let workbook = zip.read_text(data, &workbook_part)?;
    let rels = zip.read_text(data, &rels_part_for(&workbook_part))?;
    let targets = relationship_targets(&rels);
    let base = parent_dir(&workbook_part);
    let mut out = HashMap::new();
    for (name, rid) in workbook_sheets(&workbook) {
        let Some(target) = targets.get(&rid) else {
            continue;
        };
        let part = resolve_part(&base, target);
        let Some(xml) = zip.read_text(data, &part) else {
            continue;
        };
        out.insert(name, scan_numeric_literals(&xml));
    }
    Some(out)
}

// ---- ZIP（中央目录；仅 stored/deflate，非 ZIP64、未加密） ----

#[derive(Debug, Clone, Copy)]
struct ZipEntry {
    method: u16,
    flags: u16,
    crc: u32,
    compressed: usize,
    uncompressed: usize,
    local_offset: usize,
}

struct ZipIndex {
    entries: HashMap<String, ZipEntry>,
}

fn le_u16(data: &[u8], at: usize) -> Option<u16> {
    let b = data.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}

fn le_u32(data: &[u8], at: usize) -> Option<u32> {
    let b = data.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

impl ZipIndex {
    fn parse(data: &[u8]) -> Option<Self> {
        const EOCD: u32 = 0x0605_4b50;
        const CENTRAL: u32 = 0x0201_4b50;
        if data.len() < 22 {
            return None;
        }
        // End of central directory：自尾部向前找（注释最长 65535）
        let lowest = data.len().saturating_sub(22 + 0xFFFF);
        let mut at = data.len() - 22;
        let eocd = loop {
            if le_u32(data, at)? == EOCD {
                break at;
            }
            if at == lowest {
                return None;
            }
            at -= 1;
        };
        let total = le_u16(data, eocd + 10)? as usize;
        let cd_size = le_u32(data, eocd + 12)?;
        let cd_offset = le_u32(data, eocd + 16)?;
        if total == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_offset == 0xFFFF_FFFF {
            return None; // ZIP64：回退
        }
        let mut entries = HashMap::with_capacity(total);
        let mut p = cd_offset as usize;
        for _ in 0..total {
            if le_u32(data, p)? != CENTRAL {
                return None;
            }
            let name_len = le_u16(data, p + 28)? as usize;
            let extra_len = le_u16(data, p + 30)? as usize;
            let comment_len = le_u16(data, p + 32)? as usize;
            let name_bytes = data.get(p + 46..p + 46 + name_len)?;
            let entry = ZipEntry {
                flags: le_u16(data, p + 8)?,
                method: le_u16(data, p + 10)?,
                crc: le_u32(data, p + 16)?,
                compressed: le_u32(data, p + 20)? as usize,
                uncompressed: le_u32(data, p + 24)? as usize,
                local_offset: le_u32(data, p + 42)? as usize,
            };
            let name = String::from_utf8_lossy(name_bytes).replace('\\', "/");
            entries.insert(name, entry);
            p += 46 + name_len + extra_len + comment_len;
        }
        Some(Self { entries })
    }

    /// 部件查找：先精确，再按 OPC 规则 ASCII 大小写不敏感。
    fn entry(&self, name: &str) -> Option<ZipEntry> {
        self.entries.get(name).copied().or_else(|| {
            self.entries
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, e)| *e)
        })
    }

    fn read(&self, data: &[u8], name: &str) -> Option<Vec<u8>> {
        const LOCAL: u32 = 0x0403_4b50;
        let e = self.entry(name)?;
        if e.flags & 1 != 0 || e.uncompressed > MAX_PART_BYTES {
            return None; // 加密 / 超限
        }
        let lo = e.local_offset;
        if le_u32(data, lo)? != LOCAL {
            return None;
        }
        // 数据起点以本地头自身的 name/extra 长度为准（可能与中央目录不同）
        let start = lo + 30 + le_u16(data, lo + 26)? as usize + le_u16(data, lo + 28)? as usize;
        let raw = data.get(start..start.checked_add(e.compressed)?)?;
        let out = match e.method {
            0 => raw.to_vec(),
            8 => inflate(raw, e.uncompressed)?,
            _ => return None,
        };
        (out.len() == e.uncompressed && crc32(&out) == e.crc).then_some(out)
    }

    fn read_text(&self, data: &[u8], name: &str) -> Option<String> {
        String::from_utf8(self.read(data, name)?).ok()
    }
}

fn crc32(data: &[u8]) -> u32 {
    static TABLE: OnceLock<[u32; 256]> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *slot = c;
        }
        t
    });
    let mut crc = 0xFFFF_FFFFu32;
    for b in data {
        crc = table[((crc ^ u32::from(*b)) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

// ---- inflate（RFC 1951；规范 Huffman 逐位解码） ----

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u64,
    count: u32,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            buf: 0,
            count: 0,
        }
    }

    fn take(&mut self, n: u32) -> Option<u32> {
        while self.count < n {
            let byte = *self.data.get(self.pos)?;
            self.pos += 1;
            self.buf |= u64::from(byte) << self.count;
            self.count += 8;
        }
        let value = (self.buf & ((1u64 << n) - 1)) as u32;
        self.buf >>= n;
        self.count -= n;
        Some(value)
    }

    /// stored 块：丢弃到字节边界。
    fn align(&mut self) {
        let drop = self.count % 8;
        self.buf >>= drop;
        self.count -= drop;
    }
}

struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    /// 由码长构造规范 Huffman；超额订阅（over-subscribed）拒绝。
    fn new(lengths: &[u8]) -> Option<Self> {
        let mut counts = [0u16; 16];
        for &len in lengths {
            counts[usize::from(len)] += 1;
        }
        counts[0] = 0;
        let mut left: i32 = 1;
        for &count in &counts[1..] {
            left = (left << 1) - i32::from(count);
            if left < 0 {
                return None;
            }
        }
        let mut offsets = [0u16; 16];
        for len in 1..15 {
            offsets[len + 1] = offsets[len] + counts[len];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (symbol, &len) in lengths.iter().enumerate() {
            if len != 0 {
                let slot = &mut offsets[usize::from(len)];
                symbols[usize::from(*slot)] = symbol as u16;
                *slot += 1;
            }
        }
        Some(Self { counts, symbols })
    }

    fn decode(&self, bits: &mut Bits<'_>) -> Option<u16> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= bits.take(1)? as i32;
            let count = i32::from(self.counts[len]);
            if code - count < first {
                return self.symbols.get((index + (code - first)) as usize).copied();
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        None
    }
}

const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
/// 码长码表的读取顺序（RFC 1951 §3.2.7）。
const CODE_LENGTH_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// 解压 raw deflate 流；输出必须恰为 `expected` 字节（超出即失败）。
fn inflate(data: &[u8], expected: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(expected);
    let mut bits = Bits::new(data);
    loop {
        let last = bits.take(1)? == 1;
        match bits.take(2)? {
            0 => stored_block(&mut bits, &mut out, expected)?,
            1 => {
                let (lit, dist) = fixed_tables()?;
                codes_block(&mut bits, &mut out, &lit, &dist, expected)?;
            }
            2 => {
                let (lit, dist) = dynamic_tables(&mut bits)?;
                codes_block(&mut bits, &mut out, &lit, &dist, expected)?;
            }
            _ => return None,
        }
        if last {
            return Some(out);
        }
    }
}

fn stored_block(bits: &mut Bits<'_>, out: &mut Vec<u8>, expected: usize) -> Option<()> {
    bits.align();
    let len = bits.take(16)?;
    let nlen = bits.take(16)?;
    if len != (!nlen & 0xFFFF) || out.len() + len as usize > expected {
        return None;
    }
    for _ in 0..len {
        out.push(bits.take(8)? as u8);
    }
    Some(())
}

fn fixed_tables() -> Option<(Huffman, Huffman)> {
    let mut lengths = [0u8; 288];
    for (symbol, len) in lengths.iter_mut().enumerate() {
        *len = match symbol {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    Some((Huffman::new(&lengths)?, Huffman::new(&[5u8; 30])?))
}

fn dynamic_tables(bits: &mut Bits<'_>) -> Option<(Huffman, Huffman)> {
    let nlen = bits.take(5)? as usize + 257;
    let ndist = bits.take(5)? as usize + 1;
    let ncode = bits.take(4)? as usize + 4;
    if nlen > 286 || ndist > 30 {
        return None;
    }
    let mut code_lengths = [0u8; 19];
    for &slot in CODE_LENGTH_ORDER.iter().take(ncode) {
        code_lengths[slot] = bits.take(3)? as u8;
    }
    let length_code = Huffman::new(&code_lengths)?;
    let mut lengths = vec![0u8; nlen + ndist];
    let mut index = 0;
    while index < nlen + ndist {
        let symbol = length_code.decode(bits)?;
        if symbol < 16 {
            lengths[index] = symbol as u8;
            index += 1;
            continue;
        }
        let (value, repeat) = match symbol {
            16 => (
                *lengths.get(index.checked_sub(1)?)?,
                3 + bits.take(2)? as usize,
            ),
            17 => (0, 3 + bits.take(3)? as usize),
            18 => (0, 11 + bits.take(7)? as usize),
            _ => return None,
        };
        if index + repeat > nlen + ndist {
            return None;
        }
        lengths[index..index + repeat].fill(value);
        index += repeat;
    }
    if lengths[256] == 0 {
        return None; // 无块结束码
    }
    Some((
        Huffman::new(&lengths[..nlen])?,
        Huffman::new(&lengths[nlen..])?,
    ))
}

fn codes_block(
    bits: &mut Bits<'_>,
    out: &mut Vec<u8>,
    lit: &Huffman,
    dist: &Huffman,
    expected: usize,
) -> Option<()> {
    loop {
        let symbol = usize::from(lit.decode(bits)?);
        if symbol < 256 {
            if out.len() >= expected {
                return None;
            }
            out.push(symbol as u8);
            continue;
        }
        if symbol == 256 {
            return Some(());
        }
        let li = symbol - 257;
        let length =
            usize::from(*LENGTH_BASE.get(li)?) + bits.take(u32::from(LENGTH_EXTRA[li]))? as usize;
        let di = usize::from(dist.decode(bits)?);
        let distance =
            usize::from(*DIST_BASE.get(di)?) + bits.take(u32::from(DIST_EXTRA[di]))? as usize;
        if distance > out.len() || out.len() + length > expected {
            return None;
        }
        let from = out.len() - distance;
        for k in 0..length {
            let byte = out[from + k];
            out.push(byte);
        }
    }
}

// ---- 包关系与 worksheet XML ----

fn attr_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?:^|\s)([A-Za-z_][\w.\-]*(?::[A-Za-z_][\w.\-]*)?)\s*=\s*(?:"([^"]*)"|'([^']*)')"#,
        )
        .expect("静态属性正则")
    })
}

/// 解析元素属性串为 (完整名 → 反转义值)。
fn attributes(raw: &str) -> HashMap<String, String> {
    attr_regex()
        .captures_iter(raw)
        .map(|c| {
            let value = c.get(2).or_else(|| c.get(3)).map_or("", |m| m.as_str());
            (c[1].to_string(), xml_unescape(value))
        })
        .collect()
}

fn xml_unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        let Some(semi) = tail.find(';') else {
            out.push_str(tail);
            return out;
        };
        let entity = &tail[1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => out.push(c),
            None => out.push_str(&tail[..=semi]),
        }
        rest = &tail[semi + 1..];
    }
    out.push_str(rest);
    out
}

fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn elements<'a>(xml: &'a str, element: &str) -> Vec<&'a str> {
    // `<(prefix:)?element ...>`（含自闭合）；返回属性串
    let re = Regex::new(&format!(r"<(?:[A-Za-z_][\w.\-]*:)?{element}\b([^>]*?)/?>"))
        .expect("静态元素正则");
    re.captures_iter(xml)
        .filter_map(|c| c.get(1).map(|m| m.as_str()))
        .collect()
}

/// `_rels/.rels` 的 officeDocument 目标（通常 `xl/workbook.xml`）。
fn office_document_part(zip: &ZipIndex, data: &[u8]) -> Option<String> {
    let rels = zip.read_text(data, "_rels/.rels")?;
    elements(&rels, "Relationship").into_iter().find_map(|raw| {
        let attrs = attributes(raw);
        let is_office = attrs
            .get("Type")
            .is_some_and(|t| t.ends_with("/officeDocument"));
        is_office
            .then(|| attrs.get("Target").map(|t| resolve_part("", t)))
            .flatten()
    })
}

fn parent_dir(part: &str) -> String {
    match part.rfind('/') {
        Some(i) => part[..=i].to_string(),
        None => String::new(),
    }
}

fn rels_part_for(part: &str) -> String {
    let dir = parent_dir(part);
    let file = &part[dir.len()..];
    format!("{dir}_rels/{file}.rels")
}

/// 关系目标 → 包内部件名（相对 base 目录或以 `/` 起的绝对名；规整 `.`/`..`）。
fn resolve_part(base: &str, target: &str) -> String {
    let joined = match target.strip_prefix('/') {
        Some(abs) => abs.to_string(),
        None => format!("{base}{target}"),
    };
    let mut segments: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            s => segments.push(s),
        }
    }
    segments.join("/")
}

fn relationship_targets(rels: &str) -> HashMap<String, String> {
    elements(rels, "Relationship")
        .into_iter()
        .filter_map(|raw| {
            let attrs = attributes(raw);
            if attrs.get("TargetMode").is_some_and(|m| m == "External") {
                return None;
            }
            Some((attrs.get("Id")?.clone(), attrs.get("Target")?.clone()))
        })
        .collect()
}

/// workbook.xml 的 (sheet 名, 关系 id)，按文档顺序。
fn workbook_sheets(workbook: &str) -> Vec<(String, String)> {
    elements(workbook, "sheet")
        .into_iter()
        .filter_map(|raw| {
            let attrs = attributes(raw);
            let name = attrs.get("name")?.clone();
            // 关系 id 位于 relationships 命名空间（通常 `r:id`）
            let rid = attrs
                .iter()
                .find(|(k, _)| k.contains(':') && local_name(k) == "id")
                .map(|(_, v)| v.clone())?;
            Some((name, rid))
        })
        .collect()
}

/// A1 引用 → (row, col) 0 基。
fn parse_a1(reference: &str) -> Option<(u32, u32)> {
    let letters = reference
        .bytes()
        .take_while(u8::is_ascii_alphabetic)
        .count();
    if letters == 0 || letters > 3 {
        return None;
    }
    let mut col: u32 = 0;
    for b in reference[..letters].bytes() {
        col = col * 26 + u32::from(b.to_ascii_uppercase() - b'A' + 1);
    }
    let row: u32 = reference[letters..].parse().ok()?;
    (row >= 1).then(|| (row - 1, col - 1))
}

/// 扫描 worksheet XML：数值型（`t` 缺省或 `n`）且无公式单元格的 `<v>` 原文。
///
/// 行/列缺省 `r` 时按前一行/前一格顺延（与 OOXML 隐式定位一致）。
fn scan_numeric_literals(xml: &str) -> SheetLiterals {
    static ROW: OnceLock<Regex> = OnceLock::new();
    static CELL: OnceLock<Regex> = OnceLock::new();
    static FORMULA: OnceLock<Regex> = OnceLock::new();
    static VALUE: OnceLock<Regex> = OnceLock::new();
    let row_re = ROW.get_or_init(|| {
        Regex::new(
            r"(?s)<(?:[A-Za-z_][\w.\-]*:)?row\b([^>]*?)(?:/>|>(.*?)</(?:[A-Za-z_][\w.\-]*:)?row>)",
        )
        .expect("静态 row 正则")
    });
    let cell_re = CELL.get_or_init(|| {
        Regex::new(
            r"(?s)<(?:[A-Za-z_][\w.\-]*:)?c\b([^>]*?)(?:/>|>(.*?)</(?:[A-Za-z_][\w.\-]*:)?c>)",
        )
        .expect("静态 c 正则")
    });
    let formula_re =
        FORMULA.get_or_init(|| Regex::new(r"<(?:[A-Za-z_][\w.\-]*:)?f\b").expect("静态 f 正则"));
    let value_re = VALUE.get_or_init(|| {
        Regex::new(r"(?s)<(?:[A-Za-z_][\w.\-]*:)?v\b[^>]*>(.*?)</(?:[A-Za-z_][\w.\-]*:)?v>")
            .expect("静态 v 正则")
    });
    let mut out = SheetLiterals::new();
    let mut next_row: u32 = 0;
    for row in row_re.captures_iter(xml) {
        let row_attrs = attributes(row.get(1).map_or("", |m| m.as_str()));
        let row_idx = match row_attrs.get("r").and_then(|r| r.parse::<u32>().ok()) {
            Some(r) if r >= 1 => r - 1,
            _ => next_row,
        };
        next_row = row_idx + 1;
        let Some(body) = row.get(2) else {
            continue;
        };
        let mut next_col: u32 = 0;
        for cell in cell_re.captures_iter(body.as_str()) {
            let attrs = attributes(cell.get(1).map_or("", |m| m.as_str()));
            let (r, c) = match attrs.get("r").and_then(|r| parse_a1(r)) {
                Some(pos) => pos,
                None => (row_idx, next_col),
            };
            next_col = c + 1;
            if !matches!(attrs.get("t").map(String::as_str), None | Some("n")) {
                continue;
            }
            let Some(content) = cell.get(2).map(|m| m.as_str()) else {
                continue;
            };
            if formula_re.is_match(content) {
                continue; // 公式缓存仅人工对照，不作为字面量
            }
            if let Some(v) = value_re.captures(content).and_then(|m| m.get(1)) {
                let literal = v.as_str().trim();
                if !literal.is_empty() {
                    out.insert((r, c), literal.to_string());
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    // 合成向量（zlib raw deflate，wbits=-15）；内容为合成 XML，无业务数据。
    const FIXED_SRC: &str = r#"<c r="H12" s="10" t="n"><v>1234.5</v></c>"#;
    const FIXED_DEFLATE: &str =
        "b3495628b255f23034525228b6553234505228b155ca53b2b329b333343236d133b5d12fb3b3d14fb60300";
    const STORED_DEFLATE: &str = "012900d6ff3c6320723d224831322220733d2231302220743d226e223e3c763e313233342e353c2f763e3c2f633e";
    const DYNAMIC_DEFLATE: &str = "75d54d4ac4401086e1abc81ca0d25f75f51f64b27025b8f408e26240461807bdbe234aba94fa76e9a4787b93875a2f6f9f7797e301876d7dfe7eb8c7e1ee7a3c9cceafa7f3cbd3f5727b7f7adfd6ebf68875b96eebf27d5a9e7fa71f7ea6cfb7a98f2d37496d5d3e7ebe2fb7f2b6fee675cf2bcb6b9457976f26b0389ff77c66f91ce5b3cb031045dcb7bd6fac6f51df7cdfba688ffb65ef17d62f51bff87e2f924bdcaf7bbfb27e8dfad5f555554ce37edbfb8df55bd46fbe5f86d888fb7def77d6ef51bffbfea8526adc1f7b7fb0fe88fac3fffd394bcd711f69f24ad4570a81a53fc292b444ee7084b9e118b1576ca949238c311d83424628199eb2994927963131836a46c819deb375c8209e3141838a46481ade744197414c63a206558d9035bceb528a24e21a1336a86c84b4e16d97a102621b1337a86e84bce17d571d02e21b1338a87084c4e18dd75a4589714ce4a0ca11328777de52964c9ceb74aed4b986ce35fd5975498c38d7e95ca9730d9dab77de5a1363ebdaed6bbeb0e38ded9d779814e25ca773a5ce3574aede792f904a9ceb74aed4b986ced53befbd4bfde7fc0b";

    fn dynamic_src() -> String {
        (1..25)
            .map(|i| {
                format!(
                    r#"<row r="{i}"><c r="B{i}" t="inlineStr"><is><t>K{i}</t></is></c><c r="H{i}" t="n"><v>{}.{:02}</v></c></row>"#,
                    i * 37,
                    (i * 7) % 100
                )
            })
            .collect()
    }

    #[test]
    fn inflate_fixed_stored_dynamic_blocks() {
        let fixed = inflate(&hex(FIXED_DEFLATE), FIXED_SRC.len()).expect("fixed Huffman");
        assert_eq!(fixed, FIXED_SRC.as_bytes());
        let stored = inflate(&hex(STORED_DEFLATE), FIXED_SRC.len()).expect("stored");
        assert_eq!(stored, FIXED_SRC.as_bytes());
        let src = dynamic_src();
        let dynamic = inflate(&hex(DYNAMIC_DEFLATE), src.len()).expect("dynamic Huffman");
        assert_eq!(dynamic, src.as_bytes());
        // 标准 CRC-32（与 zlib.crc32 一致）
        assert_eq!(crc32(FIXED_SRC.as_bytes()), 0x425c_b234);
    }

    #[test]
    fn inflate_rejects_truncated_or_oversized() {
        let data = hex(DYNAMIC_DEFLATE);
        let src = dynamic_src();
        assert!(
            inflate(&data[..data.len() / 2], src.len()).is_none(),
            "截断流"
        );
        assert!(inflate(&data, src.len() - 1).is_none(), "超出声明长度");
    }

    #[test]
    fn scan_literals_numeric_only_without_formula() {
        let xml = r#"<worksheet><sheetData>
            <row r="4"><c r="B4" t="inlineStr"><is><t>科目</t></is></c></row>
            <row r="12"><c r="H12" s="10" t="n"><v>1234.56</v></c><c r="I12" s="2"><v>-0.5</v></c>
              <c r="J12" t="n"><f>H12*2</f><v>2469.12</v></c><c r="K12" t="s"><v>3</v></c>
              <c r="L12" s="18" t="n"></c><c r="M12" s="18" t="n"/><c r="N12"><v>1E-3</v></c></row>
            <row><c><v>7</v></c><c><v>8.25</v></c></row>
          </sheetData></worksheet>"#;
        let lit = scan_numeric_literals(xml);
        assert_eq!(lit.get(&(11, 7)).map(String::as_str), Some("1234.56"));
        assert_eq!(lit.get(&(11, 8)).map(String::as_str), Some("-0.5"));
        assert!(!lit.contains_key(&(11, 9)), "公式缓存不采");
        assert!(!lit.contains_key(&(11, 10)), "共享串索引不采");
        assert!(
            !lit.contains_key(&(11, 11)) && !lit.contains_key(&(11, 12)),
            "空格无字面量"
        );
        assert_eq!(lit.get(&(11, 13)).map(String::as_str), Some("1E-3"));
        // 缺省 r：行顺延到第 13 行、列从 A 起顺延
        assert_eq!(lit.get(&(12, 0)).map(String::as_str), Some("7"));
        assert_eq!(lit.get(&(12, 1)).map(String::as_str), Some("8.25"));
    }

    #[test]
    fn relationships_and_names() {
        assert_eq!(parse_a1("H12"), Some((11, 7)));
        assert_eq!(parse_a1("AA1"), Some((0, 26)));
        assert_eq!(parse_a1("12"), None);
        assert_eq!(
            resolve_part("xl/", "worksheets/sheet1.xml"),
            "xl/worksheets/sheet1.xml"
        );
        assert_eq!(
            resolve_part("xl/", "/xl/worksheets/s.xml"),
            "xl/worksheets/s.xml"
        );
        assert_eq!(
            resolve_part("xl/", "../xl/worksheets/s.xml"),
            "xl/worksheets/s.xml"
        );
        assert_eq!(
            rels_part_for("xl/workbook.xml"),
            "xl/_rels/workbook.xml.rels"
        );
        let sheets = workbook_sheets(
            r#"<sheets><sheet name="A&amp;B" sheetId="1" r:id="rId3"/><sheet sheetId="2" name='核算表' r:id="rId1"/></sheets>"#,
        );
        assert_eq!(
            sheets,
            vec![
                ("A&B".to_string(), "rId3".to_string()),
                ("核算表".to_string(), "rId1".to_string())
            ]
        );
        assert_eq!(xml_unescape("a&#x4E00;&#20108;&lt;"), "a一二<");
    }
}
