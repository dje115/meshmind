# MeshMind Protocol

Wire protocol documentation for MeshMind peer-to-peer communication.

## Message Format

All messages use Protobuf-encoded `mesh.Envelope` frames, length-prefixed on the wire.

## Message Types

- **HELLO** — Initial handshake with capabilities and version
- **PING/PONG** — Liveness and RTT measurement
- **ASK** — Question with context bullets and budgets
- **ANSWER** — Response with confidence and evidence refs
- **REFUSE** — Decline with reason code

## Budgets

Every ASK carries hard limits: `ttl_hops`, `deadline_ms`, `max_context_bytes`, `max_answer_bytes`.

## Policy Flags

PolicyFlags travel with each Envelope to control web research, training, citations, and redaction requirements.
