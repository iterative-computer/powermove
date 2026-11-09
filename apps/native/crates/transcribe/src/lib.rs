//! Speech to text on this Mac, as main's transcription (`main/transcription`):
//! the transcribe.cpp runtime (ggml, Metal on Apple Silicon) loaded natively,
//! the pinned Hugging Face GGUF models and main's transcript shape.
//! Everything here blocks: call it on a worker, never the main thread.
pub mod pipeline;
pub mod runtime;
pub mod store;
