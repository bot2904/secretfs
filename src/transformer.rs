use regex::Regex;

use crate::config::SecretDef;
use crate::mapper::SecretMapper;

/// Handles transforming file content: replacing secrets with placeholders (on read)
/// and placeholders back with secrets (on write).
#[derive(Clone)]
pub struct Transformer {
    mapper: SecretMapper,
    /// Compiled regex patterns for dynamic secret detection.
    patterns: Vec<Regex>,
    /// Pre-registered literal secrets (stored for ordering during replacement).
    literals: Vec<String>,
}

/// Escape any existing placeholder-like sequences in original content so they
/// survive a round-trip without being mistaken for our placeholders.
const ESCAPE_MARKER: &str = "<|ESCAPED_SECRET:";
const PLACEHOLDER_PREFIX: &str = "<|SECRET:";
#[allow(dead_code)]
const PLACEHOLDER_SUFFIX: &str = "|>";

impl Transformer {
    /// Create a new transformer from config secret definitions.
    /// Literal secrets are immediately registered with the mapper.
    pub fn new(secrets: &[SecretDef], mapper: SecretMapper) -> Self {
        let mut patterns = Vec::new();
        let mut literals = Vec::new();

        for def in secrets {
            match def {
                SecretDef::Literal { literal } => {
                    mapper.register(literal);
                    literals.push(literal.clone());
                }
                SecretDef::Pattern { pattern } => {
                    // Unwrap is safe: config validation already checked these compile
                    patterns.push(Regex::new(pattern).unwrap());
                }
            }
        }

        // Sort literals by length descending so longer matches take priority
        literals.sort_by(|a, b| b.len().cmp(&a.len()));

        Self {
            mapper,
            patterns,
            literals,
        }
    }

    /// Transform content for reading: replace secrets with placeholders.
    pub fn filter_read(&self, content: &[u8]) -> Vec<u8> {
        // Work on the content as a byte string.
        // We'll do replacements on the full content as a string where possible,
        // and pass through non-UTF8 regions unchanged.
        //
        // Strategy: try to interpret as UTF-8. If it works, do string-based replacement.
        // If not, find ASCII-safe regions and replace within them.

        match std::str::from_utf8(content) {
            Ok(text) => self.filter_read_str(text).into_bytes(),
            Err(_) => self.filter_read_binary(content),
        }
    }

    /// Transform content for writing: replace placeholders back with secrets.
    pub fn filter_write(&self, content: &[u8]) -> Vec<u8> {
        match std::str::from_utf8(content) {
            Ok(text) => self.filter_write_str(text).into_bytes(),
            Err(_) => self.filter_write_binary(content),
        }
    }

    /// String-based read filter.
    fn filter_read_str(&self, text: &str) -> String {
        let mut result = text.to_string();

        // Step 1: Escape any pre-existing placeholder-like sequences in the original content.
        // This prevents them from being mistaken for our placeholders on write-back.
        result = result.replace(PLACEHOLDER_PREFIX, ESCAPE_MARKER);

        // Step 2: Replace literal secrets (longest first to avoid partial matches).
        for literal in &self.literals {
            if let Some(placeholder) = self.mapper.get_placeholder(literal) {
                result = result.replace(literal.as_str(), &placeholder);
            }
        }

        // Step 3: Replace regex pattern matches.
        for pattern in &self.patterns {
            let mut new_result = String::new();
            let mut last_end = 0;

            for m in pattern.find_iter(&result) {
                new_result.push_str(&result[last_end..m.start()]);
                let matched = m.as_str();
                let placeholder = self.mapper.register(matched);
                new_result.push_str(&placeholder);
                last_end = m.end();
            }
            new_result.push_str(&result[last_end..]);
            result = new_result;
        }

        result
    }

    /// String-based write filter.
    fn filter_write_str(&self, text: &str) -> String {
        let mut result = text.to_string();

        // Step 1: Replace all known placeholders with their secrets.
        let placeholders = self.mapper.all_placeholders();
        // Sort by placeholder length descending for safety
        let mut sorted = placeholders;
        sorted.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

        for (placeholder, secret) in &sorted {
            result = result.replace(placeholder.as_str(), secret);
        }

        // Step 2: Unescape any escaped placeholder sequences.
        result = result.replace(ESCAPE_MARKER, PLACEHOLDER_PREFIX);

        result
    }

    /// Binary content read filter: find and replace within ASCII-compatible regions.
    fn filter_read_binary(&self, content: &[u8]) -> Vec<u8> {
        // Strategy: split content into chunks at non-text boundaries,
        // process text-like chunks through string replacement.
        let regions = find_text_regions(content);
        let mut result = content.to_vec();

        // Process regions in reverse order so byte offsets remain valid
        for (start, end) in regions.into_iter().rev() {
            let region = &content[start..end];
            if let Ok(text) = std::str::from_utf8(region) {
                let filtered = self.filter_read_str(text);
                let filtered_bytes = filtered.as_bytes();
                result.splice(start..end, filtered_bytes.iter().copied());
            }
        }

        result
    }

