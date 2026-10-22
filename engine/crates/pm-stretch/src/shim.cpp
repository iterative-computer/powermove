// A C interface to Signalsmith Stretch (vendor/, MIT) for pm-stretch.
//
// Every entry point catches C++ exceptions (allocation failures inside the
// library's std::vector members) and reports them as a status, so nothing
// unwinds across the FFI boundary. Channel buffers are passed as arrays of
// planar channel pointers, which the library's templated `inputs[c][i]`
// indexing reads directly.

#include <cstdint>
#include <algorithm>
#include <atomic>
#include <memory>
#include <new>
#if defined(__APPLE__) || defined(__linux__)
#	include <time.h>
#endif

extern "C" {
typedef void pm_stretch_checkpoint_fn();
}

// The library invokes this before each indivisible analysis, spectrum, or
// synthesis step. A null callback leaves ordinary/offline processing alone.
static thread_local pm_stretch_checkpoint_fn *pm_stretch_active_checkpoint = nullptr;
// 0 direct process, 1 output seek, 2 flush, 3 outside a diagnosed entry.
static thread_local unsigned pm_stretch_diag_entry = 3;
// Setup hooks also occur in STFT helpers used during processing. Enable them
// only inside construction/reset; processing keeps its existing step hooks.
static thread_local pm_stretch_checkpoint_fn *pm_stretch_active_setup_checkpoint = nullptr;

// Explicitly enabled by an ordinary caller before guarded work. The normal
// path takes no clock readings, allocates nothing, and never logs. Each
// record packs the largest thread-CPU span in the high 48 bits and an
// operator-local or block-relative index in the low 16 bits. FFT
// categories 4/7 use channel index; category 9 uses the block step index.
static std::atomic<bool> pm_stretch_diag_enabled{false};
static std::atomic<uint64_t> pm_stretch_diag_max[3][10]{};

static bool pm_stretch_thread_cpu_ns(uint64_t &out) {
#if defined(__APPLE__) || defined(__linux__)
	timespec ts{};
	if (clock_gettime(CLOCK_THREAD_CPUTIME_ID, &ts) != 0) return false;
	out = uint64_t(ts.tv_sec)*1000000000ULL + uint64_t(ts.tv_nsec);
	return true;
#else
	// GetThreadTimes has scheduler-quantum resolution on Windows, so its
	// apparent per-operator maxima would be misleading. Keep the binding
	// buildable there and report zero diagnostic observations.
	(void)out;
	return false;
#endif
}

struct pm_stretch_diag_scope {
	unsigned category, step, entry = 3;
	uint64_t start = 0;
	bool active = false;
	pm_stretch_diag_scope(unsigned category, unsigned step, bool start_now=true)
		: category(category), step(step) {
		if (start_now) begin();
	}
	void begin() {
		if (active || !pm_stretch_diag_enabled.load(std::memory_order_relaxed)) return;
		if (!pm_stretch_active_checkpoint || pm_stretch_diag_entry >= 3) return;
		entry = pm_stretch_diag_entry;
		active = pm_stretch_thread_cpu_ns(start);
	}
	void finish() {
		if (!active) return;
		active = false;
		uint64_t end;
		if (!pm_stretch_thread_cpu_ns(end)) return;
		uint64_t duration = std::min<uint64_t>(end >= start ? end - start : 0, (uint64_t(1) << 48) - 1);
		uint64_t packed = (duration << 16) | std::min<unsigned>(step, 65535);
		auto &maximum = pm_stretch_diag_max[entry][category];
		uint64_t old = maximum.load(std::memory_order_relaxed);
		while (packed > old && !maximum.compare_exchange_weak(old, packed, std::memory_order_relaxed)) {}
	}
	~pm_stretch_diag_scope() { finish(); }
};

#define PM_STRETCH_DIAG_JOIN_(a, b) a##b
#define PM_STRETCH_DIAG_JOIN(a, b) PM_STRETCH_DIAG_JOIN_(a, b)
#define SIGNALSMITH_STRETCH_DIAG_SCOPE(category, step) \
	pm_stretch_diag_scope PM_STRETCH_DIAG_JOIN(pm_stretch_diag_scope_, __LINE__)(category, step)
