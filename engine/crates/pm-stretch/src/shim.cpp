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
#if defined(_WIN32)
#	ifndef NOMINMAX
#		define NOMINMAX
#	endif
#	include <windows.h>
#elif defined(__APPLE__) || defined(__linux__)
#	include <time.h>
#endif

extern "C" {
typedef void pm_stretch_checkpoint_fn();
}

// The library invokes this before each indivisible analysis, spectrum, or
// synthesis step. A null callback leaves ordinary/offline processing alone.
static thread_local pm_stretch_checkpoint_fn *pm_stretch_active_checkpoint = nullptr;
// Setup hooks also occur in STFT helpers used during processing. Enable them
// only inside construction/reset; processing keeps its existing step hooks.
static thread_local pm_stretch_checkpoint_fn *pm_stretch_active_setup_checkpoint = nullptr;

// Explicitly enabled by an ordinary caller before guarded work. The normal
// path takes no clock readings, allocates nothing, and never logs. Each
// record packs the largest thread-CPU span in the high 48 bits and its
// logical step ordinal in the low 16 bits.
static std::atomic<bool> pm_stretch_diag_enabled{false};
static std::atomic<uint64_t> pm_stretch_diag_max[10]{};

static bool pm_stretch_thread_cpu_ns(uint64_t &out) {
#if defined(_WIN32)
	FILETIME created{}, exited{}, kernel{}, user{};
	if (!GetThreadTimes(GetCurrentThread(), &created, &exited, &kernel, &user)) return false;
	uint64_t kernel100ns = (uint64_t(kernel.dwHighDateTime) << 32) | kernel.dwLowDateTime;
	uint64_t user100ns = (uint64_t(user.dwHighDateTime) << 32) | user.dwLowDateTime;
	out = (kernel100ns + user100ns)*100;
	return true;
#elif defined(__APPLE__) || defined(__linux__)
	timespec ts{};
	if (clock_gettime(CLOCK_THREAD_CPUTIME_ID, &ts) != 0) return false;
	out = uint64_t(ts.tv_sec)*1000000000ULL + uint64_t(ts.tv_nsec);
	return true;
#else
	(void)out;
	return false;
#endif
}

struct pm_stretch_diag_scope {
	unsigned category, step;
	uint64_t start = 0;
	bool active = false;
	pm_stretch_diag_scope(unsigned category, unsigned step) : category(category), step(step) {
		if (!pm_stretch_active_checkpoint || !pm_stretch_diag_enabled.load(std::memory_order_relaxed)) return;
		active = pm_stretch_thread_cpu_ns(start);
	}
	~pm_stretch_diag_scope() {
		if (!active) return;
		uint64_t end;
		if (!pm_stretch_thread_cpu_ns(end)) return;
		uint64_t duration = std::min<uint64_t>(end >= start ? end - start : 0, (uint64_t(1) << 48) - 1);
		uint64_t packed = (duration << 16) | std::min<unsigned>(step, 65535);
		auto &maximum = pm_stretch_diag_max[category];
		uint64_t old = maximum.load(std::memory_order_relaxed);
		while (packed > old && !maximum.compare_exchange_weak(old, packed, std::memory_order_relaxed)) {}
	}
};

#define PM_STRETCH_DIAG_JOIN_(a, b) a##b
#define PM_STRETCH_DIAG_JOIN(a, b) PM_STRETCH_DIAG_JOIN_(a, b)
#define SIGNALSMITH_STRETCH_DIAG_SCOPE(category, step) \
	pm_stretch_diag_scope PM_STRETCH_DIAG_JOIN(pm_stretch_diag_scope_, __LINE__)(category, step)
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
	explicit CheckpointScope(pm_stretch_checkpoint_fn *callback, bool setup=false)
		: previous(pm_stretch_active_checkpoint), previousSetup(pm_stretch_active_setup_checkpoint) {
		pm_stretch_active_checkpoint = callback;
		pm_stretch_active_setup_checkpoint = setup ? callback : nullptr;
	}
	~CheckpointScope() {
		pm_stretch_active_checkpoint = previous;
		pm_stretch_active_setup_checkpoint = previousSetup;
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

// Categories 0..9: energy, input copy, new-block setup, analysis step,
// analysis FFT, spectrum step, synthesis step, synthesis FFT, output read,
// and the entire logical process step after its cooperative checkpoint.
void pm_stretch_diag_enable(int enabled) {
	pm_stretch_diag_enabled.store(false, std::memory_order_relaxed);
	for (auto &maximum : pm_stretch_diag_max) maximum.store(0, std::memory_order_relaxed);
	pm_stretch_diag_enabled.store(enabled != 0, std::memory_order_relaxed);
}

int pm_stretch_diag_snapshot(uint64_t *packed, int capacity) {
	if (!packed || capacity < 10) return -1;
	for (int i = 0; i < 10; ++i) packed[i] = pm_stretch_diag_max[i].load(std::memory_order_relaxed);
	return 10;
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
	CheckpointScope scope(checkpoint);
	try {
		static_cast<Handle *>(handle)->stretch->outputSeek(inputs, samples);
		return 0;
	} catch (...) {
		return -1;
	}
}

int pm_stretch_process(void *handle, const float *const *inputs, int input_samples,
		float *const *outputs, int output_samples, pm_stretch_checkpoint_fn *checkpoint) {
	CheckpointScope scope(checkpoint);
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
	CheckpointScope scope(checkpoint);
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
