# MeshMind Web Research

## Gates

Requires all three:
1. Policy flag: allow_web = true
2. Node capability: research_web = true
3. Redaction: redaction_required = true

## Flow

1. Validate policy and capability
2. Apply redaction rules to query
3. Search web with domain restrictions
4. Summarize with citations
5. Store as WebBrief artifact with TTL
6. Emit WEB_BRIEF_CREATED and ARTIFACT_PUBLISHED events
