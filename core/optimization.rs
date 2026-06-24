#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OptimizationProfile {
    pub target_execution_p95_ms: u64,
    pub target_runtime_p95_ms: u64,
    pub min_throughput_tps: f32,
    pub max_defer_rate: f32,
    pub max_reject_rate: f32,
    pub max_memory_percent: f32,
    pub max_temperature_c: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OptimizationMetrics {
    pub tasks: usize,
    pub elapsed_ms: u128,
    pub admitted: usize,
    pub deferred: usize,
    pub rejected: usize,
    pub executed: usize,
    pub execution_p50_ms: u64,
    pub execution_p95_ms: u64,
    pub runtime_p95_ms: u64,
    pub peak_memory_percent: f32,
    pub peak_temperature_c: f32,
    pub max_queue_pressure: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OptimizationRecommendation {
    MaintainCurrentProfile,
    ReduceInputResolution,
    LowerVlmSoftMemoryGate,
    ReduceSourceRate,
    IncreasePoolSlots,
    InvestigateRuntimeBackend,
    ApplyJetsonHardening,
}

impl Default for OptimizationProfile {
    fn default() -> Self {
        Self {
            target_execution_p95_ms: 120,
            target_runtime_p95_ms: 20,
            min_throughput_tps: 20.0,
            max_defer_rate: 0.35,
            max_reject_rate: 0.15,
            max_memory_percent: 82.0,
            max_temperature_c: 78.0,
        }
    }
}

impl OptimizationMetrics {
    pub fn throughput_tps(self) -> f32 {
        if self.elapsed_ms == 0 {
            return 0.0;
        }

        (self.executed as f32 * 1000.0) / self.elapsed_ms as f32
    }

    pub fn admission_rate(self) -> f32 {
        ratio(self.admitted, self.tasks)
    }

    pub fn defer_rate(self) -> f32 {
        ratio(self.deferred, self.tasks)
    }

    pub fn reject_rate(self) -> f32 {
        ratio(self.rejected, self.tasks)
    }

    pub fn recommend(self, profile: OptimizationProfile) -> Vec<OptimizationRecommendation> {
        let mut recommendations = Vec::new();

        if self.peak_temperature_c >= profile.max_temperature_c {
            recommendations.push(OptimizationRecommendation::ApplyJetsonHardening);
        }

        if self.peak_memory_percent >= profile.max_memory_percent {
            recommendations.push(OptimizationRecommendation::LowerVlmSoftMemoryGate);
            recommendations.push(OptimizationRecommendation::ReduceInputResolution);
        }

        if self.execution_p95_ms > profile.target_execution_p95_ms {
            recommendations.push(OptimizationRecommendation::ReduceInputResolution);
        }

        if self.runtime_p95_ms > profile.target_runtime_p95_ms {
            recommendations.push(OptimizationRecommendation::InvestigateRuntimeBackend);
        }

        if self.defer_rate() > profile.max_defer_rate {
            recommendations.push(OptimizationRecommendation::IncreasePoolSlots);
            recommendations.push(OptimizationRecommendation::ReduceSourceRate);
        }

        if self.reject_rate() > profile.max_reject_rate {
            recommendations.push(OptimizationRecommendation::LowerVlmSoftMemoryGate);
        }

        if self.throughput_tps() < profile.min_throughput_tps && self.tasks > 0 {
            recommendations.push(OptimizationRecommendation::ReduceSourceRate);
        }

        dedupe_recommendations(&mut recommendations);

        if recommendations.is_empty() {
            recommendations.push(OptimizationRecommendation::MaintainCurrentProfile);
        }

        recommendations
    }
}

impl OptimizationRecommendation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MaintainCurrentProfile => "maintain_current_profile",
            Self::ReduceInputResolution => "reduce_input_resolution",
            Self::LowerVlmSoftMemoryGate => "lower_vlm_soft_memory_gate",
            Self::ReduceSourceRate => "reduce_source_rate",
            Self::IncreasePoolSlots => "increase_pool_slots",
            Self::InvestigateRuntimeBackend => "investigate_runtime_backend",
            Self::ApplyJetsonHardening => "apply_jetson_hardening",
        }
    }
}

pub fn percentile_u64(values: &[u64], percentile: usize) -> u64 {
    if values.is_empty() {
        return 0;
    }

    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let index = ((sorted.len() - 1) * percentile) / 100;
    sorted[index]
}

fn ratio(numerator: usize, denominator: usize) -> f32 {
    if denominator == 0 {
        return 0.0;
    }

    numerator as f32 / denominator as f32
}

fn dedupe_recommendations(recommendations: &mut Vec<OptimizationRecommendation>) {
    let mut deduped = Vec::with_capacity(recommendations.len());
    for recommendation in recommendations.iter().copied() {
        if !deduped.contains(&recommendation) {
            deduped.push(recommendation);
        }
    }
    *recommendations = deduped;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn baseline() -> OptimizationMetrics {
        OptimizationMetrics {
            tasks: 100,
            elapsed_ms: 4_000,
            admitted: 90,
            deferred: 5,
            rejected: 5,
            executed: 90,
            execution_p50_ms: 20,
            execution_p95_ms: 50,
            runtime_p95_ms: 5,
            peak_memory_percent: 60.0,
            peak_temperature_c: 55.0,
            max_queue_pressure: 10,
        }
    }

    #[test]
    fn healthy_metrics_maintain_current_profile() {
        let recommendations = baseline().recommend(OptimizationProfile::default());

        assert_eq!(
            recommendations,
            vec![OptimizationRecommendation::MaintainCurrentProfile]
        );
    }

    #[test]
    fn high_memory_recommends_vlm_and_input_tuning() {
        let mut metrics = baseline();
        metrics.peak_memory_percent = 86.0;

        let recommendations = metrics.recommend(OptimizationProfile::default());

        assert!(recommendations.contains(&OptimizationRecommendation::LowerVlmSoftMemoryGate));
        assert!(recommendations.contains(&OptimizationRecommendation::ReduceInputResolution));
    }

    #[test]
    fn high_defer_rate_recommends_pool_and_source_tuning() {
        let mut metrics = baseline();
        metrics.deferred = 60;

        let recommendations = metrics.recommend(OptimizationProfile::default());

        assert!(recommendations.contains(&OptimizationRecommendation::IncreasePoolSlots));
        assert!(recommendations.contains(&OptimizationRecommendation::ReduceSourceRate));
    }

    #[test]
    fn percentile_handles_empty_and_sorted_values() {
        assert_eq!(percentile_u64(&[], 95), 0);
        assert_eq!(percentile_u64(&[10, 1, 5, 7], 50), 5);
        assert_eq!(percentile_u64(&[10, 1, 5, 7], 95), 7);
    }
}
