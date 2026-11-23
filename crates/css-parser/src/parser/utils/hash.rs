//! Port of `src/parser/utils/hash.ts`.
//!
//! The typescript `hash()` is async because of `crypto.subtle.digest()`: the rust digests are synchronous, `hash()` and
//! `sync_hash()` share the implementation (`sync_hash()` rejects the sha algorithms, like `syncHash()`).

use crate::ast::arena::AstArena;
use base64::Engine;
use sha1::Digest;

pub const LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
pub const DIGITS: &str = "0123456789";

/// supported hash algorithms
pub const HASH_ALGORITHMS: [&str; 7] = ["hex", "base64", "base64url", "sha1", "sha256", "sha384", "sha512"];

const FULL_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
const FIRST_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz";

/// simple deterministic hash
fn hash_code(str: &str) -> u32 {
    let mut hash: u32 = 0;
    for c in str.encode_utf16() {
        hash = hash.wrapping_mul(31).wrapping_add(c as u32);
    }
    hash
}

/// generate a hash id
pub fn hash_id(input: &str, length: usize) -> String {
    let mut n = hash_code(input) as u64;
    let mut chars: Vec<u8> = Vec::with_capacity(length);
    chars.push(FIRST_ALPHABET[(n % FIRST_ALPHABET.len() as u64) as usize]);
    for i in 1..length {
        n = (n + (chars.len() * i) as u64) % FULL_ALPHABET.len() as u64;
        chars.push(FULL_ALPHABET[n as usize]);
    }
    String::from_utf8(chars).unwrap()
}

/// structural signature of a node or a token: type, name, values, tokens and children
pub fn node_signature(arena: &AstArena, id: u32) -> String {
    let mut result = format!(
        "{}:{}:{}:{}:{}:{}",
        arena.data_types.get(id),
        arena.name.get(id),
        arena.string_value.get(id),
        crate::utils::number::js_number_to_string(arena.numeric_value.get(id)),
        arena.unit.get(id),
        arena.color_type.get(id)
    );
    if arena.first_token.get(id) != 0 {
        result.push('(');
        for token in arena.token_iter(id) {
            result.push_str(&node_signature(arena, token));
            result.push(',');
        }
        result.push(')');
    }
    if arena.first_child.get(id) != 0 {
        result.push('{');
        for child in arena.child_iter(id) {
            result.push_str(&node_signature(arena, child));
            result.push(',');
        }
        result.push('}');
    }
    result
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// hex of the code points of a string (`charCodeAt(0)` of each character)
fn string_to_hex(input: &str) -> String {
    let mut result = String::new();
    for c in input.chars() {
        let code = c.encode_utf16(&mut [0u16; 2])[0];
        result.push_str(&format!("{:02x}", code));
    }
    result
}

/// `String.prototype.slice(0, length)`
fn take(s: String, length: usize) -> String {
    if s.len() <= length {
        s
    } else {
        s[..length].to_string()
    }
}

/// `btoa()`: None when a character is not latin1 (`btoa()` throws)
fn btoa(input: &str) -> Result<String, String> {
    let mut bytes = Vec::with_capacity(input.len());
    for c in input.chars() {
        if (c as u32) > 0xff {
            return Err("Invalid character".into());
        }
        bytes.push(c as u32 as u8);
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn hash_impl(input: &str, length: usize, algo: Option<&str>, allow_sha: bool) -> Result<String, String> {
    let Some(algo) = algo else {
        return Ok(hash_id(input, length));
    };
    match algo {
        "hex" => Ok(take(string_to_hex(input), length)),
        "base64" | "base64url" => {
            let mut result = btoa(input)?;
            if algo == "base64url" {
                result = result.replace('+', "-").replace('/', "_").trim_end_matches('=').to_string();
            }
            Ok(take(result, length))
        }
        "sha1" | "sha256" | "sha384" | "sha512" if allow_sha => {
            let bytes = input.as_bytes();
            let digest = match algo {
                "sha1" => sha1::Sha1::digest(bytes).to_vec(),
                "sha256" => sha2::Sha256::digest(bytes).to_vec(),
                "sha384" => sha2::Sha384::digest(bytes).to_vec(),
                _ => sha2::Sha512::digest(bytes).to_vec(),
            };
            Ok(take(bytes_to_hex(&digest), length))
        }
        _ => Err(format!("Unsupported hash algorithm: {}", algo)),
    }
}

/// generate a hash (`hash()`)
pub fn hash(input: &str, length: usize, algo: Option<&str>) -> Result<String, String> {
    hash_impl(input, length, algo, true)
}

/// generate a hash (`syncHash()`): the sha algorithms are not supported
pub fn sync_hash(input: &str, length: usize, algo: Option<&str>) -> Result<String, String> {
    hash_impl(input, length, algo, false)
}
