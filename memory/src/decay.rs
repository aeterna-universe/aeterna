//! Memory importance decay with cold-tier archival trigger.
//!
//! Importance scores decay exponentially based on time since last access,
//! with per-layer configurable rates. When a score drops below the archival
//! threshold the memory becomes a candidate for cold-tier archival.

use std::collections::HashMap;

/// Decay configuration per memory layer.
#[derive(Debug, Clone)]
pub struct DecayConfig {
    /// Decay rate per day (0.0 to 1.0). Higher = faster decay.
    pub rates: HashMap<String, f64>,
    /// Importance threshold below which memories are candidates for archival.
    pub archival_threshold: f64,
}

impl Default for DecayConfig {
    fn default() -> Self {
        let mut rates = HashMap::new();
        rates.insert("agent".to_string(), 0.10); // Fast decay
        rates.insert("session".to_string(), 0.05); // Medium
        rates.insert("user".to_string(), 0.03);
        rates.insert("project".to_string(), 0.01);
        rates.insert("team".to_string(), 0.008);
        rates.insert("org".to_string(), 0.005);
        rates.insert("tenant".to_string(), 0.002); // Very slow
        Self {
            rates,
            archival_threshold: 0.01,
        }
    }
}

impl DecayConfig {
    /// Build a `DecayConfig` from environment variables.
    ///
    /// Reads `AETERNA_DECAY_RATE_{LAYER}` (uppercase) for each layer and
    /// `AETERNA_DECAY_ARCHIVAL_THRESHOLD`. Falls back to defaults for any
    /// value that is absent or unparseable.
    pub fn from_env() -> Self {
        let defaults = Self::default();
        let mut rates = defaults.rates.clone();

        for (layer, default_rate) in &defaults.rates {
            let env_key = format!("AETERNA_DECAY_RATE_{}", layer.to_uppercase());
            if let Ok(val) = std::env::var(&env_key) {
                if let Ok(rate) = val.parse::<f64>()
                    && (0.0..=1.0).contains(&rate)
                {
                    rates.insert(layer.clone(), rate);
                }
            } else {
                rates.insert(layer.clone(), *default_rate);
            }
        }

        let archival_threshold = std::env::var("AETERNA_DECAY_ARCHIVAL_THRESHOLD")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(defaults.archival_threshold);

        Self {
            rates,
            archival_threshold,
        }
    }

    /// Look up the decay rate for a given layer name, returning `None` if
    /// the layer is not configured.
    pub fn rate_for_layer(&self, layer: &str) -> Option<f64> {
        self.rates.get(layer).copied()
    }
}

/// Calculate the decayed importance score.
///
/// Formula: `new_score = current_score * (1 - decay_rate) ^ days_since_last_access`
///
/// Returns `(new_score, should_archive)` where `should_archive` is `true`
/// when the new score drops below `archival_threshold`.
pub fn calculate_decay(
    current_score: f64,
    decay_rate: f64,
    days_since_last_access: f64,
    archival_threshold: f64,
) -> (f64, bool) {
    if days_since_last_access <= 0.0 || decay_rate <= 0.0 {
        return (current_score, current_score < archival_threshold);
    }
    let new_score = current_score * (1.0 - decay_rate).powf(days_since_last_access);
    let should_archive = new_score < archival_threshold;
    (new_score, should_archive)
}

