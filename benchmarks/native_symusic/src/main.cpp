// Native, source-pinned Symusic tick-score benchmark. The full score-contract
// digest is checked before and after timing; parsing, score lifetime, and score
// destruction are the only work in the measured operation.

#include <algorithm>
#include <array>
#include <atomic>
#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <optional>
#include <span>
#include <sstream>
#include <stdexcept>
#include <string>
#include <string_view>
#include <vector>

#if defined(__linux__)
#include <sys/utsname.h>
#endif

#include "symusic.h"

namespace {

constexpr std::string_view kSchema = "miso-native-symusic-benchmark/v1";
constexpr std::string_view kContractSchema = "miso-score-contract/v1";
constexpr std::string_view kMagic{"MISO-SCORE-CONTRACT\0\1", 21};
constexpr std::size_t kDefaultSamples = 30;
constexpr std::size_t kDefaultWarmup = 5;
constexpr std::uint64_t kDefaultMinSampleNs = 50'000'000;

#if defined(__VERSION__)
constexpr std::string_view kCompilerVersion = __VERSION__;
#elif defined(_MSC_FULL_VER)
constexpr std::string_view kCompilerVersion = "MSVC";
#else
constexpr std::string_view kCompilerVersion = "unknown";
#endif

#if defined(_WIN32)
constexpr std::string_view kTargetOs = "windows";
#elif defined(__APPLE__)
constexpr std::string_view kTargetOs = "macos";
#elif defined(__linux__)
constexpr std::string_view kTargetOs = "linux";
#else
constexpr std::string_view kTargetOs = "unknown";
#endif

#if defined(__x86_64__) || defined(_M_X64)
constexpr std::string_view kTargetArch = "x86_64";
#elif defined(__aarch64__) || defined(_M_ARM64)
constexpr std::string_view kTargetArch = "aarch64";
#else
constexpr std::string_view kTargetArch = "unknown";
#endif

#if defined(NDEBUG)
constexpr bool kDebugAssertions = false;
#else
constexpr bool kDebugAssertions = true;
#endif

struct Counts {
    std::size_t tracks{};
    std::size_t notes{};
    std::size_t controls{};
    std::size_t pitch_bends{};
    std::size_t pedals{};
    std::size_t lyrics{};
    std::size_t time_signatures{};
    std::size_t key_signatures{};
    std::size_t tempos{};
    std::size_t markers{};

