use crate::cxx_bridge::ffi::RuntimeResult;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeatureEmbedding {
    pub mean: f32,
    pub variance: f32,
    pub edge_density: f32,
    pub entropy: f32,
    pub saliency_score: f32,
    pub texture_score: f32,
    pub anomaly_score: f32,
    pub checksum: u64,
    pub input_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChangeDetectionResult {
    pub baseline_ready: bool,
    pub score: f32,
    pub changed: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct ChangeDetectorConfig {
    pub change_threshold: f32,
    pub high_confidence_threshold: f32,
}

#[derive(Clone, Debug)]
pub struct ChangeDetector {
    config: ChangeDetectorConfig,
    previous: Option<FeatureEmbedding>,
}

impl Default for ChangeDetectorConfig {
    fn default() -> Self {
        Self {
            change_threshold: 0.08,
            high_confidence_threshold: 0.35,
        }
    }
}

impl Default for ChangeDetector {
    fn default() -> Self {
        Self::new(ChangeDetectorConfig::default())
    }
}

impl ChangeDetector {
    pub fn new(config: ChangeDetectorConfig) -> Self {
        Self {
            config,
            previous: None,
        }
    }

    pub fn evaluate(&mut self, embedding: FeatureEmbedding) -> ChangeDetectionResult {
        let Some(previous) = self.previous else {
            self.previous = Some(embedding);
            return ChangeDetectionResult {
                baseline_ready: false,
                score: 0.0,
                changed: false,
            };
        };

        let score = embedding_distance(previous, embedding);
        let checksum_changed = previous.checksum != embedding.checksum;
        let changed = score >= self.config.high_confidence_threshold
            || (checksum_changed && score >= self.config.change_threshold);

        self.previous = Some(embedding);
        ChangeDetectionResult {
            baseline_ready: true,
            score,
            changed,
        }
    }
}

impl From<&RuntimeResult> for FeatureEmbedding {
    fn from(result: &RuntimeResult) -> Self {
        Self {
            mean: result.mean,
            variance: result.variance,
            edge_density: result.edge_density,
            entropy: result.entropy,
            saliency_score: result.saliency_score,
            texture_score: result.texture_score,
            anomaly_score: result.anomaly_score,
            checksum: result.checksum,
            input_bytes: result.input_bytes,
        }
    }
}

pub fn embedding_distance(previous: FeatureEmbedding, current: FeatureEmbedding) -> f32 {
    let mean_delta = (previous.mean - current.mean).abs() * 2.0;
    let variance_delta = (previous.variance - current.variance).abs() * 4.0;
    let edge_delta = (previous.edge_density - current.edge_density).abs();
    let entropy_delta = ((previous.entropy - current.entropy).abs() / 8.0).min(1.0);
    let saliency_delta = (previous.saliency_score - current.saliency_score).abs();
    let texture_delta = (previous.texture_score - current.texture_score).abs();
    let anomaly_delta = (previous.anomaly_score - current.anomaly_score).abs();
    let byte_delta = byte_delta_ratio(previous.input_bytes, current.input_bytes) * 0.5;

    mean_delta
        + variance_delta
        + edge_delta
        + entropy_delta
        + saliency_delta
        + texture_delta
        + anomaly_delta
        + byte_delta
}

fn byte_delta_ratio(previous: u64, current: u64) -> f32 {
    let max = previous.max(current);
    if max == 0 {
        return 0.0;
    }

    let min = previous.min(current);
    (max - min) as f32 / max as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn embedding(mean: f32, edge_density: f32, checksum: u64) -> FeatureEmbedding {
        FeatureEmbedding {
            mean,
            variance: 0.01,
            edge_density,
            entropy: 4.0,
            saliency_score: edge_density,
            texture_score: edge_density * 0.5,
            anomaly_score: edge_density * 0.75,
            checksum,
            input_bytes: 1024,
        }
    }

    #[test]
    fn first_embedding_initializes_baseline() {
        let mut detector = ChangeDetector::default();
        let result = detector.evaluate(embedding(0.4, 0.2, 1));

        assert!(!result.baseline_ready);
        assert!(!result.changed);
        assert_eq!(result.score, 0.0);
    }

    #[test]
    fn stable_embedding_does_not_trigger_change() {
        let mut detector = ChangeDetector::default();
        detector.evaluate(embedding(0.4, 0.2, 1));
        let result = detector.evaluate(embedding(0.401, 0.2, 1));

        assert!(result.baseline_ready);
        assert!(!result.changed);
        assert!(result.score < 0.08);
    }

    #[test]
    fn embedding_shift_triggers_change() {
        let mut detector = ChangeDetector::default();
        detector.evaluate(embedding(0.4, 0.2, 1));
        let result = detector.evaluate(embedding(0.5, 0.35, 2));

        assert!(result.baseline_ready);
        assert!(result.changed);
        assert!(result.score >= 0.08);
    }
}
