//! Port of `src/fs/resolve.ts`: path and url helpers.

use std::cell::RefCell;
use crate::utils::fx::HashMap;

/// `matchUrl`: `/^(https?:)?\/\//`
pub fn match_url(url: &str) -> bool {
    url.starts_with("//") || url.starts_with("http://") || url.starts_with("https://")
}

/// `/^\/?[a-zA-Z]:/`: the length of the windows drive prefix, 0 when none
fn windows_prefix(path: &str) -> usize {
    let b = path.as_bytes();
    let start = if b.first() == Some(&b'/') { 1 } else { 0 };
    if b.len() > start + 1 && b[start].is_ascii_alphabetic() && b[start + 1] == b':' {
        start + 2
    } else {
        0
    }
}

/// the directory name of a path
pub fn dirname(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    if path.starts_with("data:") {
        return path.to_string();
    }
    let mut parts: Vec<&str> = path.split('/').collect();
    parts.pop();
    parts.join("/")
}

fn split_path(result: &str) -> Vec<String> {
    if result.is_empty() {
        return Vec::new();
    }
    let mut parts: Vec<String> = if result == "/" { Vec::new() } else { vec![String::new()] };
    for c in result.chars() {
        if c == '/' {
            parts.push(String::new());
        } else {
            if parts.is_empty() {
                // typescript: `parts[-1] += chr` is not reachable for "/" (the string has a single character)
                parts.push(String::new());
            }
            parts.last_mut().unwrap().push(c);
        }
    }
    parts
}

thread_local! {
    static NORMALIZED: RefCell<HashMap<String, String>> = RefCell::new(HashMap::default());
    static RESOLVED: RefCell<HashMap<(String, String, String), ResolvedPath>> = RefCell::new(HashMap::default());
}

/// normalize a path: backslashes, windows drive, query and fragment, `..`
pub fn normalize(path: &str) -> String {
    if let Some(n) = NORMALIZED.with(|c| c.borrow().get(path).cloned()) {
        return n;
    }
    let mut p = path.replace('\\', "/");
    let w = windows_prefix(&p);
    if w > 0 {
        p = p[w..].to_string();
    }
    let mut parts: Vec<String> = Vec::new();
    for c in p.chars() {
        if c == '/' {
            if parts.last().is_none_or(|l| !l.is_empty()) {
                parts.push(String::new());
            }
        } else if c == '?' || c == '#' {
            break;
        } else {
            if parts.is_empty() {
                parts.push(String::new());
            }
            parts.last_mut().unwrap().push(c);
        }
    }
    let mut k: isize = -1;
    loop {
        k += 1;
        if k as usize >= parts.len() {
            break;
        }
        if k > 0 && parts[k as usize] == ".." {
            parts.drain((k - 1) as usize..(k + 1) as usize);
            k -= 2;
        }
    }
    let result = format!("{}{}", if p.starts_with('/') { "/" } else { "" }, parts.join("/"));
    NORMALIZED.with(|c| c.borrow_mut().insert(path.to_string(), result.clone()));
    result
}

/// relative path from a directory
pub fn diff(path1: &str, path2: &str) -> String {
    let mut parts = split_path(path1);
    for p in split_path(path2) {
        if parts.first() == Some(&p) {
            parts.remove(0);
        } else {
            parts.insert(0, "..".into());
        }
    }
    parts.join("/")
}

/// absolute and relative path of a resolved url
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedPath {
    pub absolute: String,
    pub relative: String,
}

/// resolve a url relative to a directory (`cwd` is optional: "")
pub fn resolve(url: &str, current_directory: &str, cwd: Option<&str>) -> ResolvedPath {
    let key = (url.to_string(), current_directory.to_string(), cwd.unwrap_or("\u{0}undefined").to_string());
    if let Some(r) = RESOLVED.with(|c| c.borrow().get(&key).cloned()) {
        return r;
    }
    let result = resolve_uncached(url, current_directory, cwd.unwrap_or(""));
    RESOLVED.with(|c| c.borrow_mut().insert(key, result.clone()));
    result
}

fn resolve_uncached(url: &str, current_directory: &str, cwd: &str) -> ResolvedPath {
    if match_url(url) {
        return ResolvedPath { absolute: url.to_string(), relative: url.to_string() };
    }
    let url = normalize(url);
    let cwd = if cwd.is_empty() { String::new() } else { normalize(cwd) };
    let current_directory = if current_directory.is_empty() { String::new() } else { normalize(current_directory) };
    let mut dir = if cwd.is_empty() { current_directory } else { cwd };
    let w = windows_prefix(&dir);
    if w > 0 {
        dir = dir[w..].to_string();
    }
    let absolute = if dir.is_empty() || url.starts_with('/') || url.starts_with(&dir) || windows_prefix(&url) > 0 {
        resolve_path(&[&url])
    } else {
        resolve_path(&[&dir, &url])
    };
    let relative = if dir.is_empty() { absolute.clone() } else { diff(&absolute, &dir) };
    ResolvedPath { absolute, relative }
}

fn resolve_path(parts: &[&str]) -> String {
    let path = parts.iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join("/");
    let is_absolute = path.starts_with('/') || path.starts_with('\\');
    let mut resolved: Vec<&str> = Vec::new();
    for segment in path.split(['/', '\\']) {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            if !resolved.is_empty() && *resolved.last().unwrap() != ".." {
                resolved.pop();
            } else if !is_absolute {
                resolved.push("..");
            }
        } else {
            resolved.push(segment);
        }
    }
    let mut result = resolved.join("/");
    if is_absolute {
        result = format!("/{}", result);
    }
    if result.is_empty() {
        if is_absolute { "/".into() } else { ".".into() }
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// reference values from the typescript build
    #[test]
    fn resolve_parity() {
        let cases: [(&str, &str, Option<&str>, &str, &str, &str); 8] = [
            ("a.css", "/x/y", None, "/x/y/a.css", "a.css", ""),
            ("../a.css", "/x/y", None, "/x/a.css", "../a.css", ".."),
            ("/a/b.css", "/x", None, "/a/b.css", "../a/b.css", "/a"),
            ("https://a.b/c.css", "x", None, "https://a.b/c.css", "https://a.b/c.css", "https://a.b"),
            ("./a/../b.css", "", None, "b.css", "b.css", "./a/.."),
            ("C:\\x\\y.css", "/z", None, "/x/y.css", "../x/y.css", ""),
            ("a.css", "x/y", Some("/cwd"), "/cwd/a.css", "a.css", ""),
            ("../../q.css?x#y", "a/b/c", None, "a/b/c/q.css", "q.css", "../.."),
        ];
        for (url, dir, cwd, absolute, relative, dn) in cases {
            let r = resolve(url, dir, cwd);
            assert_eq!((r.absolute.as_str(), r.relative.as_str()), (absolute, relative), "{}", url);
            assert_eq!(dirname(url), dn, "{}", url);
        }
    }
}
