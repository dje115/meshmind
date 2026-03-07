# MeshMind Web Research

## Gates

Requires all three:

1. **Policy flag**: `allow_web = true`
2. **Node capability**: `research_web_capable = true`
3. **Redaction**: `redaction_required = true`

---

## Flow

1. Validate policy and capability
2. Apply redaction rules to query
3. Search web with domain restrictions
4. Summarize with citations
5. Store as WebBrief artifact with TTL
6. Emit `WEB_BRIEF_CREATED` and `ARTIFACT_PUBLISHED` events

---

## Output

- WebBrief with summary, sources, citations
- Stored in `web_briefs_view`
- Expires per TTL
- Used as evidence in future ask flows

---

## References

- `crates/node_research` — Implementation
- [docs/architecture/security.md](../architecture/security.md) — Redaction
- [docs/workflows/ask-flow.md](ask-flow.md) — When web research is triggered
