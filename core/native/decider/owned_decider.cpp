#include <llama.h>
#include <ggml-backend.h>
#include <algorithm>
#include <atomic>
#include <cerrno>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <exception>
#include <memory>
#include <map>
#include <sstream>
#include <mutex>
#include <string>
#include <sys/stat.h>
#include <unistd.h>
#include <vector>

// The coordinator opens and hashes the file. This shim consumes a duplicate of
// that same open file description, never a model pathname or configured alias.
struct owned_decider {
    llama_model * model = nullptr;
    llama_context * context = nullptr;
    int pinned_fd = -1;
    FILE * loader_file = nullptr;
    struct stat source{};
    uint32_t maximum_tokens = 0;
    std::atomic<bool> abort{false};
    std::mutex gate;
    ~owned_decider() {
        if (context) llama_free(context);
        if (model) llama_model_free(model);
        // Pinned upstream llama_file(FILE*) has owns_fp=false. The caller closes
        // its stream after the model/mappings have been released.
        if (loader_file) fclose(loader_file);
        if (pinned_fd >= 0) close(pinned_fd);
    }
};

static void error_text(char * output, size_t size, const char * value) {
    if (output && size) { std::snprintf(output, size, "%s", value); }
}
static bool unchanged(const struct stat & left, const struct stat & right) {
    return left.st_dev == right.st_dev && left.st_ino == right.st_ino &&
        left.st_size == right.st_size && left.st_mtimespec.tv_sec == right.st_mtimespec.tv_sec &&
        left.st_mtimespec.tv_nsec == right.st_mtimespec.tv_nsec &&
        left.st_ctimespec.tv_sec == right.st_ctimespec.tv_sec && left.st_ctimespec.tv_nsec == right.st_ctimespec.tv_nsec;
}
static bool source_valid(owned_decider * state) {
    struct stat current{};
    return state && state->pinned_fd >= 0 && fstat(state->pinned_fd, &current) == 0 && unchanged(state->source, current);
}
static bool abort_decode(void * pointer) {
    return static_cast<owned_decider *>(pointer)->abort.load(std::memory_order_relaxed);
}
static void discard_log(enum ggml_log_level, const char *, void *) {}
static std::string json_string(const char * value) {
    std::string result = "\"";
    for (const unsigned char * p = reinterpret_cast<const unsigned char *>(value ? value : ""); *p; ++p) {
        if (*p == '\\' || *p == '\"') { result += '\\'; result += char(*p); }
        else if (*p >= 0x20) result += char(*p);
    }
    return result + "\"";
}
static int copy_metadata(const std::string & text, char * output, size_t size) {
    if (!output || text.size() + 1 > size) return -1;
    std::memcpy(output, text.c_str(), text.size() + 1); return int(text.size());
}
// Pinned source exposes its model tensor map for metadata inspection. No tensor
// contents, source text, configured filename or inferred offload claim is read.
const std::vector<std::pair<std::string, ggml_tensor *>> & llama_internal_get_tensor_map(const llama_model * model);


