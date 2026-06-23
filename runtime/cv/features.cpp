#include "runtime/cv/features.hpp"

#include <array>
#include <cmath>
#include <cstddef>
#include <limits>

namespace talos::runtime::cv {
namespace {

constexpr std::uint64_t kFnvOffsetBasis = 14695981039346656037ULL;
constexpr std::uint64_t kFnvPrime = 1099511628211ULL;
constexpr float kByteScale = 1.0F / 255.0F;
constexpr int kEdgeThreshold = 32;

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

    for (std::size_t index = 0; index < buffer.size(); ++index) {
        const std::uint8_t value = buffer[index];
        histogram[value] += 1;
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

    for (const std::uint64_t bucket_count : histogram) {
        if (bucket_count == 0) {
            continue;
        }
        const double probability = static_cast<double>(bucket_count) / count;
        entropy -= probability * std::log2(probability);
    }

    summary.mean = static_cast<float>(mean_byte * kByteScale);
    summary.variance = static_cast<float>(variance_byte * kByteScale * kByteScale);
    summary.min_value = static_cast<float>(min_byte) * kByteScale;
    summary.max_value = static_cast<float>(max_byte) * kByteScale;
    summary.edge_density = buffer.size() > 1
                               ? static_cast<float>(
                                     static_cast<double>(edge_count) /
                                     static_cast<double>(buffer.size() - 1))
                               : 0.0F;
    summary.entropy = static_cast<float>(entropy);
    summary.checksum = checksum;
    return summary;
}

}  // namespace talos::runtime::cv