    [[nodiscard]] bool operator==(const Counts&) const = default;
};

struct ExpectedCorpus {
    std::string_view name;
    std::string_view input_sha256;
    std::string_view semantic_sha256;
    Counts counts;
};

constexpr std::array kExpected = {
    ExpectedCorpus{"tiny", "39da22e3a55fdf78b68855e8ed870ccfbf3e5d077401fba7174773f7fa7c92d7",
                   "bd36b66d133db7772eb2bc5e81e7a1c9ea4a62561de0131a9465ba73c9491acc",
                   {1, 16, 3, 1, 1, 0, 0, 0, 0, 1}},
    ExpectedCorpus{"normal", "4b62f8bbd60175f610097817e1759514297f694a46320e1f3d770dbb88c94f97",
                   "d75cb3bb06a230b8bbbb371e32cf86f5aeaa2a4c1ea098f7f5f371eb559271f1",
                   {8, 16'000, 272, 64, 8, 0, 0, 0, 0, 32}},
    ExpectedCorpus{"huge", "90d7ad33e14e80149d8cd2c3d0dae204de9b2ec4670b850593864111245bd40f",
                   "fe10b416f2f7a65925f38e2a66f201b427040c3243d2b7c818bde3297b12d37c",
                   {16, 192'000, 3'040, 752, 16, 0, 0, 0, 0, 384}},
    ExpectedCorpus{"mahler", "35a59329ab8f1f86ec2602bb5293b9fbddc694e512aafa00e310cb8da237f302",
                   "d8fcfebd208541d7791fc0dab49b561893a7c50180ccbcc61b7049e009013f69",
                   {51, 60'411, 36'287, 0, 0, 0, 97, 97, 177, 97}},
};

class Sha256 {
public:
    void update(std::span<const std::uint8_t> input) {
        for (const auto byte : input) {
            buffer_[buffer_len_++] = byte;
            if (buffer_len_ == buffer_.size()) {
                transform(buffer_);
                total_bits_ += 512;
                buffer_len_ = 0;
            }
        }
    }

    [[nodiscard]] std::array<std::uint8_t, 32> finish() {
        total_bits_ += static_cast<std::uint64_t>(buffer_len_) * 8;
        buffer_[buffer_len_++] = 0x80;
        if (buffer_len_ > 56) {
            while (buffer_len_ < 64) buffer_[buffer_len_++] = 0;
            transform(buffer_);
            buffer_len_ = 0;
        }
        while (buffer_len_ < 56) buffer_[buffer_len_++] = 0;
        for (int index = 7; index >= 0; --index) {
            buffer_[buffer_len_++] = static_cast<std::uint8_t>(total_bits_ >> (index * 8));
        }
        transform(buffer_);
        std::array<std::uint8_t, 32> output{};
        for (std::size_t index = 0; index < state_.size(); ++index) {
            for (std::size_t byte = 0; byte < 4; ++byte) {
                output[index * 4 + byte] = static_cast<std::uint8_t>(state_[index] >> (24 - byte * 8));
            }
        }
        return output;
    }

private:
    static constexpr std::array<std::uint32_t, 64> k = {
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    };
    std::array<std::uint32_t, 8> state_ = {
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    };
    std::array<std::uint8_t, 64> buffer_{};
    std::size_t buffer_len_{};
    std::uint64_t total_bits_{};

    static constexpr std::uint32_t rotr(const std::uint32_t value, const std::uint32_t amount) {
        return (value >> amount) | (value << (32 - amount));
    }

    void transform(const std::array<std::uint8_t, 64>& block) {
        std::array<std::uint32_t, 64> words{};
        for (std::size_t index = 0; index < 16; ++index) {
            words[index] = (static_cast<std::uint32_t>(block[index * 4]) << 24)
                         | (static_cast<std::uint32_t>(block[index * 4 + 1]) << 16)
                         | (static_cast<std::uint32_t>(block[index * 4 + 2]) << 8)
                         | static_cast<std::uint32_t>(block[index * 4 + 3]);
        }
        for (std::size_t index = 16; index < words.size(); ++index) {
            const auto s0 = rotr(words[index - 15], 7) ^ rotr(words[index - 15], 18) ^ (words[index - 15] >> 3);
            const auto s1 = rotr(words[index - 2], 17) ^ rotr(words[index - 2], 19) ^ (words[index - 2] >> 10);
            words[index] = words[index - 16] + s0 + words[index - 7] + s1;
        }
        auto [a, b, c, d, e, f, g, h] = state_;
        for (std::size_t index = 0; index < words.size(); ++index) {
            const auto s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
            const auto choose = (e & f) ^ ((~e) & g);
            const auto first = h + s1 + choose + k[index] + words[index];
            const auto s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
            const auto majority = (a & b) ^ (a & c) ^ (b & c);
            const auto second = s0 + majority;
            h = g; g = f; f = e; e = d + first; d = c; c = b; b = a; a = first + second;
        }
        state_[0] += a; state_[1] += b; state_[2] += c; state_[3] += d;
        state_[4] += e; state_[5] += f; state_[6] += g; state_[7] += h;
    }
};

[[nodiscard]] std::string hex_digest(std::span<const std::uint8_t> bytes) {
    Sha256 hash;
    hash.update(bytes);
    const auto digest = hash.finish();
    std::ostringstream output;
    output << std::hex << std::setfill('0');
    for (const auto byte : digest) output << std::setw(2) << static_cast<unsigned>(byte);
    return output.str();
}

class ContractEncoder {
public:
    ContractEncoder() { bytes_.insert(bytes_.end(), kMagic.begin(), kMagic.end()); }

    void integer(const std::int64_t value) {
        const auto raw = static_cast<std::uint64_t>(value);
        for (int shift = 56; shift >= 0; shift -= 8) bytes_.push_back(static_cast<std::uint8_t>(raw >> shift));
    }
    void count(const std::size_t value) { integer_unsigned(static_cast<std::uint64_t>(value)); }
    void boolean(const bool value) { bytes_.push_back(value ? 1 : 0); }
    void text(const std::string& value) {
        count(value.size());
        bytes_.insert(bytes_.end(), value.begin(), value.end());
    }
    [[nodiscard]] std::string digest() const { return hex_digest(bytes_); }

private:
    void integer_unsigned(const std::uint64_t value) {
        for (int shift = 56; shift >= 0; shift -= 8) bytes_.push_back(static_cast<std::uint8_t>(value >> shift));
    }
    std::vector<std::uint8_t> bytes_;
};

template <typename T>
void encode_integer(ContractEncoder& encoder, const T value) {
    encoder.integer(static_cast<std::int64_t>(value));
}

struct ContractResult { std::string sha256; Counts counts; };

[[nodiscard]] ContractResult score_contract(const symusic::Score<symusic::Tick>& score) {
    ContractEncoder encoder;
    Counts counts{};
    encode_integer(encoder, score.ticks_per_quarter);
    encoder.count(score.tracks->size());
    counts.tracks = score.tracks->size();
    for (const auto& track : *score.tracks) {
        encoder.text(track->name);
        encode_integer(encoder, track->program);
        encoder.boolean(track->is_drum);
        encoder.count(track->notes->size());
        counts.notes += track->notes->size();
        for (const auto& event : *track->notes) {
            encode_integer(encoder, event.time); encode_integer(encoder, event.duration);
            encode_integer(encoder, event.pitch); encode_integer(encoder, event.velocity);
        }
        encoder.count(track->controls->size());
        counts.controls += track->controls->size();
        for (const auto& event : *track->controls) {
            encode_integer(encoder, event.time); encode_integer(encoder, event.number); encode_integer(encoder, event.value);
        }
        encoder.count(track->pitch_bends->size());
        counts.pitch_bends += track->pitch_bends->size();
        for (const auto& event : *track->pitch_bends) {
            encode_integer(encoder, event.time); encode_integer(encoder, event.value);
        }
        encoder.count(track->pedals->size());
        counts.pedals += track->pedals->size();
        for (const auto& event : *track->pedals) {
            encode_integer(encoder, event.time); encode_integer(encoder, event.duration);
        }
        encoder.count(track->lyrics->size());
        counts.lyrics += track->lyrics->size();
        for (const auto& event : *track->lyrics) {
            encode_integer(encoder, event.time); encoder.text(event.text);
        }
    }
    encoder.count(score.time_signatures->size());
    counts.time_signatures = score.time_signatures->size();
    for (const auto& event : *score.time_signatures) {
        encode_integer(encoder, event.time); encode_integer(encoder, event.numerator); encode_integer(encoder, event.denominator);
    }
    encoder.count(score.key_signatures->size());
    counts.key_signatures = score.key_signatures->size();
    for (const auto& event : *score.key_signatures) {
        encode_integer(encoder, event.time); encode_integer(encoder, event.key); encode_integer(encoder, event.tonality);
    }
    encoder.count(score.tempos->size());
    counts.tempos = score.tempos->size();
    for (const auto& event : *score.tempos) {
        encode_integer(encoder, event.time); encode_integer(encoder, event.mspq);
    }
    encoder.count(score.markers->size());
    counts.markers = score.markers->size();
    for (const auto& event : *score.markers) {
        encode_integer(encoder, event.time); encoder.text(event.text);
    }
    return {encoder.digest(), counts};
}

[[nodiscard]] const ExpectedCorpus& expected_for(const std::string& name) {
    for (const auto& item : kExpected) if (item.name == name) return item;
    throw std::runtime_error("unknown dataset '" + name + "'; expected tiny,normal,huge,mahler");
}

[[nodiscard]] std::vector<std::uint8_t> read_file(const std::filesystem::path& path) {
    std::ifstream input(path, std::ios::binary);
    if (!input) throw std::runtime_error("cannot read " + path.string());
    return {std::istreambuf_iterator<char>(input), std::istreambuf_iterator<char>()};
}

void verify(const std::string& name, std::span<const std::uint8_t> data, const ExpectedCorpus& expected) {
    const auto input_hash = hex_digest(data);
    if (input_hash != expected.input_sha256) throw std::runtime_error(name + ": corpus SHA-256 differs from fixed expectation");
    const auto score = symusic::Score<symusic::Tick>::parse<symusic::DataFormat::MIDI>(data);
    const auto contract = score_contract(score);
    if (contract.sha256 != expected.semantic_sha256 || contract.counts != expected.counts) {
        throw std::runtime_error(name + ": Symusic native score differs from the fixed full score contract");
    }
}

template <typename T>
inline void do_not_optimize(const T& value) {
#if defined(__clang__) || defined(__GNUC__)
    asm volatile("" : : "g"(&value) : "memory");
#else
    std::atomic_signal_fence(std::memory_order_seq_cst);
    (void)value;
#endif
}

struct Distribution { std::size_t iterations{}; std::vector<double> values; };

template <typename Operation>
[[nodiscard]] Distribution measure(Operation&& operation, const std::size_t samples, const std::size_t warmup,
                                   std::optional<std::size_t> requested_iterations, const std::uint64_t min_sample_ns) {
    for (std::size_t index = 0; index < warmup; ++index) operation();
    std::size_t iterations = requested_iterations.value_or(1);
    if (!requested_iterations) {
        while (true) {
            const auto begin = std::chrono::steady_clock::now();
            for (std::size_t index = 0; index < iterations; ++index) operation();
            const auto elapsed = std::chrono::steady_clock::now() - begin;
            if (std::chrono::duration_cast<std::chrono::nanoseconds>(elapsed).count() >= static_cast<long long>(min_sample_ns)) break;
            if (iterations > (1U << 30)) throw std::runtime_error("iteration calibration overflow");
            iterations *= 2;
        }
    }
    Distribution result{iterations, {}};
    result.values.reserve(samples);
    for (std::size_t sample = 0; sample < samples; ++sample) {
        const auto begin = std::chrono::steady_clock::now();
        for (std::size_t index = 0; index < iterations; ++index) operation();
        const auto elapsed = std::chrono::steady_clock::now() - begin;
        result.values.push_back(static_cast<double>(std::chrono::duration_cast<std::chrono::nanoseconds>(elapsed).count()) / static_cast<double>(iterations));
    }
    return result;
}

[[nodiscard]] double median(const Distribution& distribution) {
    auto values = distribution.values;
    std::sort(values.begin(), values.end());
    const auto middle = values.size() / 2;
    return values.size() % 2 == 0 ? (values[middle - 1] + values[middle]) / 2.0 : values[middle];
}

[[nodiscard]] std::string json_escape(const std::string& value) {
    std::ostringstream output;
    for (const unsigned char byte : value) {
        switch (byte) {
            case '\\': output << "\\\\"; break; case '"': output << "\\\""; break;
            case '\n': output << "\\n"; break; case '\r': output << "\\r"; break; case '\t': output << "\\t"; break;
            default: if (byte < 0x20) output << "\\u" << std::hex << std::setw(4) << std::setfill('0') << static_cast<int>(byte) << std::dec; else output << static_cast<char>(byte);
        }
    }
    return output.str();
}

[[nodiscard]] std::string cpuinfo_value(const std::string& key) {
    std::ifstream input("/proc/cpuinfo");
    std::string line;
    while (std::getline(input, line)) {
        const auto colon = line.find(':');
        if (colon == std::string::npos) continue;
        const auto trim = [](std::string text) {
            const auto first = text.find_first_not_of(" \t");
            const auto last = text.find_last_not_of(" \t");
            return first == std::string::npos ? std::string{} : text.substr(first, last - first + 1);
        };
        if (trim(line.substr(0, colon)) == key) return trim(line.substr(colon + 1));
    }
    return "unknown";
}

[[nodiscard]] std::string affinity() {
    std::ifstream input("/proc/self/status");
    std::string line;
    while (std::getline(input, line)) {
        if (line.rfind("Cpus_allowed_list:", 0) == 0) {
            const auto value = line.substr(line.find(':') + 1);
            const auto first = value.find_first_not_of(" \t");
            return first == std::string::npos ? "unknown" : value.substr(first);
        }
    }
    return "unknown";
}

[[nodiscard]] std::optional<unsigned int> first_cpu_from_affinity(const std::string_view value) {
    std::size_t begin{};
    while (begin < value.size() && (value[begin] == ' ' || value[begin] == '\t')) ++begin;
    std::size_t end = begin;
    while (end < value.size() && value[end] >= '0' && value[end] <= '9') ++end;
    if (begin == end) return std::nullopt;
    try { return static_cast<unsigned int>(std::stoul(std::string{value.substr(begin, end - begin)})); }
    catch (const std::exception&) { return std::nullopt; }
}

[[nodiscard]] std::string cpu_governor(const std::string_view cpu_affinity) {
#if defined(__linux__)
    const auto cpu = first_cpu_from_affinity(cpu_affinity);
    if (!cpu) return "unknown";
    std::ifstream input("/sys/devices/system/cpu/cpu" + std::to_string(*cpu) + "/cpufreq/scaling_governor");
    std::string value;
    if (!std::getline(input, value)) return "unknown";
    const auto first = value.find_first_not_of(" \t");
    const auto last = value.find_last_not_of(" \t");
    return first == std::string::npos ? "unknown" : value.substr(first, last - first + 1);
#else
    static_cast<void>(cpu_affinity);
    return "unknown";
#endif
}

[[nodiscard]] std::string kernel_release() {
#if defined(__linux__)
    utsname details{};
    return uname(&details) == 0 ? details.release : "unknown";
#else
    return "unknown";
#endif
}

struct Options {
    std::filesystem::path corpus_dir{"benchmarks/corpus"};
    std::vector<std::string> datasets{"tiny", "normal", "huge", "mahler"};
    std::size_t samples{kDefaultSamples};
    std::size_t warmup{kDefaultWarmup};
    std::optional<std::size_t> iterations{};
    std::uint64_t min_sample_ns{kDefaultMinSampleNs};
    std::optional<std::filesystem::path> output{};
    bool verify_only{};
    bool self_test{};
};

[[nodiscard]] std::vector<std::string> split_csv(const std::string& value) {
    std::vector<std::string> output; std::size_t begin{};
    while (begin <= value.size()) { const auto end = value.find(',', begin); output.push_back(value.substr(begin, end - begin)); if (end == std::string::npos) break; begin = end + 1; }
    return output;
}

[[nodiscard]] Options parse_options(const int argc, char** argv) {
    Options options;
    auto next = [&](int& index, const char* flag) -> std::string { if (++index == argc) throw std::runtime_error(std::string(flag) + " needs a value"); return argv[index]; };
    for (int index = 1; index < argc; ++index) {
        const std::string flag = argv[index];
        if (flag == "--corpus-dir") options.corpus_dir = next(index, "--corpus-dir");
        else if (flag == "--datasets") options.datasets = split_csv(next(index, "--datasets"));
        else if (flag == "--samples") options.samples = std::stoull(next(index, "--samples"));
        else if (flag == "--warmup") options.warmup = std::stoull(next(index, "--warmup"));
        else if (flag == "--iterations") { const auto value = std::stoull(next(index, "--iterations")); options.iterations = value == 0 ? std::nullopt : std::optional{value}; }
        else if (flag == "--min-sample-ns") options.min_sample_ns = std::stoull(next(index, "--min-sample-ns"));
        else if (flag == "--output") options.output = next(index, "--output");
        else if (flag == "--verify-only") options.verify_only = true;
        else if (flag == "--self-test") options.self_test = true;
        else if (flag == "--help") { std::cout << "--corpus-dir DIR --datasets CSV --samples N --warmup N --iterations N(0=calibrate) --min-sample-ns N --output FILE --verify-only --self-test\n"; std::exit(0); }
        else throw std::runtime_error("unknown option " + flag);
    }
    if (options.samples == 0 || options.datasets.empty()) throw std::runtime_error("samples and datasets must be nonempty");
    return options;
}

[[nodiscard]] std::string counts_json(const Counts& value) {
    std::ostringstream output;
    output << "{\"tracks\":" << value.tracks << ",\"notes\":" << value.notes << ",\"controls\":" << value.controls
           << ",\"pitch_bends\":" << value.pitch_bends << ",\"pedals\":" << value.pedals << ",\"lyrics\":" << value.lyrics
           << ",\"time_signatures\":" << value.time_signatures << ",\"key_signatures\":" << value.key_signatures
           << ",\"tempos\":" << value.tempos << ",\"markers\":" << value.markers << "}";
    return output.str();
}

[[nodiscard]] std::string distribution_json(const Distribution& value) {
    const auto [minimum, maximum] = std::minmax_element(value.values.begin(), value.values.end());
    double mean{}; for (const auto item : value.values) mean += item; mean /= static_cast<double>(value.values.size());
    std::ostringstream output; output << std::fixed << std::setprecision(6);
    output << "{\"iterations\":" << value.iterations << ",\"samples_ns_per_operation\":[";
    for (std::size_t index = 0; index < value.values.size(); ++index) { if (index) output << ','; output << value.values[index]; }
    output << "],\"median_ns\":" << median(value) << ",\"mean_ns\":" << mean << ",\"min_ns\":" << *minimum << ",\"max_ns\":" << *maximum << "}";
    return output.str();
}

[[nodiscard]] std::string configuration_json(const Options& options) {
    std::ostringstream output;
    output << "{\"datasets\":[";
    for (std::size_t index = 0; index < options.datasets.size(); ++index) {
        if (index) output << ',';
        output << "\"" << json_escape(options.datasets[index]) << "\"";
    }
    output << "],\"samples\":" << options.samples << ",\"warmup\":" << options.warmup
           << ",\"iterations\":";
    if (options.iterations) output << *options.iterations; else output << "\"auto\"";
    output << ",\"min_sample_ns\":" << options.min_sample_ns
           << ",\"parse_only\":true,\"timed_operation\":\"parse_score_and_destroy\"}";
    return output.str();
}

[[nodiscard]] bool self_test() {
    const std::array<std::uint8_t, 3> abc{'a', 'b', 'c'};
    Options options;
    options.datasets = {"tiny", "normal"};
    options.samples = 7;
    options.warmup = 3;
    options.iterations = 11;
    options.min_sample_ns = 99;
    const auto config = configuration_json(options);
    return hex_digest(abc) == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        && cpuinfo_value("model name") != ""
        && first_cpu_from_affinity("4-7,12") == std::optional<unsigned int>{4}
        && !first_cpu_from_affinity("unknown")
        && config == "{\"datasets\":[\"tiny\",\"normal\"],\"samples\":7,\"warmup\":3,\"iterations\":11,\"min_sample_ns\":99,\"parse_only\":true,\"timed_operation\":\"parse_score_and_destroy\"}";
}

int run(const Options& options) {
    if (options.self_test) {
        if (!self_test()) throw std::runtime_error("native Symusic harness self-test failed");
        std::cout << "native Symusic harness self-test: ok\n";
        return 0;
    }
    std::vector<std::string> datasets_json;
    for (const auto& name : options.datasets) {
        const auto& expected = expected_for(name);
        const auto data = read_file(options.corpus_dir / (name + ".mid"));
        verify(name, data, expected);
        if (options.verify_only) continue;
        auto operation = [&data] {
            auto score = symusic::Score<symusic::Tick>::parse<symusic::DataFormat::MIDI>(data);
            do_not_optimize(score);
        };
        const auto distribution = measure(operation, options.samples, options.warmup, options.iterations, options.min_sample_ns);
        verify(name, data, expected);
        std::ostringstream item;
        item << "{\"dataset\":\"" << name << "\",\"input_bytes\":" << data.size()
             << ",\"input_sha256\":\"" << expected.input_sha256 << "\",\"semantic_contract\":{\"schema\":\""
             << kContractSchema << "\",\"sha256\":\"" << expected.semantic_sha256 << "\",\"summary\":" << counts_json(expected.counts)
             << "},\"parse_score_midi\":" << distribution_json(distribution) << "}";
        datasets_json.push_back(item.str());
    }
    if (options.verify_only) { std::cout << "native Symusic fixed-contract verification: ok\n"; return 0; }
    std::ostringstream report;
    report << "{\"schema\":\"" << kSchema << "\",\"method\":\"warm in-memory Score<Tick>::parse<MIDI>; score destruction included; full fixed contract checked outside timing\","
           << "\"source\":{\"repository\":\"https://github.com/Yikai-Liao/symusic\",\"commit\":\"" << MISO_SYMUSIC_PIN << "\"},"
           << "\"machine\":{\"target_arch\":\"" << kTargetArch << "\",\"target_os\":\"" << kTargetOs
           << "\",\"compiler\":\"" << json_escape(std::string{kCompilerVersion}) << "\",\"cmake_compiler_id\":\"" << MISO_SYMUSIC_CMAKE_COMPILER_ID
           << "\",\"cmake_compiler_version\":\"" << MISO_SYMUSIC_CMAKE_COMPILER_VERSION << "\",\"cmake_version\":\"" << MISO_SYMUSIC_CMAKE_VERSION
           << "\",\"cmake_generator\":\"" << MISO_SYMUSIC_CMAKE_GENERATOR << "\",\"cxx_flags\":\"" << json_escape(MISO_SYMUSIC_CMAKE_CXX_FLAGS) << "\",\"cxx_standard\":" << __cplusplus
           << ",\"build_type\":\"" << MISO_SYMUSIC_BUILD_TYPE << "\",\"debug_assertions\":" << (kDebugAssertions ? "true" : "false")
           << ",\"ipo_enabled\":" << (MISO_SYMUSIC_IPO_ENABLED ? "true" : "false")
           << ",\"symusic_library_ipo_enabled\":" << (MISO_SYMUSIC_LIBRARY_IPO_ENABLED ? "true" : "false")
           << ",\"lto_requested\":" << (MISO_SYMUSIC_LTO_REQUESTED ? "true" : "false")
           << ",\"cpu_affinity\":\"" << json_escape(affinity()) << "\",\"cpu_model\":\"" << json_escape(cpuinfo_value("model name"))
           << "\",\"cpu_governor\":\"" << json_escape(cpu_governor(affinity())) << "\",\"kernel_release\":\"" << json_escape(kernel_release())
           << "\"},\"configuration\":" << configuration_json(options) << ",\"datasets\":[";
    for (std::size_t index = 0; index < datasets_json.size(); ++index) { if (index) report << ','; report << datasets_json[index]; }
    report << "]}";
    if (options.output) {
        if (!options.output->parent_path().empty()) std::filesystem::create_directories(options.output->parent_path());
        std::ofstream(*options.output) << report.str() << '\n';
    }
    else std::cout << report.str() << '\n';
    return 0;
}

} // namespace

int main(const int argc, char** argv) {
    try { return run(parse_options(argc, argv)); }
    catch (const std::exception& error) { std::cerr << "error: " << error.what() << '\n'; return 2; }
}
