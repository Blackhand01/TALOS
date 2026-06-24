#pragma once

#include <cstdint>

#include "rust/cxx.h"

namespace talos::runtime::cv {

inline constexpr std::uint32_t kFeatureDim = 11;

struct FeatureSummary {
    bool ok;
    std::uint64_t input_bytes;
    std::uint32_t feature_dim;
    float mean;
    float variance;
    float min_value;
    float max_value;
    float edge_density;
    float entropy;
    float saliency_score;
    float texture_score;
    float anomaly_score;
    std::uint32_t detection_count;
    std::uint64_t checksum;
};

FeatureSummary extract_features(rust::Slice<const std::uint8_t> buffer);

}  // namespace talos::runtime::cv
