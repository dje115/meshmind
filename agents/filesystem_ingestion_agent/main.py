#!/usr/bin/env python3
"""
MeshMind filesystem ingestion agent — entry point.
Local-only: watches folders, extracts content, sends to MeshMind core.

Modes:
  - Default: print config (use --one-shot or --watch for actual ingest)
  - --one-shot PATH [--source-id ID]: scan, extract, POST to core once
  - --watch: run filesystem watcher (TODO: full implementation)
"""

import argparse
import os
import sys
import time
from pathlib import Path

# Add agent root to path
sys.path.insert(0, str(Path(__file__).resolve().parent))


def one_shot_ingest(path: Path, source_id: str, api_url: str, admin_token: str) -> int:
    """Scan folder, extract, publish to core. Returns 0 on success."""
    from watcher import WatchedSource, scan_source
    from extraction import extract_and_normalize
    from publisher import publish_batch

    path = path.resolve()
    if not path.exists() or not path.is_dir():
        print(f"Error: not a directory: {path}", file=sys.stderr)
        return 1

    source = WatchedSource(
        source_id=source_id,
        root=path,
        recursion=True,
        include_patterns=["*"],
        exclude_patterns=[],
    )

    items: list = []
    def on_queued(p: Path, _prior) -> None:
        item = extract_and_normalize(p, source_id, str(path))
        items.append(item)

    from watcher import WatchStateStore

    state_path = path / ".meshmind_agent_state.json"
    scan_source(source, WatchStateStore(state_path), on_queued)

    if not items:
        print(f"No document files found in {path}")
        return 0

    sent, docs, err = publish_batch(items, api_url, admin_token)
    if err:
        print(f"Publish failed: {err}", file=sys.stderr)
        return 1
    print(f"Published {sent} items, {docs} docs created")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="MeshMind filesystem ingestion agent")
    parser.add_argument("--one-shot", metavar="PATH", help="Scan folder once and ingest")
    parser.add_argument("--source-id", default="agent-fs", help="MeshMind source_id")
    parser.add_argument("--api-url", default=os.environ.get("MESHMIND_API_URL", "http://127.0.0.1:9900"))
    parser.add_argument("--admin-token", default=os.environ.get("MESHMIND_ADMIN_TOKEN", ""))
    parser.add_argument("--watch", action="store_true", help="Run watcher (TODO)")
    args = parser.parse_args()

    if args.one_shot:
        return one_shot_ingest(
            Path(args.one_shot),
            args.source_id,
            args.api_url,
            args.admin_token,
        )

    if args.watch:
        print("Watch mode not yet fully implemented; use --one-shot for now")
        return 0

    # Default: print config
    watch_dirs = os.environ.get("WATCH_DIRS", "")
    print("MeshMind filesystem ingestion agent (local-only)")
    print(f"  API URL: {args.api_url}")
    print(f"  Watch dirs: {watch_dirs or '(none configured)'}")
    print("  Usage: --one-shot /path/to/folder [--source-id src-1]")
    return 0


if __name__ == "__main__":
    sys.exit(main())
