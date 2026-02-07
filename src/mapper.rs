use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Placeholder format: <|SECRET:XXXX|> where XXXX is a zero-padded hex ID.
const PLACEHOLDER_PREFIX: &str = "<|SECRET:";
const PLACEHOLDER_SUFFIX: &str = "|>";

/// Bidirectional mapping between secrets and placeholder strings.
#[derive(Debug, Clone)]
pub struct SecretMapper {
    inner: Arc<RwLock<MapperInner>>,
}

#[derive(Debug)]
struct MapperInner {
    /// secret → placeholder
    secret_to_placeholder: HashMap<String, String>,
    /// placeholder → secret
    placeholder_to_secret: HashMap<String, String>,
    /// Next ID to assign
    next_id: u32,
}

impl SecretMapper {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(MapperInner {
                secret_to_placeholder: HashMap::new(),
                placeholder_to_secret: HashMap::new(),
                next_id: 1,
            })),
        }
    }

    /// Register a known secret and return its placeholder.
    /// If already registered, returns the existing placeholder.
    pub fn register(&self, secret: &str) -> String {
        // Fast path: check read lock first
        {
            let inner = self.inner.read().unwrap();
            if let Some(ph) = inner.secret_to_placeholder.get(secret) {
                return ph.clone();
            }
        }

        // Slow path: acquire write lock
        let mut inner = self.inner.write().unwrap();
        // Double-check after acquiring write lock
        if let Some(ph) = inner.secret_to_placeholder.get(secret) {
            return ph.clone();
        }

        let placeholder = format!("{}{:04X}{}", PLACEHOLDER_PREFIX, inner.next_id, PLACEHOLDER_SUFFIX);
        inner.next_id += 1;
        inner
            .secret_to_placeholder
            .insert(secret.to_string(), placeholder.clone());
        inner
            .placeholder_to_secret
            .insert(placeholder.clone(), secret.to_string());
        placeholder
    }

    /// Get the placeholder for a known secret, if registered.
    pub fn get_placeholder(&self, secret: &str) -> Option<String> {
        let inner = self.inner.read().unwrap();
        inner.secret_to_placeholder.get(secret).cloned()
    }

    /// Get the secret for a known placeholder, if registered.
    #[allow(dead_code)]
    pub fn get_secret(&self, placeholder: &str) -> Option<String> {
        let inner = self.inner.read().unwrap();
        inner.placeholder_to_secret.get(placeholder).cloned()
    }

    /// Return all placeholder→secret mappings (for reverse transformation).
    pub fn all_placeholders(&self) -> Vec<(String, String)> {
        let inner = self.inner.read().unwrap();
        inner
            .placeholder_to_secret
            .iter()
            .map(|(p, s)| (p.clone(), s.clone()))
            .collect()
    }

    /// Return all secret→placeholder mappings (for forward transformation).
    #[allow(dead_code)]
    pub fn all_secrets(&self) -> Vec<(String, String)> {
        let inner = self.inner.read().unwrap();
        inner
            .secret_to_placeholder
            .iter()
            .map(|(s, p)| (s.clone(), p.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_and_lookup() {
        let mapper = SecretMapper::new();
        let ph1 = mapper.register("secret-key-1");
        let ph2 = mapper.register("secret-key-2");

        assert_eq!(ph1, "<|SECRET:0001|>");
        assert_eq!(ph2, "<|SECRET:0002|>");

        // Same secret returns same placeholder
        assert_eq!(mapper.register("secret-key-1"), ph1);

        // Reverse lookup
        assert_eq!(mapper.get_secret(&ph1), Some("secret-key-1".to_string()));
        assert_eq!(mapper.get_placeholder("secret-key-2"), Some(ph2.clone()));
    }
}
