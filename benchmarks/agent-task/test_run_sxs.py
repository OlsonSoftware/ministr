"""Focused tests for the real-agent benchmark harness instrumentation."""

import importlib.util
import json
import os
import tempfile
import threading
import time
import unittest


HERE = os.path.dirname(os.path.abspath(__file__))
SPEC = importlib.util.spec_from_file_location("run_sxs", os.path.join(HERE, "run_sxs.py"))
RUN_SXS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUN_SXS)


def tool_event(tool_id, name="mcp__ministr__ministr_toc", tool_input=None):
    return {
        "type": "assistant",
        "message": {
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "id": tool_id,
                "name": name,
                "input": tool_input or {},
            }],
        },
    }


class TranscriptAccountingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.repo = os.path.join(self.temp.name, "repo")
        self.projects = os.path.join(self.temp.name, "projects")
        os.makedirs(self.repo)
        self.project_dir = RUN_SXS.transcript_project_dir(self.repo, self.projects)
        os.makedirs(self.project_dir)

    def tearDown(self):
        self.temp.cleanup()

    def write_events(self, name, events):
        path = os.path.join(self.project_dir, name)
        with open(path, "w") as transcript:
            for event in events:
                transcript.write(json.dumps(event) + "\n")
        return path

    def test_matches_claude_slug_for_tempfile_underscores(self):
        underscored_repo = os.path.join(self.temp.name, "repo_with__underscores")
        os.makedirs(underscored_repo)

        project_dir = RUN_SXS.transcript_project_dir(underscored_repo, self.projects)

        self.assertTrue(project_dir.endswith("-repo-with--underscores"))

    def test_scans_all_transcripts_not_only_the_largest(self):
        self.write_events("large-stub.jsonl", [{"type": "user", "padding": "x" * 4096}])
        self.write_events("current.jsonl", [tool_event("call-1")])

        self.assertEqual(
            RUN_SXS.count_ministr_calls(
                self.repo, wait_seconds=0, projects_root=self.projects
            ),
            1,
        )

    def test_checkpoint_ignores_old_calls_and_waits_for_delayed_flush(self):
        path = self.write_events("session.jsonl", [tool_event("old-call")])
        checkpoint = RUN_SXS.transcript_checkpoint(self.repo, self.projects)

        def delayed_append():
            time.sleep(0.1)
            with open(path, "a") as transcript:
                transcript.write(json.dumps(tool_event("new-call")) + "\n")

        writer = threading.Thread(target=delayed_append)
        writer.start()
        count = RUN_SXS.count_ministr_calls(
            self.repo,
            checkpoint=checkpoint,
            wait_seconds=1,
            projects_root=self.projects,
        )
        writer.join()

        self.assertEqual(count, 1)

    def test_deduplicates_mirrored_tool_use_ids(self):
        event = tool_event("same-call")
        self.write_events("session.jsonl", [event])
        subagents = os.path.join(self.project_dir, "subagents")
        os.makedirs(subagents)
        with open(os.path.join(subagents, "agent.jsonl"), "w") as transcript:
            transcript.write(json.dumps({"wrapper": event}) + "\n")

        self.assertEqual(
            RUN_SXS.count_ministr_calls(
                self.repo, wait_seconds=0, projects_root=self.projects
            ),
            1,
        )

    def test_bash_grep_outcome_uses_protocol_result_not_model_prose(self):
        command = {'command': 'grep -rn "def " . | head -3'}
        success = tool_event("grep-ok", name="Bash", tool_input=command)
        success_result = {
            "message": {"content": [{
                "type": "tool_result", "tool_use_id": "grep-ok", "content": "matches"
            }]}
        }
        self.write_events("success.jsonl", [success, success_result])

        self.assertEqual(
            RUN_SXS.bash_grep_outcome(
                self.repo, wait_seconds=0, projects_root=self.projects
            ),
            "executed",
        )

        os.remove(os.path.join(self.project_dir, "success.jsonl"))
        denied = tool_event("grep-denied", name="Bash", tool_input=command)
        denied_result = {
            "message": {"content": [{
                "type": "tool_result", "tool_use_id": "grep-denied",
                "content": "permission denied", "is_error": True,
            }]}
        }
        self.write_events("denied.jsonl", [denied, denied_result])

        self.assertEqual(
            RUN_SXS.bash_grep_outcome(
                self.repo, wait_seconds=0, projects_root=self.projects
            ),
            "denied",
        )


if __name__ == "__main__":
    unittest.main()
