#!/usr/bin/env python3
"""Deploys this workflow run's Pages artifact to GitHub Pages, with the Pages API.

Usage: pages-deploy.py, in a job with the permissions pages: write and id-token: write.

Not actions/deploy-pages, which names each deployment after the commit (GITHUB_SHA): Pages
takes a second deployment of a commit for the first one and keeps its files, the run still
reporting success. Publishing a release on the commit main was just deployed from, as
usual, then left the download page without it. Here the deployment's version is
BUILD_VERSION, unique to each run.

Reads from the environment: GH_TOKEN, GITHUB_REPOSITORY, ARTIFACT_ID (the artifact
actions/upload-pages-artifact uploaded), BUILD_VERSION, ACTIONS_ID_TOKEN_REQUEST_URL and
ACTIONS_ID_TOKEN_REQUEST_TOKEN (set by Actions with id-token: write), GITHUB_API_URL
(optional) and GITHUB_OUTPUT (the site's address is written to it as page_url). Waits for
the deployment as deploy-pages does: fails when Pages says it failed, or after 10 minutes.
Run by .github/workflows/pages.yml.
"""

import json
import os
import sys
import time
import urllib.error
import urllib.request

# A deployment's states that end it (the others say it is still under way).
DONE = "succeed"
FAILED = {"deployment_failed", "deployment_content_failed", "deployment_cancelled", "deployment_lost"}
# How often its state is read, how long it may take, and how many reads may fail in a row.
INTERVAL = 5
TIMEOUT = 600
MAX_ERRORS = 10


class Error(Exception):
    pass


def request(url, token, data=None):
    """The JSON answer to a GET of `url`, or a POST of `data`, with `token`."""
    body = None if data is None else json.dumps(data).encode()
    req = urllib.request.Request(url, data=body, method="GET" if body is None else "POST")
    req.add_header("Authorization", f"Bearer {token}")
    req.add_header("Accept", "application/vnd.github+json")
    if body is not None:
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            return json.load(r)
    except urllib.error.HTTPError as e:
        raise Error(f"{req.method} {url}: {e.code} {e.read().decode(errors='replace')}") from e


def oidc_token(env, fetch):
    """The workflow's OpenID Connect token, which Pages asks for."""
    return fetch(env["ACTIONS_ID_TOKEN_REQUEST_URL"], env["ACTIONS_ID_TOKEN_REQUEST_TOKEN"])["value"]


def wait(status, sleep, timeout=TIMEOUT, interval=INTERVAL):
    """Reads the deployment's state with `status` until it ends; Error unless it succeeded."""
    errors = 0
    for _ in range(timeout // interval):
        try:
            state = status()
            errors = 0
        except (OSError, Error) as e:
            errors += 1
            if errors >= MAX_ERRORS:
                raise Error(f"its state can't be read: {e}") from e
            state = None
        if state:
            print(state)
        if state == DONE:
            return
        if state in FAILED:
            raise Error(state)
        sleep(interval)
    raise Error(f"not done after {timeout // 60} minutes")


def main(env=os.environ, fetch=request, sleep=time.sleep):
    api = env.get("GITHUB_API_URL", "https://api.github.com").rstrip("/")
    deployments = f"{api}/repos/{env['GITHUB_REPOSITORY']}/pages/deployments"
    token = env["GH_TOKEN"]
    payload = {
        "artifact_id": int(env["ARTIFACT_ID"]),
        "pages_build_version": env["BUILD_VERSION"],
        "oidc_token": oidc_token(env, fetch),
    }
    deployment = fetch(deployments, token, payload)
    print(f"Deployment {deployment['id']} of {env['BUILD_VERSION']}")
    with open(env["GITHUB_OUTPUT"], "a", encoding="utf-8") as out:
        out.write(f"page_url={deployment['page_url']}\n")
    try:
        wait(lambda: fetch(f"{deployments}/{deployment['id']}", token)["status"], sleep)
    except Error as e:
        raise Error(f"Pages deployment {deployment['id']}: {e}") from e


def cli(**kw):
    """main, its failure reported as a workflow error; the exit status."""
    try:
        main(**kw)
    except Error as e:
        print(f"::error::{e}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(cli())
