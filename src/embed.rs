//! Backend for producing 768-dim embeddings compatible with the palace's
//! Qdrant collection. Exactly one of `ollama` or `fastembed` features must be
//! enabled — enforced by compile_error! in main.rs.

#[cfg(all(feature = "ollama", not(feature = "fastembed")))]
mod ollama;
#[cfg(all(feature = "ollama", not(feature = "fastembed")))]
pub use ollama::Embedder;

#[cfg(feature = "fastembed")]
mod fastembed_backend;
#[cfg(feature = "fastembed")]
pub use fastembed_backend::Embedder;