    /// Binary content write filter.
    fn filter_write_binary(&self, content: &[u8]) -> Vec<u8> {
        let regions = find_text_regions(content);
        let mut result = content.to_vec();

        for (start, end) in regions.into_iter().rev() {
            let region = &result[start..end];
            if let Ok(text) = std::str::from_utf8(region) {
                let filtered = self.filter_write_str(text);
                let filtered_bytes = filtered.as_bytes();
                // We need to work on current result since we're modifying in place
                let new_result_len = result.len() - (end - start) + filtered_bytes.len();
                let mut new_result = Vec::with_capacity(new_result_len);
                new_result.extend_from_slice(&result[..start]);
                new_result.extend_from_slice(filtered_bytes);
                new_result.extend_from_slice(&result[end..]);
                result = new_result;
            }
        }

        result
    }
}

/// Find contiguous regions of text-like bytes (printable ASCII, UTF-8, common whitespace).
/// Returns (start, end) pairs. Minimum region length of 4 bytes to avoid noise.
fn find_text_regions(content: &[u8]) -> Vec<(usize, usize)> {
    let mut regions = Vec::new();
    let mut start = None;

    for (i, &byte) in content.iter().enumerate() {
        let is_text = byte == b'\n'
            || byte == b'\r'
            || byte == b'\t'
            || (byte >= 0x20 && byte < 0x7F)
            || byte >= 0x80; // could be UTF-8 continuation

        if is_text {
            if start.is_none() {
                start = Some(i);
            }
        } else {
            if let Some(s) = start {
                if i - s >= 4 {
                    regions.push((s, i));
                }
                start = None;
            }
        }
    }

    if let Some(s) = start {
        if content.len() - s >= 4 {
            regions.push((s, content.len()));
        }
    }

    regions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SecretDef;
    use crate::mapper::SecretMapper;

    fn make_transformer(literals: Vec<&str>, patterns: Vec<&str>) -> Transformer {
        let mut defs = Vec::new();
        for l in literals {
            defs.push(SecretDef::Literal {
                literal: l.to_string(),
            });
        }
        for p in patterns {
            defs.push(SecretDef::Pattern {
                pattern: p.to_string(),
            });
        }
        let mapper = SecretMapper::new();
        Transformer::new(&defs, mapper)
    }

    #[test]
    fn test_roundtrip_literals() {
        let t = make_transformer(vec!["my-api-key-123", "db-password-456"], vec![]);

        let original = b"connect to db with db-password-456 and use my-api-key-123 for auth";
        let filtered = t.filter_read(original);
        let filtered_str = std::str::from_utf8(&filtered).unwrap();

        assert!(!filtered_str.contains("my-api-key-123"));
        assert!(!filtered_str.contains("db-password-456"));
        assert!(filtered_str.contains("<|SECRET:"));

        // Round-trip
        let restored = t.filter_write(&filtered);
        assert_eq!(restored, original);
    }

    #[test]
    fn test_regex_pattern() {
        let t = make_transformer(vec![], vec![r"sk-[a-zA-Z0-9]{8}"]);

        let original = b"key is sk-abcd1234 here";
        let filtered = t.filter_read(original);
        let filtered_str = std::str::from_utf8(&filtered).unwrap();

        assert!(!filtered_str.contains("sk-abcd1234"));
        assert!(filtered_str.contains("<|SECRET:"));

        let restored = t.filter_write(&filtered);
        assert_eq!(restored, original);
    }

    #[test]
    fn test_preexisting_placeholder_escaped() {
        let t = make_transformer(vec!["secret123"], vec![]);

        let original = b"existing <|SECRET:BEEF|> and secret123 here";
        let filtered = t.filter_read(original);
        let filtered_str = std::str::from_utf8(&filtered).unwrap();

        // The original <|SECRET:BEEF|> should be escaped
        assert!(filtered_str.contains("<|ESCAPED_SECRET:BEEF|>"));
        // secret123 should be replaced
        assert!(!filtered_str.contains("secret123"));

        // Round-trip restores original
        let restored = t.filter_write(&filtered);
        assert_eq!(restored, original);
    }

    #[test]
    fn test_longest_match_first() {
        let t = make_transformer(vec!["secret", "secret-long"], vec![]);

        let original = b"value is secret-long here";
        let filtered = t.filter_read(original);
        let filtered_str = std::str::from_utf8(&filtered).unwrap();

        // "secret-long" should be matched, not "secret" + "-long"
        assert!(!filtered_str.contains("secret-long"));
        // Should only have one placeholder
        let count = filtered_str.matches("<|SECRET:").count();
        assert_eq!(count, 1);

        let restored = t.filter_write(&filtered);
        assert_eq!(restored, original);
    }
}
