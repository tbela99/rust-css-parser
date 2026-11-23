//! Port of `src/printer/lib/codec.ts` (vlq, from <https://github.com/Rich-Harris/vlq,> credit: Rich Harris).

const CHARS: &[u8; 65] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=";

fn char_to_integer(c: u8) -> i32 {
    CHARS.iter().position(|&x| x == c).map_or(0, |p| p as i32)
}

pub fn decode(str: &str) -> Vec<i64> {
    let mut result = Vec::new();
    let mut shift: u32 = 0;
    let mut value: i32 = 0;
    for &c in str.as_bytes() {
        let mut integer = char_to_integer(c);
        let has_continuation_bit = integer & 32;
        integer &= 31;
        value = value.wrapping_add(integer.wrapping_shl(shift));
        if has_continuation_bit != 0 {
            shift += 5;
        } else {
            let should_negate = value & 1;
            let v = (value as u32) >> 1;
            if should_negate != 0 {
                result.push(if v == 0 { -0x80000000 } else { -(v as i64) });
            } else {
                result.push(v as i64);
            }
            value = 0;
            shift = 0;
        }
    }
    result
}

pub fn encode(value: &[i64]) -> String {
    let mut result = String::new();
    for &v in value {
        encode_integer(v, &mut result);
    }
    result
}

fn encode_integer(num: i64, result: &mut String) {
    let mut num: u32 = if num < 0 { ((-num as i32) << 1 | 1) as u32 } else { ((num as i32) << 1) as u32 };
    loop {
        let mut clamped = num & 31;
        num >>= 5;
        if num > 0 {
            clamped |= 32;
        }
        result.push(CHARS[clamped as usize] as char);
        if num == 0 {
            break;
        }
    }
}