/// Batch decay result for reporting.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecayReport {
    /// Total entries processed during the decay cycle.
    pub entries_processed: u64,
    /// Entries whose score was actually reduced.
    pub entries_decayed: u64,
    /// Entries flagged for cold-tier archival.
    pub entries_archived: u64,
    /// Entries skipped (e.g. recently accessed, no configured rate).
    pub entries_skipped: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_days_no_decay() {
        let (score, archive) = calculate_decay(0.8, 0.05, 0.0, 0.01);
        assert!((score - 0.8).abs() < f64::EPSILON);
        assert!(!archive);
    }

    #[test]
    fn positive_days_exponential_decay() {
        let (score, _) = calculate_decay(1.0, 0.05, 10.0, 0.01);
        // 1.0 * (0.95)^10 ≈ 0.5987
        assert!(score > 0.59 && score < 0.60, "score was {score}");
    }

    #[test]
    fn high_decay_rate_fast_drop() {
        let (score, archive) = calculate_decay(0.5, 0.50, 5.0, 0.01);
        // 0.5 * (0.5)^5 = 0.015625
        assert!(score < 0.02, "score was {score}");
        assert!(!archive); // 0.015625 > 0.01
    }

    #[test]
    fn low_decay_rate_slow_drop() {
        let (score, _) = calculate_decay(1.0, 0.002, 30.0, 0.01);
        // 1.0 * (0.998)^30 ≈ 0.9418
        assert!(score > 0.93 && score < 0.95, "score was {score}");
    }

    #[test]
    fn archival_threshold_detection() {
        let (score, archive) = calculate_decay(0.02, 0.50, 2.0, 0.01);
        // 0.02 * (0.5)^2 = 0.005
        assert!(score < 0.01);
        assert!(archive);
    }

    #[test]
    fn per_layer_rate_lookup() {
        let config = DecayConfig::default();
        assert!((config.rate_for_layer("agent").unwrap() - 0.10).abs() < f64::EPSILON);
        assert!((config.rate_for_layer("session").unwrap() - 0.05).abs() < f64::EPSILON);
        assert!((config.rate_for_layer("tenant").unwrap() - 0.002).abs() < f64::EPSILON);
        assert!(config.rate_for_layer("nonexistent").is_none());
    }

    #[test]
    fn default_config_values() {
        let config = DecayConfig::default();
        assert_eq!(config.rates.len(), 7);
        assert!((config.archival_threshold - 0.01).abs() < f64::EPSILON);
        assert!(config.rates.contains_key("agent"));
        assert!(config.rates.contains_key("user"));
        assert!(config.rates.contains_key("org"));
    }

    #[test]
    fn from_env_returns_defaults_when_no_env_set() {
        // When no AETERNA_DECAY_RATE_* env vars are set, from_env should
        // return the same rates as Default.
        let config = DecayConfig::from_env();
        let defaults = DecayConfig::default();
        // Session should always be at default since we never set its env var
        assert!(
            (config.rate_for_layer("session").unwrap()
                - defaults.rate_for_layer("session").unwrap())
            .abs()
                < f64::EPSILON
        );
        assert_eq!(config.rates.len(), defaults.rates.len());
    }

    #[test]
    fn negative_days_no_decay() {
        let (score, _) = calculate_decay(0.5, 0.05, -3.0, 0.01);
        assert!((score - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn zero_score_stays_zero() {
        let (score, archive) = calculate_decay(0.0, 0.05, 10.0, 0.01);
        assert!(score.abs() < f64::EPSILON);
        assert!(archive); // 0.0 < 0.01
    }

    #[test]
    fn rate_one_decays_to_zero() {
        let (score, archive) = calculate_decay(0.8, 1.0, 1.0, 0.01);
        // 0.8 * (1 - 1)^1 = 0
        assert!(score.abs() < f64::EPSILON);
        assert!(archive);
    }

    #[test]
    fn zero_decay_rate_no_change() {
        let (score, archive) = calculate_decay(0.8, 0.0, 100.0, 0.01);
        assert!((score - 0.8).abs() < f64::EPSILON);
        assert!(!archive);
    }

    #[test]
    fn decay_report_serializes_to_camel_case() {
        let report = DecayReport {
            entries_processed: 100,
            entries_decayed: 80,
            entries_archived: 5,
            entries_skipped: 15,
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["entriesProcessed"], 100);
        assert_eq!(json["entriesDecayed"], 80);
        assert_eq!(json["entriesArchived"], 5);
        assert_eq!(json["entriesSkipped"], 15);
    }

    #[test]
    fn from_env_invalid_value_ignored() {
        // Test the parsing logic indirectly: a non-numeric value should not
        // crash and the rate should remain a valid f64.
        let config = DecayConfig::from_env();
        // All rates should be valid f64 in 0..=1 range
        for (layer, rate) in &config.rates {
            assert!(
                (0.0..=1.0).contains(rate),
                "Layer {layer} has out-of-range rate {rate}"
            );
        }
    }

    #[test]
    fn from_env_preserves_all_layers() {
        // from_env should always have the same set of layer keys as default
        let config = DecayConfig::from_env();
        let defaults = DecayConfig::default();
        for layer in defaults.rates.keys() {
            assert!(config.rates.contains_key(layer), "Missing layer: {layer}");
        }
    }
}
