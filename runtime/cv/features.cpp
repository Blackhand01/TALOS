#include "runtime/cv/features.hpp"

#include <array>
#include <algorithm>
#include <cmath>
#include <cstddef>
#include <limits>

namespace talos::runtime::cv {
namespace {

constexpr std::uint64_t kFnvOffsetBasis = 14695981039346656037ULL;
constexpr std::uint64_t kFnvPrime = 1099511628211ULL;
constexpr float kByteScale = 1.0F / 255.0F;
constexpr int kEdgeThreshold = 32;
constexpr std::size_t kGridCells = 64;
constexpr float kSaliencyThreshold = 0.35F;

std::uint64_t update_checksum(std::uint64_t checksum, std::uint8_t value) {
    checksum ^= static_cast<std::uint64_t>(value);
    checksum *= kFnvPrime;
    return checksum;
}

}  // namespace

FeatureSummary extract_features(rust::Slice<const std::uint8_t> buffer) {
    FeatureSummary summary{};
    summary.ok = !buffer.empty();
    summary.input_bytes = static_cast<std::uint64_t>(buffer.size());
    summary.feature_dim = kFeatureDim;

    if (buffer.empty()) {
        summary.checksum = kFnvOffsetBasis;
        return summary;
    }

    std::array<std::uint64_t, 256> histogram{};
    std::uint64_t checksum = kFnvOffsetBasis;
    std::uint64_t sum = 0;
    double sum_squares = 0.0;
    std::uint8_t min_byte = std::numeric_limits<std::uint8_t>::max();
    std::uint8_t max_byte = std::numeric_limits<std::uint8_t>::min();
    std::uint64_t edge_count = 0;
    std::array<double, kGridCells> grid_sums{};
    std::array<std::uint64_t, kGridCells> grid_counts{};

    for (std::size_t index = 0; index < buffer.size(); ++index) {
        const std::uint8_t value = buffer[index];
        const std::size_t grid_index = (index * kGridCells) / buffer.size();
        histogram[value] += 1;
        grid_sums[grid_index] += static_cast<double>(value);
        grid_counts[grid_index] += 1;
        checksum = update_checksum(checksum, value);
        sum += value;
        sum_squares += static_cast<double>(value) * static_cast<double>(value);
        min_byte = value < min_byte ? value : min_byte;
        max_byte = value > max_byte ? value : max_byte;

        if (index > 0) {
            const int previous = static_cast<int>(buffer[index - 1]);
            const int current = static_cast<int>(value);
            if (std::abs(current - previous) > kEdgeThreshold) {
                edge_count += 1;
            }
        }
    }

    const double count = static_cast<double>(buffer.size());
    const double mean_byte = static_cast<double>(sum) / count;
    const double variance_byte = (sum_squares / count) - (mean_byte * mean_byte);
    double entropy = 0.0;
    double texture_accumulator = 0.0;
    double saliency_accumulator = 0.0;
    std::uint32_t detection_count = 0;
    double previous_cell_mean = -1.0;

    for (const std::uint64_t bucket_count : histogram) {
        if (bucket_count == 0) {
            continue;
        }
        const double probability = static_cast<double>(bucket_count) / count;
        entropy -= probability * std::log2(probability);
    }

    for (std::size_t cell = 0; cell < kGridCells; ++cell) {
        if (grid_counts[cell] == 0) {
            continue;
        }

        const double cell_mean = grid_sums[cell] / static_cast<double>(grid_counts[cell]);
        const double normalized_delta = std::abs(cell_mean - mean_byte) / 255.0;
        saliency_accumulator = std::max(saliency_accumulator, normalized_delta);
        if (normalized_delta >= kSaliencyThreshold) {
            detection_count += 1;
        }

        if (previous_cell_mean >= 0.0) {
            texture_accumulator += std::abs(cell_mean - previous_cell_mean) / 255.0;
        }
        previous_cell_mean = cell_mean;
    }

    const double texture_score = texture_accumulator / static_cast<double>(kGridCells - 1);
    const double edge_ratio = buffer.size() > 1
                                  ? static_cast<double>(edge_count) /
                                        static_cast<double>(buffer.size() - 1)
                                  : 0.0;
    const double entropy_ratio = entropy / 8.0;
    const double contrast = (static_cast<double>(max_byte) - static_cast<double>(min_byte)) / 255.0;

    summary.mean = static_cast<float>(mean_byte * kByteScale);
    summary.variance = static_cast<float>(variance_byte * kByteScale * kByteScale);
    summary.min_value = static_cast<float>(min_byte) * kByteScale;
    summary.max_value = static_cast<float>(max_byte) * kByteScale;
    summary.edge_density = static_cast<float>(edge_ratio);
    summary.entropy = static_cast<float>(entropy);
    summary.saliency_score = static_cast<float>(saliency_accumulator);
    summary.texture_score = static_cast<float>(texture_score);
    summary.anomaly_score = static_cast<float>(
        std::min(1.0, (saliency_accumulator * 0.45) + (edge_ratio * 0.25) +
                      (texture_score * 0.20) + (contrast * 0.10) +
                      (std::max(0.0, entropy_ratio - 0.5) * 0.10)));
    summary.detection_count = detection_count;
    summary.checksum = checksum;
    return summary;
}

}  // namespace talos::runtime::cv