#define SIGNALSMITH_STRETCH_DIAG_OUTPUT_RUN_DECLARE() pm_stretch_diag_scope pm_stretch_diag_output_run(8, 0, false)
#define SIGNALSMITH_STRETCH_DIAG_OUTPUT_RUN_BEGIN() pm_stretch_diag_output_run.begin()
#define SIGNALSMITH_STRETCH_DIAG_OUTPUT_RUN_END() pm_stretch_diag_output_run.finish()
#define SIGNALSMITH_STRETCH_PROFILE_PROCESS_STEP(step, steps) \
	do { \
		if (pm_stretch_active_checkpoint) pm_stretch_active_checkpoint(); \
	} while (false)
#define SIGNALSMITH_STRETCH_SETUP_CHECKPOINT() \
	do { if (pm_stretch_active_setup_checkpoint) pm_stretch_active_setup_checkpoint(); } while (false)
#define SIGNALSMITH_STRETCH_HAS_SETUP_CHECKPOINT() (pm_stretch_active_setup_checkpoint != nullptr)
#include "signalsmith-stretch/signalsmith-stretch.h"
#undef SIGNALSMITH_STRETCH_PROFILE_PROCESS_STEP
#undef SIGNALSMITH_STRETCH_SETUP_CHECKPOINT
#undef SIGNALSMITH_STRETCH_HAS_SETUP_CHECKPOINT
#undef SIGNALSMITH_STRETCH_DIAG_SCOPE
#undef SIGNALSMITH_STRETCH_DIAG_OUTPUT_RUN_DECLARE
#undef SIGNALSMITH_STRETCH_DIAG_OUTPUT_RUN_BEGIN
#undef SIGNALSMITH_STRETCH_DIAG_OUTPUT_RUN_END
#undef PM_STRETCH_DIAG_JOIN
#undef PM_STRETCH_DIAG_JOIN_

namespace {

using Stretch = signalsmith::stretch::SignalsmithStretch<float>;

struct Handle {
	std::unique_ptr<Stretch> stretch;
	long seed;
	int channels;
	explicit Handle(long s) : stretch(std::make_unique<Stretch>(s)), seed(s), channels(0) {}
};

struct CheckpointScope {
	pm_stretch_checkpoint_fn *previous;
	pm_stretch_checkpoint_fn *previousSetup;
	unsigned previousEntry;
	explicit CheckpointScope(pm_stretch_checkpoint_fn *callback, bool setup=false, unsigned entry=3)
		: previous(pm_stretch_active_checkpoint), previousSetup(pm_stretch_active_setup_checkpoint),
		  previousEntry(pm_stretch_diag_entry) {
		pm_stretch_active_checkpoint = callback;
		pm_stretch_active_setup_checkpoint = setup ? callback : nullptr;
		pm_stretch_diag_entry = entry;
	}
	~CheckpointScope() {
		pm_stretch_active_checkpoint = previous;
		pm_stretch_active_setup_checkpoint = previousSetup;
		pm_stretch_diag_entry = previousEntry;
	}
};

// Zero-valued input of any length, for flushes and silent pre-rolls.
struct Zeros {
	struct Channel {
		float operator[](int) const { return 0.0f; }
	};
	Channel operator[](int) const { return {}; }
};

} // namespace

