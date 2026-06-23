#include "runtime/backend.hpp"

#include <chrono>

#include "runtime/cv/features.hpp"
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
    result.checksum = features.checksum;
    return result;
}

}  // namespace talos::runtime
