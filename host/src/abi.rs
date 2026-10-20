//! Values on the command line and in test scripts, and their Ethereum ABI
//! encoding as a tuple.
//!
//! A value is one token: a 32-byte word (`42`, `-1`, `true`, `0xHEX`), a
//! `string` (`"text"`), or a `bytes` (`hex"00ff"`).

use crate::parse_word;
use prova_common::Word;

pub enum Value {
    Word(Word),
    /// `string` or `bytes`: the length in the head's offset, then the data.
    Dynamic(Vec<u8>),
}

/// Splits on whitespace, but keeps `"..."` (and `hex"..."`) together.
pub fn tokenize(text: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        let mut token = String::new();
        let mut quoted = false;
        while let Some(&c) = chars.peek() {
            if c == '"' {
                quoted = !quoted;
            } else if c.is_whitespace() && !quoted {
                break;
            }
            token.push(c);
            chars.next();
        }
        if quoted {
            return Err(format!("unterminated string in `{text}`"));
        }
        tokens.push(token);
    }
    Ok(tokens)
}

pub fn parse_value(token: &str) -> Result<Value, String> {
    if let Some(text) = token.strip_prefix("hex\"") {
        let digits = text
            .strip_suffix('"')
            .filter(|d| d.len() % 2 == 0)
            .ok_or_else(|| format!("`{token}` is not hex\"\" with an even number of digits"))?;
        let bytes = (0..digits.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&digits[i..i + 2], 16))
            .collect::<Result<_, _>>()
            .map_err(|_| format!("`{token}` has a digit that isn't hex"))?;
        return Ok(Value::Dynamic(bytes));
    }
    if let Some(text) = token.strip_prefix('"') {
        let text = text
            .strip_suffix('"')
            .ok_or_else(|| format!("`{token}` is not a \"string\""))?;
        return Ok(Value::Dynamic(text.as_bytes().to_vec()));
    }
    Ok(Value::Word(parse_word(token)?))
}

pub fn parse_values<'a>(tokens: impl IntoIterator<Item = &'a str>) -> Result<Vec<Value>, String> {
    tokens.into_iter().map(parse_value).collect()
}

/// The tuple encoding: words in the head, and for each dynamic value an
/// offset in the head pointing to its length and padded data in the tail.
pub fn encode(values: &[Value]) -> Vec<u8> {
    let mut head = Vec::new();
    let mut tail = Vec::new();
    for value in values {
        match value {
            Value::Word(word) => head.extend_from_slice(word),
            Value::Dynamic(bytes) => {
                head.extend_from_slice(&number(32 * values.len() + tail.len()));
                tail.extend_from_slice(&number(bytes.len()));
                tail.extend_from_slice(bytes);
                tail.resize(tail.len().next_multiple_of(32), 0);
            }
        }
    }
    head.extend_from_slice(&tail);
    head
}

fn number(n: usize) -> Word {
    let mut word = [0; 32];
    word[24..].copy_from_slice(&(n as u64).to_be_bytes());
    word
}
