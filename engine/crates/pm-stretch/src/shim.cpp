// A C interface to Signalsmith Stretch (vendor/, MIT) for pm-stretch.
//
// Every entry point catches C++ exceptions (allocation failures inside the
// library's std::vector members) and reports them as a status, so nothing
// unwinds across the FFI boundary. Channel buffers are passed as arrays of
// planar channel pointers, which the library's templated `inputs[c][i]`
// indexing reads directly.

#include <cstdint>
#include <new>

#include "signalsmith-stretch/signalsmith-stretch.h"

namespace {

using Stretch = signalsmith::stretch::SignalsmithStretch<float>;

struct Handle {
	Stretch stretch;
	long seed;
	int channels;
	explicit Handle(long s) : stretch(s), seed(s), channels(0) {}
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

// Status codes: 0 ok, -1 an exception (allocation) inside the library.

void *pm_stretch_new(int channels, int block, int interval, long seed) {
	try {
		auto *h = new Handle(seed);
		h->channels = channels;
		h->stretch.configure(channels, block, interval, false);
		return h;
	} catch (...) {
		return nullptr;
	}
}

void pm_stretch_free(void *handle) {
	delete static_cast<Handle *>(handle);
}

int pm_stretch_input_latency(const void *handle) {
	return static_cast<const Handle *>(handle)->stretch.inputLatency();
}

int pm_stretch_output_latency(const void *handle) {
	return static_cast<const Handle *>(handle)->stretch.outputLatency();
}

int pm_stretch_block_samples(const void *handle) {
	return static_cast<const Handle *>(handle)->stretch.blockSamples();
}

int pm_stretch_interval_samples(const void *handle) {
	return static_cast<const Handle *>(handle)->stretch.intervalSamples();
}

int pm_stretch_seek_length(const void *handle) {
	return static_cast<const Handle *>(handle)->stretch.seekLength();
}

int pm_stretch_output_seek_length(const void *handle, float rate) {
	return static_cast<const Handle *>(handle)->stretch.outputSeekLength(rate);
}

// Back to the state of a new instance with the same configuration: the
// processing state is cleared and the random engine (used for stretches
// slower than 0.5x) is reseeded, so a run is a function of its input alone.
int pm_stretch_reset(void *handle) {
	auto *h = static_cast<Handle *>(handle);
	try {
		int block = h->stretch.blockSamples();
		int interval = h->stretch.intervalSamples();
		h->stretch.~Stretch();
		new (&h->stretch) Stretch(h->seed);
		h->stretch.configure(h->channels, block, interval, false);
		return 0;
	} catch (...) {
		return -1;
	}
}

int pm_stretch_set_transpose(void *handle, float factor) {
	try {
		static_cast<Handle *>(handle)->stretch.setTransposeFactor(factor);
		return 0;
	} catch (...) {
		return -1;
	}
}

int pm_stretch_seek(void *handle, const float *const *inputs, int samples, double rate) {
	try {
		static_cast<Handle *>(handle)->stretch.seek(inputs, samples, rate);
		return 0;
	} catch (...) {
		return -1;
	}
}

int pm_stretch_output_seek(void *handle, const float *const *inputs, int samples) {
	try {
		static_cast<Handle *>(handle)->stretch.outputSeek(inputs, samples);
		return 0;
	} catch (...) {
		return -1;
	}
}

int pm_stretch_process(void *handle, const float *const *inputs, int input_samples,
		float *const *outputs, int output_samples) {
	try {
		auto &s = static_cast<Handle *>(handle)->stretch;
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

int pm_stretch_flush(void *handle, float *const *outputs, int output_samples, float rate) {
	try {
		static_cast<Handle *>(handle)->stretch.flush(outputs, output_samples, rate);
		return 0;
	} catch (...) {
		return -1;
	}
}

} // extern "C"
