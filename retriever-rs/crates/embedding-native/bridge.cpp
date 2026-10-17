// Narrow, synchronous embedding-only ABI. No server, generation, or downloads.
#include "llama.h"
#include "ggml-backend.h"
#include <algorithm>
#include <cstring>
#include <cstdlib>
#include <memory>
#include <mutex>
#include <stdexcept>
#include <string>
#include <vector>

namespace {
constexpr size_t dimensions = 768;
constexpr size_t token_limit = 2048;
constexpr size_t batch_limit = 4096;
struct Runtime {
    llama_model *model = nullptr;
    llama_context *context = nullptr;
    llama_batch_ext *batch = nullptr;
    size_t batch_tokens = batch_limit;
    size_t batch_sequences = 8;
    ~Runtime() {
        if (batch) llama_batch_ext_free(batch);
        if (context) llama_free(context);
        if (model) llama_model_free(model);
    }
};
void error(char *dest, size_t capacity, const char *message) {
    if (!dest || !capacity) return;
    size_t length = std::min(capacity - 1, std::strlen(message));
    std::memcpy(dest, message, length);
    dest[length] = 0;
}
void quiet_log(ggml_log_level, const char *, void *) {}
}

extern "C" void *retriever_embedding_create(const char *path, char *message, size_t capacity) noexcept {
    try {
        if (!path) throw std::runtime_error("Missing model path");
        static std::once_flag backend;
        std::call_once(backend, [] { llama_log_set(quiet_log, nullptr); llama_backend_init(); });
        auto runtime = std::make_unique<Runtime>();
        auto mp = llama_model_default_params();
        const char *device = std::getenv("RETRIEVER_EMBEDDING_DEVICE");
        // The CPU-only build has no GPU backend. Accelerated builds select an
        // available backend, and can be forced to CPU for parity/diagnostics.
        mp.n_gpu_layers = device && std::strcmp(device, "cpu") == 0 ? 0 : 999;
        runtime->model = llama_model_load_from_file(path, mp);
        if (!runtime->model && mp.n_gpu_layers != 0) {
            mp.n_gpu_layers = 0;
            runtime->model = llama_model_load_from_file(path, mp);
        }
        if (!runtime->model) throw std::runtime_error("Cannot load native EmbeddingGemma 2 model");
        if (llama_model_n_embd_out(runtime->model) != dimensions)
            throw std::runtime_error("EmbeddingGemma must produce 768 dimensions");
        auto cp = llama_context_default_params();
        if (mp.n_gpu_layers != 0) {
            for (size_t i = 0; i < ggml_backend_dev_count(); ++i) {
                if (ggml_backend_dev_type(ggml_backend_dev_get(i)) == GGML_BACKEND_DEVICE_TYPE_GPU) {
                    // A bidirectional encoder's combined attention matrix grows
                    // quadratically with batched tokens. On the validated CUDA
                    // path, separate sequences are faster and use less memory;
                    // callers can still group 32 results into one cache commit.
                    runtime->batch_tokens = batch_limit;
                    runtime->batch_sequences = 1;
                    break;
                }
            }
        }
        cp.n_ctx = runtime->batch_tokens;
        cp.n_batch = runtime->batch_tokens;
        cp.n_ubatch = runtime->batch_tokens;
        cp.n_seq_max = runtime->batch_sequences;
        cp.kv_unified = true;
        cp.embeddings = true;
        cp.pooling_type = LLAMA_POOLING_TYPE_MEAN;
        cp.n_threads = 4;
        cp.n_threads_batch = 4;
        runtime->context = llama_init_from_model(runtime->model, cp);
        if (!runtime->context && mp.n_gpu_layers != 0) {
            llama_model_free(runtime->model);
            mp.n_gpu_layers = 0;
            runtime->model = llama_model_load_from_file(path, mp);
            runtime->batch_tokens = batch_limit;
            runtime->batch_sequences = 8;
            cp.n_ctx = cp.n_batch = cp.n_ubatch = batch_limit;
            cp.n_seq_max = 8;
            if (runtime->model) runtime->context = llama_init_from_model(runtime->model, cp);
        }
        if (!runtime->context) throw std::runtime_error("Cannot create native embedding context");
        runtime->batch = llama_batch_ext_init(runtime->context);
        if (!runtime->batch) throw std::runtime_error("Cannot allocate native embedding batch");
        return runtime.release();
    } catch (const std::exception &e) { error(message, capacity, e.what()); }
    catch (...) { error(message, capacity, "Native embedding initialization failed"); }
    return nullptr;
}

extern "C" int retriever_embedding_encode(void *handle, const char *const *texts,
    const size_t *lengths, size_t count, float *output, size_t output_length,
    char *message, size_t capacity) noexcept {
    try {
        if (!handle || !texts || !lengths || !output || !count || count > 32 || output_length != count * dimensions)
            throw std::runtime_error("Invalid native embedding batch");
        auto &runtime = *static_cast<Runtime *>(handle);
        auto vocab = llama_model_get_vocab(runtime.model);
        std::vector<std::vector<llama_token>> tokens;
        for (size_t i = 0; i < count; ++i) {
            if (!texts[i] || !lengths[i] || lengths[i] > 65536)
                throw std::runtime_error("Invalid embedding input length");
            std::vector<llama_token> row(token_limit);
            int n = llama_tokenize(vocab, texts[i], static_cast<int32_t>(lengths[i]), row.data(),
                static_cast<int32_t>(row.size()), true, false);
            if (n <= 0 || n > static_cast<int>(token_limit))
                throw std::runtime_error("Embedding passage exceeds 2048 tokens; recrawl with smaller passages");
            row.resize(n);
            tokens.push_back(std::move(row));
        }
        size_t start = 0;
        while (start < count) {
            size_t end = start, total = 0;
            while (end < count && end - start < runtime.batch_sequences && total + tokens[end].size() <= runtime.batch_tokens) {
                total += tokens[end].size(); ++end;
            }
            llama_batch_ext_clear(runtime.batch);
            if (auto memory = llama_get_memory(runtime.context)) llama_memory_clear(memory, false);
            for (size_t i = start; i < end; ++i) {
                for (size_t p = 0; p < tokens[i].size(); ++p) {
                    auto index = llama_batch_ext_add_token(runtime.batch, static_cast<llama_seq_id>(i-start), tokens[i][p]);
                    llama_pos position[4] = {static_cast<llama_pos>(p), 0, 0, 0};
                    if (index < 0 || !llama_batch_ext_set_pos(runtime.batch, index, position)
                        || !llama_batch_ext_set_output_embd(runtime.batch, index, true))
                        throw std::runtime_error("Cannot populate native embedding batch");
                }
            }
            // EmbeddingGemma 2 is a bidirectional encoder.
            int status = llama_process(runtime.context, LLAMA_PROCESS_TYPE_ENCODE, runtime.batch);
            if (status != 0) throw std::runtime_error("Native embedding inference failed: " + std::to_string(status));
            for (size_t i = start; i < end; ++i) {
                auto vector = llama_get_embeddings_seq(runtime.context, static_cast<llama_seq_id>(i-start));
                if (!vector) throw std::runtime_error("Missing pooled embedding");
                std::copy_n(vector, dimensions, output + i * dimensions);
            }
            start = end;
        }
        return 0;
    } catch (const std::exception &e) { error(message, capacity, e.what()); }
    catch (...) { error(message, capacity, "Native embedding inference failed"); }
    return -1;
}

extern "C" void retriever_embedding_free(void *handle) noexcept { delete static_cast<Runtime *>(handle); }
