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


def one_shot_ingest(
    path: Path,
    source_id: str,
    api_url: str,
    admin_token: str,
    llm_helper_enabled: bool = False,
) -> int:
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
        item = extract_and_normalize(p, source_id, str(path), llm_helper_enabled=False)
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


def run_watch_mode(api_url: str, admin_token: str) -> int:
    """Run watcher with sources from main app. Polls config periodically."""
    from watcher import WatchedSource, run_watcher
    from extraction import extract_and_normalize
    from publisher import publish_batch

    sources, err = fetch_agent_config(api_url, admin_token)
    if err:
        print(f"Config fetch failed: {err}", file=sys.stderr)
        return 1
    if not sources:
        print("No agent sources configured; exiting. Add [[agent_sources]] to meshmind.toml")
        return 0

    watched: list[WatchedSource] = []
    for s in sources:
        path_str = s.get("path", "")
        src_id = s.get("source_id", "agent-fs")
        if not path_str:
            continue
        p = Path(path_str)
        if not p.exists() or not p.is_dir():
            print(f"Skipping {src_id}: not a directory: {path_str}", file=sys.stderr)
            continue
        inc = s.get("include_patterns") or ["*"]
        exc = s.get("exclude_patterns") or []
        rec = s.get("recursion", True)
        ocr = s.get("ocr_enabled", True)
        llm = s.get("llm_helper_enabled", False)
        watched.append(WatchedSource(
            source_id=src_id,
            root=p.resolve(),
            recursion=rec,
            include_patterns=inc,
            exclude_patterns=exc,
            ocr_enabled=ocr,
            llm_helper_enabled=llm,
        ))

    if not watched:
        print("No valid sources to watch")
        return 0

    state_path = Path(__file__).resolve().parent / ".meshmind_watch_state.json"

    def on_queued(path: Path, _prior) -> None:
        src_id = "agent-fs"
        llm_enabled = False
        for w in watched:
            try:
                path.resolve().relative_to(w.root)
                src_id = w.source_id
                llm_enabled = w.llm_helper_enabled
                break
            except ValueError:
                pass
        item = extract_and_normalize(path, src_id, str(path.parent), llm_helper_enabled=llm_enabled)
        if item.ingest_status == "ingested" and item.chunks:
            sent, _, err_msg = publish_batch([item], api_url, admin_token)
            if err_msg:
                print(f"Publish failed for {path}: {err_msg}", file=sys.stderr)
            else:
                print(f"Published {path.name} ({sent} docs)")

    print("Watch mode: monitoring", len(watched), "source(s). Ctrl+C to stop.")
    try:
        run_watcher(watched, state_path, on_queued, poll_interval=5.0)
    except KeyboardInterrupt:
        print("\nStopped")
    return 0


def fetch_agent_config(api_url: str, admin_token: str) -> tuple[list[dict], str | None]:
    """Fetch agent config from main app. Returns (sources, error)."""
    import urllib.request
    import json

    url = f"{api_url.rstrip('/')}/v1/ingest/agent/config"
    req = urllib.request.Request(url)
    if admin_token:
        req.add_header("Authorization", f"Bearer {admin_token}")
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            data = json.loads(resp.read().decode())
            return data.get("sources", []), None
    except Exception as e:
        return [], str(e)


def main() -> int:
    parser = argparse.ArgumentParser(description="MeshMind filesystem ingestion agent")
    parser.add_argument("--one-shot", metavar="PATH", help="Scan folder once and ingest")
    parser.add_argument("--source-id", default="agent-fs", help="MeshMind source_id")
    parser.add_argument("--api-url", default=os.environ.get("MESHMIND_API_URL", "http://127.0.0.1:9900"))
    parser.add_argument("--admin-token", default=os.environ.get("MESHMIND_ADMIN_TOKEN", ""))
    parser.add_argument("--watch", action="store_true", help="Run watcher (TODO)")
    parser.add_argument("--config-from-api", action="store_true", help="Fetch sources from main app, run one-shot per source")
    args = parser.parse_args()

    if args.one_shot:
        return one_shot_ingest(
            Path(args.one_shot),
            args.source_id,
            args.api_url,
            args.admin_token,
        )

    if args.config_from_api:
        sources, err = fetch_agent_config(args.api_url, args.admin_token)
        if err:
            print(f"Config fetch failed: {err}", file=sys.stderr)
            return 1
        if not sources:
            print("No agent sources configured in main app (meshmind.toml [[agent_sources]])")
            return 0
        failed = 0
        for s in sources:
            path_str = s.get("path", "")
            src_id = s.get("source_id", "agent-fs")
            if not path_str:
                continue
            p = Path(path_str)
            if not p.exists() or not p.is_dir():
                print(f"Skipping {src_id}: not a directory: {path_str}", file=sys.stderr)
                failed += 1
                continue
            rc = one_shot_ingest(
                p, src_id, args.api_url, args.admin_token,
                llm_helper_enabled=s.get("llm_helper_enabled", False),
            )
            if rc != 0:
                failed += 1
        return 1 if failed > 0 else 0

    if args.watch:
        return run_watch_mode(args.api_url, args.admin_token)

    # Default: print config
    watch_dirs = os.environ.get("WATCH_DIRS", "")
    print("MeshMind filesystem ingestion agent (local-only)")
    print(f"  API URL: {args.api_url}")
    print(f"  Watch dirs: {watch_dirs or '(none configured)'}")
    print("  Usage: --one-shot /path/to/folder [--source-id src-1]")
    print("         --config-from-api  Fetch sources from main app, ingest each")
    return 0


if __name__ == "__main__":
    sys.exit(main())
