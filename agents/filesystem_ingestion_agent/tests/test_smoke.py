"""Minimal smoke test for the Python ingestion agent."""

import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

# Add agent root to path
ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))


class TestSmoke(unittest.TestCase):
    def test_contract_models_import(self: "TestSmoke") -> None:
        """Verify contract models can be imported."""
        from contract_models import IngestedChunk, IngestedItem, PIPELINE_VERSION

        self.assertEqual(PIPELINE_VERSION, 1)
        chunk = IngestedChunk(chunk_index=0, chunk_text="hello", page_number=1)
        self.assertEqual(chunk.chunk_text, "hello")

    def test_main_runs(self: "TestSmoke") -> None:
        """Verify main entry point runs without error."""
        import main

        # Patch sys.argv for default (no --one-shot)
        import sys
        old = sys.argv
        sys.argv = ["main"]
        try:
            self.assertEqual(main.main(), 0)
        finally:
            sys.argv = old

    def test_publisher_import(self: "TestSmoke") -> None:
        """Verify publisher module imports."""
        import publisher

        self.assertTrue(hasattr(publisher, "publish_batch"))

    def test_watcher_import(self: "TestSmoke") -> None:
        """Verify watcher module imports."""
        import watcher

        self.assertTrue(hasattr(watcher, "scan_source"))
        self.assertTrue(hasattr(watcher, "WatchedSource"))
        self.assertTrue(hasattr(watcher, "compute_content_hash"))

    def test_extraction_import(self: "TestSmoke") -> None:
        """Verify extraction module imports."""
        import extraction

        self.assertTrue(hasattr(extraction, "extract_and_normalize"))
        self.assertTrue(hasattr(extraction, "get_provider"))

    def test_one_shot_flow(self: "TestSmoke") -> None:
        """One-shot: scan folder, extract, publish (mocked)."""
        import main

        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "sample.txt"
            p.write_text("Q1 2024 revenue increased 12%", encoding="utf-8")

            captured: list = []

            def fake_publish(items: list, _api_url: str, _token: str) -> tuple[int, int, str | None]:
                captured.extend(items)
                return len(items), len(items), None

            with patch("publisher.publish_batch", fake_publish):
                rc = main.one_shot_ingest(
                    Path(tmp), "agent-fs", "http://localhost:9900", "test-token"
                )

            self.assertEqual(rc, 0, "one_shot_ingest should succeed")
            self.assertEqual(len(captured), 1, "one item should be extracted")
            item = captured[0]
            self.assertIn("Q1 2024 revenue", item.extracted_text)
            self.assertEqual(item.source_id, "agent-fs")
            self.assertEqual(item.source_type, "filesystem")


if __name__ == "__main__":
    unittest.main()