extern "C" {
const char * od_abi() { return "pennel-managed-decider-cabi-v1"; }
#ifndef PENNEL_DECIDER_BUILD_SIGNATURE
#error "The pinned-source runtime build signature is required"
#endif
const char * od_build_signature() { return PENNEL_DECIDER_BUILD_SIGNATURE; }
const char * od_build_system() { return llama_print_system_info(); }
int od_device_metadata(char * output, size_t size) {
    llama_log_set(discard_log, nullptr); llama_backend_init();
    std::ostringstream json; json << "{\"scope\":\"registered runtime devices; no model loaded\",\"devices\":[";
    for (size_t index = 0; index < ggml_backend_dev_count(); ++index) {
        auto * device = ggml_backend_dev_get(index);
        size_t free_bytes = 0, total_bytes = 0; ggml_backend_dev_memory(device, &free_bytes, &total_bytes);
        if (index) json << ',';
        json << "{\"name\":" << json_string(ggml_backend_dev_name(device)) << ",\"description\":" << json_string(ggml_backend_dev_description(device))
             << ",\"type\":" << int(ggml_backend_dev_type(device)) << ",\"free_bytes\":" << free_bytes << ",\"total_bytes\":" << total_bytes << '}';
    }
    json << "]}"; return copy_metadata(json.str(), output, size);
}
int od_model_buffer_metadata(owned_decider * state, char * output, size_t size) {
    if (!source_valid(state) || !state->model) return -1;
    struct entry { uint64_t tensors = 0, bytes = 0; int type = -1; };
    std::map<std::string, entry> entries;
    for (const auto & item : llama_internal_get_tensor_map(state->model)) {
        const ggml_tensor * tensor = item.second;
        const ggml_tensor * owner = tensor;
        for (int depth = 0; owner && !owner->buffer && owner->view_src && depth < 8; ++depth) owner = owner->view_src;
        if (!owner || !owner->buffer) continue;
        auto * buft = ggml_backend_buffer_get_type(owner->buffer);
        auto * device = ggml_backend_buft_get_device(buft);
        auto & value = entries[ggml_backend_buft_name(buft)];
        value.tensors++; value.bytes += ggml_nbytes(tensor);
        value.type = device ? int(ggml_backend_dev_type(device)) : -1;
    }
    std::ostringstream json; json << "{\"scope\":\"actual loaded weight tensor buffer allocation; not per-operation scheduling\",\"buffers\":[";
    bool first = true; for (const auto & item : entries) {
        if (!first) json << ','; first = false;
        json << "{\"buffer_type\":" << json_string(item.first.c_str()) << ",\"device_type\":" << item.second.type
             << ",\"tensor_count\":" << item.second.tensors << ",\"tensor_bytes\":" << item.second.bytes << '}';
    }
    json << "]}"; return copy_metadata(json.str(), output, size);
}

owned_decider * od_load(int fd, uint32_t maximum_tokens, int gpu_layers, int threads,
                       const char * backend_directory, char * error, size_t error_size) {
    if (fd < 0 || maximum_tokens < 128 || maximum_tokens > 32768 || threads < 1 || threads > 128) {
        error_text(error, error_size, "invalid_load_bounds"); return nullptr;
    }
    auto state = std::make_unique<owned_decider>();
    if (fstat(fd, &state->source) != 0 || !S_ISREG(state->source.st_mode)) {
        error_text(error, error_size, "model_file_handle_invalid"); return nullptr;
    }
    state->pinned_fd = dup(fd);
    int loader_fd = dup(fd);
    if (state->pinned_fd < 0 || loader_fd < 0) {
        if (loader_fd >= 0) close(loader_fd);
        error_text(error, error_size, "model_file_handle_duplicate_failed"); return nullptr;
    }
    state->loader_file = fdopen(loader_fd, "rb");
    if (!state->loader_file || fseeko(state->loader_file, 0, SEEK_SET) != 0) {
        if (!state->loader_file) close(loader_fd);
        error_text(error, error_size, "model_file_stream_failed"); return nullptr;
    }
    llama_log_set(discard_log, nullptr);
    llama_backend_init();
    // This product runtime links only its pinned built-in backends. A caller
    // cannot discover or load arbitrary machine-wide backend plugins.
    if (backend_directory && backend_directory[0]) {
        error_text(error, error_size, "external_backends_forbidden"); return nullptr;
    }
    auto model_params = llama_model_default_params();
    model_params.n_gpu_layers = gpu_layers;
    model_params.load_mode = LLAMA_LOAD_MODE_MMAP;
    model_params.check_tensors = true;
    state->model = llama_model_load_from_file_ptr(state->loader_file, model_params);
    if (!state->model || !source_valid(state.get())) {
        error_text(error, error_size, "owned_model_load_failed_or_changed"); return nullptr;
    }
    auto params = llama_context_default_params();
    params.n_ctx = maximum_tokens;
    params.n_batch = maximum_tokens;
    params.n_ubatch = maximum_tokens;
    params.n_seq_max = 8;
    params.kv_unified = false;
    params.n_outputs_max = 64;
    params.n_outputs_max_per_seq = 8;
    params.n_threads = threads;
    params.n_threads_batch = threads;
    params.attention_type = LLAMA_ATTENTION_TYPE_NON_CAUSAL;
    params.flash_attn_type = LLAMA_FLASH_ATTN_TYPE_ENABLED;
    params.no_perf = true;
    params.abort_callback = abort_decode;
    params.abort_callback_data = state.get();
    state->context = llama_init_from_model(state->model, params);
    if (!state->context || !source_valid(state.get())) {
        error_text(error, error_size, "owned_context_init_failed_or_changed"); return nullptr;
    }
    llama_set_causal_attn(state->context, false);
    state->maximum_tokens = maximum_tokens;
    error_text(error, error_size, "");
    return state.release();
}

void od_abort(owned_decider * state) { if (state) state->abort.store(true, std::memory_order_relaxed); }
void od_destroy(owned_decider * state) {
    if (!state) return;
    delete state;
}
int od_source_stat(owned_decider * state, uint64_t * values, size_t count) {
    if (!source_valid(state) || !values || count < 7) return -1;
    const auto & s = state->source;
    values[0] = s.st_dev; values[1] = s.st_ino; values[2] = s.st_size;
    values[3] = s.st_mtimespec.tv_sec; values[4] = s.st_mtimespec.tv_nsec;
    values[5] = s.st_ctimespec.tv_sec; values[6] = s.st_ctimespec.tv_nsec;
    return 0;
}
int od_metadata(owned_decider * state, const char * key, char * result, size_t size) {
    if (!source_valid(state) || !key) return -1;
    return llama_model_meta_val_str(state->model, key, result, size);
}
int od_tokenize(owned_decider * state, const char * text, int32_t bytes, int32_t * output, int32_t capacity,
                bool add_special, bool parse_special) {
    if (!source_valid(state) || !text || bytes < 0 || capacity < 0) return 0;
    return llama_tokenize(llama_model_get_vocab(state->model), text, bytes, output, capacity, add_special, parse_special);
}

// logits_out is row-major [marker_count, code_count]. Only marker rows are
// requested from the decoder; there is no sampler or generated text.
int od_logits_sequences(owned_decider * state, const int32_t * tokens, int32_t token_count,
              const int32_t * sequence_ids, const int32_t * positions,
              const int32_t * marker_positions, int32_t marker_count,
              const int32_t * code_tokens, int32_t code_count, float * logits_out,
              char * error, size_t error_size) {
    if (!state || !tokens || !sequence_ids || !positions || !marker_positions || !code_tokens || !logits_out ||
        token_count < 1 || uint32_t(token_count) > state->maximum_tokens || marker_count < 1 || marker_count > 64 ||
        code_count < 2 || code_count > 255) { error_text(error, error_size, "invalid_decision_bounds"); return -1; }
    std::lock_guard<std::mutex> guard(state->gate);
    if (!source_valid(state)) { error_text(error, error_size, "model_file_changed"); return -2; }
    const int32_t vocabulary = llama_vocab_n_tokens(llama_model_get_vocab(state->model));
    int32_t last_sequence = -1;
    int32_t last_position = -1;
    int32_t marker_counts[8]{};
    for (int32_t index = 0; index < token_count; ++index) {
        const int32_t seq = sequence_ids[index];
        if (seq < 0 || seq >= 8 || seq < last_sequence || seq > last_sequence + 1 ||
            positions[index] >= int32_t(state->maximum_tokens / 8) ||
            positions[index] != (seq == last_sequence ? last_position + 1 : 0)) {
            error_text(error, error_size, "invalid_independent_sequence_layout"); return -1;
        }
        last_sequence = seq; last_position = positions[index];
    }
    for (int32_t index = 0; index < marker_count; ++index) {
        if (marker_positions[index] < 0 || marker_positions[index] >= token_count ||
            (index && marker_positions[index] <= marker_positions[index-1])) {
            error_text(error, error_size, "invalid_marker_positions"); return -1;
        }
        if (++marker_counts[sequence_ids[marker_positions[index]]] > 8) {
            error_text(error, error_size, "sequence_marker_count_limit"); return -1;
        }
    }
    for (int32_t index = 0; index < code_count; ++index) {
        if (code_tokens[index] < 0 || code_tokens[index] >= vocabulary) { error_text(error, error_size, "invalid_code_token"); return -1; }
    }
    if (state->abort.load(std::memory_order_relaxed)) {
        error_text(error, error_size, "decision_aborted"); return 2;
    }
    llama_memory_clear(llama_get_memory(state->context), true);
    llama_set_causal_attn(state->context, false);
    auto batch = llama_batch_init(token_count, 0, 1);
    batch.n_tokens = token_count;
    for (int32_t index = 0; index < token_count; ++index) {
        batch.token[index] = tokens[index]; batch.pos[index] = positions[index];
        batch.n_seq_id[index] = 1; batch.seq_id[index][0] = sequence_ids[index];
        batch.logits[index] = std::binary_search(marker_positions, marker_positions + marker_count, index);
    }
    const int status = llama_decode(state->context, batch);
    if (status != 0 || !source_valid(state)) {
        llama_batch_free(batch); llama_memory_clear(llama_get_memory(state->context), true);
        error_text(error, error_size, status == 2 ? "decision_aborted" : "decision_decode_failed_or_changed"); return status ? status : -2;
    }
    for (int32_t row = 0; row < marker_count; ++row) {
        const float * logits = llama_get_logits_ith(state->context, marker_positions[row]);
        if (!logits) { llama_batch_free(batch); error_text(error, error_size, "marker_logits_unavailable"); return -3; }
        for (int32_t column = 0; column < code_count; ++column) {
            logits_out[row * code_count + column] = logits[code_tokens[column]];
        }
    }
    llama_batch_free(batch);
    error_text(error, error_size, ""); return 0;
}

// Compatibility entry for a single independent prompt, never a cross-window map.
int od_logits(owned_decider * state, const int32_t * tokens, int32_t token_count,
              const int32_t * marker_positions, int32_t marker_count,
              const int32_t * code_tokens, int32_t code_count, float * logits_out,
              char * error, size_t error_size) {
    if (token_count < 1 || token_count > 32768) { error_text(error, error_size, "invalid_decision_bounds"); return -1; }
    std::vector<int32_t> sequences(token_count, 0), positions(token_count);
    for (int32_t index = 0; index < token_count; ++index) positions[index] = index;
    return od_logits_sequences(state, tokens, token_count, sequences.data(), positions.data(),
        marker_positions, marker_count, code_tokens, code_count, logits_out, error, error_size);
}
}
