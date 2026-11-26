"""Tests of pages-deploy.py. Run with: python3 -m unittest discover -s scripts"""

import importlib.util
import io
import json
import tempfile
import unittest
import urllib.error
from contextlib import redirect_stdout
from pathlib import Path
from unittest import mock

_spec = importlib.util.spec_from_file_location("pages_deploy", Path(__file__).with_name("pages-deploy.py"))
pd = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(pd)

API = "https://api.example.com"
DEPLOYMENTS = f"{API}/repos/o/r/pages/deployments"


class FakeApi:
    """Answers the requests main makes, the deployment's states taken from `states` in turn
    (an exception in it is raised); records the requests."""

    def __init__(self, states):
        self.states = list(states)
        self.calls = []

    def __call__(self, url, token, data=None):
        self.calls.append((url, token, data))
        if url == "https://token.example.com":
            return {"value": "oidc"}
        if url == DEPLOYMENTS:
            return {"id": "d1", "page_url": "https://o.github.io/r/"}
        if url == f"{DEPLOYMENTS}/d1":
            state = self.states.pop(0)
            if isinstance(state, Exception):
                raise state
            return {"status": state}
        raise AssertionError(url)


class Main(unittest.TestCase):
    def run_main(self, states):
        api = FakeApi(states)
        sleeps = []
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "output"
            env = {
                "GITHUB_API_URL": API + "/",
                "GITHUB_REPOSITORY": "o/r",
                "GH_TOKEN": "gh",
                "ARTIFACT_ID": "42",
                "BUILD_VERSION": "abc-7-1",
                "ACTIONS_ID_TOKEN_REQUEST_URL": "https://token.example.com",
                "ACTIONS_ID_TOKEN_REQUEST_TOKEN": "request",
                "GITHUB_OUTPUT": str(output),
            }
            log = io.StringIO()
            try:
                with redirect_stdout(log):
                    pd.main(env, api, sleeps.append)
            finally:
                self.output = output.read_text(encoding="utf-8") if output.exists() else ""
        return api, sleeps, log.getvalue()

    def test_deploys_with_the_runs_version_and_waits(self):
        api, sleeps, log = self.run_main(["deployment_queued", "syncing_files", "succeed"])
        oidc, create, *_ = api.calls
        self.assertEqual(oidc, ("https://token.example.com", "request", None))
        self.assertEqual(
            create, (DEPLOYMENTS, "gh", {"artifact_id": 42, "pages_build_version": "abc-7-1", "oidc_token": "oidc"})
        )
        self.assertEqual(len(api.calls), 5)
        self.assertEqual(sleeps, [pd.INTERVAL] * 2)
        self.assertEqual(self.output, "page_url=https://o.github.io/r/\n")
        self.assertIn("Deployment d1 of abc-7-1", log)
        self.assertIn("syncing_files", log)

    def test_a_failed_deployment_fails(self):
        with self.assertRaisesRegex(pd.Error, "Pages deployment d1: deployment_content_failed"):
            self.run_main(["deployment_queued", "deployment_content_failed"])
        # The address is still given.
        self.assertEqual(self.output, "page_url=https://o.github.io/r/\n")


class Cli(unittest.TestCase):
    def test_a_failure_is_a_workflow_error(self):
        def main(**_):
            raise pd.Error("Pages deployment d1: deployment_lost")

        log = io.StringIO()
        with mock.patch.object(pd, "main", main), redirect_stdout(log):
            self.assertEqual(pd.cli(), 1)
        self.assertEqual(log.getvalue(), "::error::Pages deployment d1: deployment_lost\n")
        with mock.patch.object(pd, "main", lambda **_: None):
            self.assertEqual(pd.cli(), 0)


class Wait(unittest.TestCase):
    def test_ends_when_done(self):
        states = iter(["deployment_in_progress", "succeed"])
        sleeps = []
        with redirect_stdout(io.StringIO()):
            pd.wait(lambda: next(states), sleeps.append)
        self.assertEqual(sleeps, [pd.INTERVAL])

    def test_times_out(self):
        sleeps = []
        with redirect_stdout(io.StringIO()), self.assertRaisesRegex(pd.Error, "not done after 1 minutes"):
            pd.wait(lambda: "deployment_in_progress", sleeps.append, timeout=60, interval=5)
        self.assertEqual(len(sleeps), 12)

    def test_reads_that_fail_now_and_then_are_tolerated(self):
        states = iter([OSError("reset"), OSError("reset"), "succeed"])

        def status():
            s = next(states)
            if isinstance(s, Exception):
                raise s
            return s

        with redirect_stdout(io.StringIO()):
            pd.wait(status, lambda _: None)

    def test_too_many_failed_reads_in_a_row_fail(self):
        def status():
            raise pd.Error("502")

        with self.assertRaisesRegex(pd.Error, "its state can't be read: 502"):
            pd.wait(status, lambda _: None)


class Request(unittest.TestCase):
    def answer(self, body):
        r = mock.MagicMock()
        r.__enter__.return_value = io.BytesIO(json.dumps(body).encode())
        return r

    def test_get_and_post(self):
        with mock.patch("urllib.request.urlopen", return_value=self.answer({"ok": 1})) as urlopen:
            self.assertEqual(pd.request("https://x/a", "t"), {"ok": 1})
        req = urlopen.call_args[0][0]
        self.assertEqual((req.method, req.data, req.get_header("Authorization")), ("GET", None, "Bearer t"))
        with mock.patch("urllib.request.urlopen", return_value=self.answer({"id": 1})) as urlopen:
            pd.request("https://x/a", "t", {"a": 1})
        req = urlopen.call_args[0][0]
        self.assertEqual((req.method, json.loads(req.data)), ("POST", {"a": 1}))
        self.assertEqual(req.get_header("Content-type"), "application/json")

    def test_an_http_error_says_what_the_api_answered(self):
        error = urllib.error.HTTPError("https://x/a", 404, "Not Found", {}, io.BytesIO(b'{"message": "Not Found"}'))
        with mock.patch("urllib.request.urlopen", side_effect=error):
            with self.assertRaisesRegex(pd.Error, 'GET https://x/a: 404 {"message": "Not Found"}'):
                pd.request("https://x/a", "t")


if __name__ == "__main__":
    unittest.main()
