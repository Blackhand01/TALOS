#include "runtime/backend.hpp"

#include <chrono>

#include "runtime/cv/features.hpp"
#include "runtime/vlm/query.hpp"
#include "talos/ipc/cxx_bridge.rs.h"

namespace talos::runtime {

RuntimeResult run(rust::Slice<const std::uint8_t> buffer) {
    return run_cv_features(buffer);
}

RuntimeResult run_cv_features(rust::Slice<const std::uint8_t> buffer) {
    const auto started = std::chrono::steady_clock::now();
    const cv::FeatureSummary features = cv::extract_features(buffer);
    const auto elapsed = std::chrono::steady_clock::now() - started;
    const auto elapsed_ms =
        std::chrono::duration_cast<std::chrono::milliseconds>(elapsed).count();

    RuntimeResult result;
    result.ok = features.ok;
    result.latency_ms = features.ok ? static_cast<std::uint64_t>(elapsed_ms < 1 ? 1 : elapsed_ms) : 0;
    result.feature_dim = features.feature_dim;
    result.input_bytes = features.input_bytes;
    result.mean = features.mean;
    result.variance = features.variance;
    result.min_value = features.min_value;
    result.max_value = features.max_value;
    result.edge_density = features.edge_density;
    result.entropy = features.entropy;
    result.saliency_score = features.saliency_score;
    result.texture_score = features.texture_score;
    result.anomaly_score = features.anomaly_score;
    result.detection_count = features.detection_count;
    result.checksum = features.checksum;
    result.vlm_output_tokens = 0;
    result.vlm_confidence = 0.0F;
    result.vlm_answer_code = 0;
    return result;
}

RuntimeResult run_vlm_query(rust::Slice<const std::uint8_t> buffer) {
    const auto started = std::chrono::steady_clock::now();
    const vlm::QuerySummary query = vlm::run_quantized_query(buffer);
    const auto elapsed = std::chrono::steady_clock::now() - started;
    const auto elapsed_ms =
        std::chrono::duration_cast<std::chrono::milliseconds>(elapsed).count();

    RuntimeResult result;
    result.ok = query.ok;
    result.latency_ms = query.ok ? static_cast<std::uint64_t>(elapsed_ms < 1 ? 1 : elapsed_ms) : 0;
    result.feature_dim = 0;
    result.input_bytes = query.input_bytes;
    result.mean = 0.0F;
    result.variance = 0.0F;
    result.min_value = 0.0F;
    result.max_value = 0.0F;
    result.edge_density = 0.0F;
    result.entropy = 0.0F;
    result.saliency_score = 0.0F;
    result.texture_score = 0.0F;
    result.anomaly_score = 0.0F;
    result.detection_count = 0;
    result.checksum = query.checksum;
    result.vlm_output_tokens = query.output_tokens;
    result.vlm_confidence = query.confidence;
    result.vlm_answer_code = query.answer_code;
    return result;
}

}  // namespace talos::runtime