extern "C" {

// Rows 0..2: direct process, output seek, flush. Categories 0..9: energy,
// input copy, new-block setup, analysis step, analysis FFT, spectrum step,
// synthesis step, synthesis FFT, contiguous inter-block output/read run,
// entire logical process step after its cooperative checkpoint.
void pm_stretch_diag_enable(int enabled) {
	pm_stretch_diag_enabled.store(false, std::memory_order_relaxed);
	for (auto &row : pm_stretch_diag_max) {
		for (auto &maximum : row) maximum.store(0, std::memory_order_relaxed);
	}
	pm_stretch_diag_enabled.store(enabled != 0, std::memory_order_relaxed);
}

int pm_stretch_diag_snapshot(uint64_t *packed, int capacity) {
	if (!packed || capacity < 30) return -1;
	for (int entry = 0; entry < 3; ++entry) {
		for (int category = 0; category < 10; ++category) {
			packed[entry*10 + category] = pm_stretch_diag_max[entry][category].load(std::memory_order_relaxed);
		}
	}
	return 30;
}

// Status codes: 0 ok, -1 an exception (allocation) inside the library.

void *pm_stretch_new_with_checkpoint(int channels, int block, int interval, long seed,
		pm_stretch_checkpoint_fn *checkpoint) {
	CheckpointScope scope(checkpoint, true);
	try {
		auto h = std::make_unique<Handle>(seed);
		h->channels = channels;
		// The immediately following Kaiser setup replaces the initial ACG
		// window/reset. Only this callback-enabled path defers that work.
		h->stretch->configure(channels, block, interval, false, checkpoint != nullptr);
		return h.release();
	} catch (...) {
		return nullptr;
	}
}

void *pm_stretch_new(int channels, int block, int interval, long seed) {
	return pm_stretch_new_with_checkpoint(channels, block, interval, seed, nullptr);
}

void pm_stretch_free(void *handle) {
	delete static_cast<Handle *>(handle);
}

int pm_stretch_input_latency(const void *handle) {
	return static_cast<const Handle *>(handle)->stretch->inputLatency();
}

int pm_stretch_output_latency(const void *handle) {
	return static_cast<const Handle *>(handle)->stretch->outputLatency();
}

int pm_stretch_block_samples(const void *handle) {
	return static_cast<const Handle *>(handle)->stretch->blockSamples();
}

int pm_stretch_interval_samples(const void *handle) {
	return static_cast<const Handle *>(handle)->stretch->intervalSamples();
}

int pm_stretch_seek_length(const void *handle) {
	return static_cast<const Handle *>(handle)->stretch->seekLength();
}

int pm_stretch_output_seek_length(const void *handle, float rate) {
	return static_cast<const Handle *>(handle)->stretch->outputSeekLength(rate);
}

// Back to the state of a new instance with the same configuration: the
// processing state is cleared and the random engine (used for stretches
// slower than 0.5x) is reseeded, so a run is a function of its input alone.
int pm_stretch_reset_with_checkpoint(void *handle, pm_stretch_checkpoint_fn *checkpoint) {
	auto *h = static_cast<Handle *>(handle);
	CheckpointScope scope(checkpoint, true);
	try {
		int block = h->stretch->blockSamples();
		int interval = h->stretch->intervalSamples();
		// Configure the replacement before touching the live object. An
		// exception leaves the previous stretch fully constructed and usable.
		auto replacement = std::make_unique<Stretch>(h->seed);
		replacement->configure(h->channels, block, interval, false, checkpoint != nullptr);
		h->stretch.swap(replacement);
		return 0;
	} catch (...) {
		return -1;
	}
}

int pm_stretch_reset(void *handle) {
	return pm_stretch_reset_with_checkpoint(handle, nullptr);
}

int pm_stretch_set_transpose(void *handle, float factor) {
	try {
		static_cast<Handle *>(handle)->stretch->setTransposeFactor(factor);
		return 0;
	} catch (...) {
		return -1;
	}
}

int pm_stretch_seek(void *handle, const float *const *inputs, int samples, double rate) {
	try {
		static_cast<Handle *>(handle)->stretch->seek(inputs, samples, rate);
		return 0;
	} catch (...) {
		return -1;
	}
}

int pm_stretch_output_seek(void *handle, const float *const *inputs, int samples,
		pm_stretch_checkpoint_fn *checkpoint) {
	CheckpointScope scope(checkpoint, false, 1);
	try {
		static_cast<Handle *>(handle)->stretch->outputSeek(inputs, samples);
		return 0;
	} catch (...) {
		return -1;
	}
}

int pm_stretch_process(void *handle, const float *const *inputs, int input_samples,
		float *const *outputs, int output_samples, pm_stretch_checkpoint_fn *checkpoint) {
	CheckpointScope scope(checkpoint, false, 0);
	try {
		auto &s = *static_cast<Handle *>(handle)->stretch;
		if (input_samples == 0) {
			s.process(Zeros{}, 0, outputs, output_samples);
		} else {
			s.process(inputs, input_samples, outputs, output_samples);
		}
		return 0;
	} catch (...) {
		return -1;
	}
}

int pm_stretch_flush(void *handle, float *const *outputs, int output_samples, float rate,
		pm_stretch_checkpoint_fn *checkpoint) {
	CheckpointScope scope(checkpoint, false, 2);
	try {
		static_cast<Handle *>(handle)->stretch->flush(outputs, output_samples, rate);
		return 0;
	} catch (...) {
		return -1;
	}
}

// The library's one-shot stretch of a whole buffer (its reference use):
// returns 1 when it stretched, 0 when the input was too short.
int pm_stretch_exact(void *handle, const float *const *inputs, int input_samples,
		float *const *outputs, int output_samples) {
	try {
		return static_cast<Handle *>(handle)->stretch->exact(inputs, input_samples, outputs, output_samples) ? 1 : 0;
	} catch (...) {
		return -1;
	}
}

} // extern "C"
