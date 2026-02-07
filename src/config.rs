use serde::Deserialize;
use std::path::Path;

/// A single secret definition — either a literal string or a regex pattern.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum SecretDef {
    Literal {
        literal: String,
    },
    Pattern {
        pattern: String,
    },
}

/// Top-level configuration file structure.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub secrets: Vec<SecretDef>,
}

impl Config {
    /// Load configuration from a YAML file.
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let contents = std::fs::read_to_string(path)?;
        let config: Config = serde_yaml::from_str(&contents)?;

        // Validate: at least one secret
        if config.secrets.is_empty() {
            return Err("config must contain at least one secret definition".into());
        }

        // Validate regex patterns compile
        for secret in &config.secrets {
            if let SecretDef::Pattern { pattern } = secret {
                regex::Regex::new(pattern).map_err(|e| {
                    format!("invalid regex pattern '{}': {}", pattern, e)
                })?;
            }
            // Validate literals are non-empty
            if let SecretDef::Literal { literal } = secret {
                if literal.is_empty() {
                    return Err("literal secret must not be empty".into());
                }
            }
        }

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config() {
        let yaml = r#"
secrets:
  - literal: "my-secret-key"
  - pattern: "sk-[a-zA-Z0-9]{32,}"
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.secrets.len(), 2);
    }
}
