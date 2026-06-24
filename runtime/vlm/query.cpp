#include "runtime/vlm/query.hpp"

#include <cstddef>

namespace talos::runtime::vlm {
namespace {

constexpr std::uint64_t kFnvOffsetBasis = 14695981039346656037ULL;
constexpr std::uint64_t kFnvPrime = 1099511628211ULL;
constexpr std::uint32_t kMaxTokens = 128;

std::uint64_t update_checksum(std::uint64_t checksum, std::uint8_t value) {
    checksum ^= static_cast<std::uint64_t>(value);
    checksum *= kFnvPrime;
    return checksum;
}

}  // namespace

QuerySummary run_quantized_query(rust::Slice<const std::uint8_t> buffer) {
    QuerySummary summary{};
    summary.ok = !buffer.empty();
    summary.input_bytes = static_cast<std::uint64_t>(buffer.size());

    std::uint64_t checksum = kFnvOffsetBasis;
    std::uint64_t sum = 0;
    for (std::size_t index = 0; index < buffer.size(); ++index) {
        const std::uint8_t value = buffer[index];
        checksum = update_checksum(checksum, value);
        sum += value;
    }

    summary.checksum = checksum;
    if (buffer.empty()) {
        return summary;
    }

    const std::uint32_t byte_token_estimate =
        static_cast<std::uint32_t>((buffer.size() + 65535U) / 65536U);
    summary.output_tokens = byte_token_estimate < 8U ? 8U : byte_token_estimate;
    if (summary.output_tokens > kMaxTokens) {
        summary.output_tokens = kMaxTokens;
    }

    const float mean = static_cast<float>(sum) / static_cast<float>(buffer.size() * 255U);
    summary.confidence = 0.45F + (mean * 0.45F);
    if (summary.confidence > 0.92F) {
        summary.confidence = 0.92F;
    }
    summary.answer_code = static_cast<std::uint32_t>(checksum % 1009ULL);
    return summary;
}

}  // namespace talos::runtime::vlm
