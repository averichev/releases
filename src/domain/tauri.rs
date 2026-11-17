//! Tauri CLI public-key envelope. Private-key packets have a different size/format.
use base64::{Engine, engine::general_purpose::STANDARD};
pub fn public_key(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 4096 {
        return None;
    }
    let decoded = STANDARD.decode(value).ok()?;
    let text = std::str::from_utf8(&decoded).ok()?;
    let mut lines = text.lines();
    if !lines.next()?.starts_with("untrusted comment: ") {
        return None;
    }
    let packet = STANDARD.decode(lines.next()?).ok()?;
    if lines.next().is_some() || packet.len() != 42 || &packet[..2] != b"Ed" {
        return None;
    }
    Some(value.to_owned())
}
