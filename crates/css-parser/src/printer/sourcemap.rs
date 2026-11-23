//! Port of `src/printer/sourcemap.ts`: generate and parse source maps.

use super::lib::codec::{decode, encode};
use crate::parser::utils::base64::to_base64;
use crate::plain::{Plain, PlainObject};
use base64::Engine;
use crate::utils::fx::{HashMap, HashSet};
use std::rc::Rc;

/// `SourceMapObject`
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SourceMapObject {
    pub version: u32,
    pub file: Option<String>,
    pub source_root: Option<String>,
    pub sources: Vec<Option<Rc<str>>>,
    pub sources_content: Vec<Option<Rc<str>>>,
    pub names: Option<Vec<String>>,
    pub mappings: String,
}

impl SourceMapObject {
    /// plain object (`toJSON()`): version, sources, sourcesContent, mappings
    pub fn to_plain(&self) -> Plain {
        let mut o = PlainObject::new();
        o.set("version", self.version);
        o.set("sources", Plain::Array(self.sources.iter().map(|s| Plain::from(s.clone())).collect()));
        o.set("sourcesContent", Plain::Array(self.sources_content.iter().map(|s| Plain::from(s.clone())).collect()));
        o.set("mappings", self.mappings.as_str());
        Plain::Object(o)
    }

    pub fn to_json(&self) -> String {
        self.to_plain().to_json()
    }

    pub fn from_json(value: &serde_json::Value) -> Self {
        let strings = |key: &str| -> Vec<Option<Rc<str>>> {
            value[key].as_array().map(|a| a.iter().map(|s| s.as_str().map(Rc::from)).collect()).unwrap_or_default()
        };
        SourceMapObject {
            version: value["version"].as_u64().unwrap_or(3) as u32,
            file: value["file"].as_str().map(String::from),
            source_root: value["sourceRoot"].as_str().map(String::from),
            sources: strings("sources"),
            sources_content: strings("sourcesContent"),
            names: value["names"].as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect()),
            mappings: value["mappings"].as_str().unwrap_or("").to_string(),
        }
    }
}

/// a line of the map: segments, in insertion order
type Line = Vec<Vec<i64>>;

/// map keyed by line, iterated in insertion order (javascript `Map`)
#[derive(Clone, Debug, Default)]
struct LineMapOrdered {
    order: Vec<i64>,
    lines: HashMap<i64, Line>,
}

impl LineMapOrdered {
    fn set(&mut self, line: i64, value: Line) {
        if !self.lines.contains_key(&line) {
            self.order.push(line);
        }
        self.lines.insert(line, value);
    }

    fn get(&self, line: i64) -> Option<&Line> {
        self.lines.get(&line)
    }

    fn get_mut(&mut self, line: i64) -> Option<&mut Line> {
        self.lines.get_mut(&line)
    }

    fn clear(&mut self) {
        self.order.clear();
        self.lines.clear();
    }

    fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// a source map: `[file, line, column, content]` lookups, json / data url output
#[derive(Clone, Debug)]
pub struct SourceMap {
    keys: HashSet<(i64, i64, i64, i64, i64)>,
    last_location: Option<(i64, i64)>,
    version: u32,
    sources_map: Vec<i64>,
    sources_content: Vec<Option<Rc<str>>>,
    sources: Vec<Option<Rc<str>>>,
    map: LineMapOrdered,
    reverse_map: LineMapOrdered,
    line: i64,
}

impl Default for SourceMap {
    fn default() -> Self {
        Self::new()
    }
}

/// `decodeURIComponent()`
fn decode_uri_component(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl SourceMap {
    pub fn new() -> Self {
        Self {
            keys: HashSet::default(),
            last_location: None,
            version: 3,
            sources_map: Vec::new(),
            sources_content: Vec::new(),
            sources: Vec::new(),
            map: LineMapOrdered::default(),
            reverse_map: LineMapOrdered::default(),
            line: -1,
        }
    }

    /// parse a source map: json, or a `data:` url (base64 or url encoded)
    pub fn parse(source: &str) -> Result<Self, String> {
        let mut text = source.to_string();
        if source.starts_with("data:") {
            let mut encoding = "";
            let mut offset = source.find(',').map_or(0, |i| i + 1);
            if offset == 0 {
                offset = source.rfind(';').map_or(0, |i| i + 1);
            } else {
                let start = source.rfind(';').map_or(0, |i| i + 1);
                encoding = if start <= offset - 1 { &source[start..offset - 1] } else { "" };
            }
            text = if encoding == "base64" {
                let decoded = base64::engine::general_purpose::STANDARD.decode(source[offset..].trim()).map_err(|e| e.to_string())?;
                // atob() returns latin1 characters
                decoded.iter().map(|&b| b as char).collect()
            } else {
                decode_uri_component(&source[offset..])
            };
        }
        let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        Ok(Self::from_object(&SourceMapObject::from_json(&value)))
    }

    /// a source map from a source map object
    pub fn from_object(object: &SourceMapObject) -> Self {
        let mut map = Self::new();
        map.sources = object.sources.clone();
        map.sources_content = object.sources_content.clone();
        let decoded: Vec<Line> = object.mappings.split(';').map(|line| line.split(',').map(decode).collect()).collect();
        map.line = decoded.len() as i64 - 1;
        for (index, line) in decoded.into_iter().enumerate() {
            if line.is_empty() || (line.len() == 1 && line[0].is_empty()) {
                continue;
            }
            map.map.set(index as i64, line);
        }
        map.compute_positions();
        map
    }

    /// add a source
    pub fn add_source_content(&mut self, id: i64, file_name: Option<&str>, content: Option<&str>) {
        if self.sources_map.contains(&id) {
            return;
        }
        self.sources_map.push(id);
        self.sources.push(file_name.filter(|f| !f.is_empty()).map(Rc::from));
        self.sources_content.push(content.filter(|c| !c.is_empty()).map(Rc::from));
    }

    /// add mappings `[newLine, newColumn, srcId, ln, col]`
    pub fn add(&mut self, maps: &[[i64; 5]]) -> Result<(), String> {
        for &[new_line, new_column, src_id, ln, col] in maps {
            let key = (src_id, ln, col, new_line, new_column);
            if !self.keys.insert(key) {
                continue;
            }
            let line = new_line - 1;
            if line > self.line {
                self.line = line;
            }
            let src_index = self
                .sources_map
                .iter()
                .position(|&s| s == src_id)
                .ok_or_else(|| format!("Source file {} not added to sourcemap", src_id))? as i64;
            let mut record: Vec<i64> = if let Some(arr) = self.map.get(line) {
                vec![0.max(new_column - 1) - arr[0][0], src_index - arr[0][1], ln - 1, col - 1]
            } else {
                vec![0.max(new_column - 1), src_index, ln - 1, col - 1]
            };
            if let Some((last_ln, last_col)) = self.last_location {
                record[2] -= last_ln - 1;
                record[3] -= last_col - 1;
            }
            if let Some(arr) = self.map.get_mut(line) {
                arr.push(record);
            } else {
                self.map.set(line, vec![record]);
            }
            self.last_location = Some((ln, col));
        }
        Ok(())
    }

    /// compute the original positions
    pub fn compute_positions(&mut self) {
        self.reverse_map.clear();
        let mut source_code_line: i64 = 0;
        let mut source_code_column: i64 = 0;
        for &i in &self.map.order {
            let line = &self.map.lines[&i];
            if line.is_empty() || (line.len() == 1 && line[0].is_empty()) {
                continue;
            }
            let first: Vec<i64> = line[0].clone();
            let mut mapped: Line = line
                .iter()
                .enumerate()
                .map(|(index, segment)| {
                    if segment.is_empty() {
                        return Vec::new();
                    }
                    let generated = if index == 0 { segment[0] } else { segment[0] + first[0] };
                    let mut result = vec![generated];
                    if segment.len() <= 1 {
                        return result;
                    }
                    let source_file_index = if index == 0 { segment[1] } else { segment[1] + first[1] };
                    source_code_line += segment.get(2).copied().unwrap_or(0);
                    source_code_column += segment.get(3).copied().unwrap_or(0);
                    result.extend([source_file_index, source_code_line, source_code_column]);
                    result
                })
                .collect();
            mapped.sort_by(|a, b| {
                let (a1, b1) = (a.get(1), b.get(1));
                match (a1, b1) {
                    (Some(x), Some(y)) if x != y => x.cmp(y),
                    (Some(_), Some(_)) => a[0].cmp(&b[0]),
                    _ => std::cmp::Ordering::Equal,
                }
            });
            if mapped.is_empty() || (mapped.len() == 1 && mapped[0].is_empty()) {
                continue;
            }
            self.reverse_map.set(i, mapped);
        }
    }

    /// original sources, lines and columns of a position: `[source, line, column, content]`
    pub fn find(&mut self, line: i64, column: i64) -> Option<Vec<(Option<Rc<str>>, i64, i64, Option<Rc<str>>)>> {
        if self.reverse_map.is_empty() {
            self.compute_positions();
        }
        let line = line - 1;
        let records = self.reverse_map.get(line)?;
        let column = column - 1;
        let mut result = Vec::new();
        for record in records {
            if record.is_empty() || record[0] < column {
                continue;
            }
            if record[0] > column {
                break;
            }
            let index = record.get(1).copied().unwrap_or(0) as usize;
            result.push((
                self.sources.get(index).cloned().flatten(),
                record.get(2).copied().unwrap_or(0) + 1,
                record.get(3).copied().unwrap_or(0) + 1,
                self.sources_content.get(index).cloned().flatten(),
            ));
        }
        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    /// data url
    pub fn to_url(&self) -> String {
        format!("data:application/json;charset=utf-8;base64,{}", to_base64(&self.to_json().to_json()))
    }

    /// source map object
    pub fn to_json(&self) -> SourceMapObject {
        let mut mappings: Vec<String> = Vec::new();
        for i in 0..=self.line.max(-1) {
            if i < 0 {
                break;
            }
            mappings.push(match self.map.get(i) {
                None => String::new(),
                Some(line) => line.iter().map(|segment| encode(segment)).collect::<Vec<_>>().join(","),
            });
        }
        SourceMapObject {
            version: self.version,
            sources: self.sources.clone(),
            sources_content: self.sources_content.clone(),
            mappings: mappings.join(";"),
            ..Default::default()
        }
    }
}
