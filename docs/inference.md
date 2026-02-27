# MeshMind Inference

## InferenceBackend Trait

Pluggable LLM runtime behind a trait:
- name() → backend identifier
- health_check() → availability
- generate(request) → response

## Backends

- **Ollama**: HTTP API to local Ollama instance
- **Mock**: Deterministic outputs for tests and CI
- **Future**: llama.cpp embedded, custom runtimes

## Prompt Assembly

Prompt construction and retrieval happen in the application layer, not in the backend.
