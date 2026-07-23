#!/usr/bin/env python3
import argparse
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request


COMMENT_MARKER = "<!-- k8s-db-dry-run -->"
MAX_COMMENT_LENGTH = 60_000
MAX_COMMENT_OPERATIONS = 200


def required_env(names):
    missing = [name for name in names if not os.environ.get(name)]
    if missing:
        raise RuntimeError(
            "missing required environment variables: " + ", ".join(missing)
        )
    return {name: os.environ[name] for name in names}


def request_json(url, token, method="GET", payload=None):
    data = None
    if payload is not None:
        data = json.dumps(payload).encode()
    request = urllib.request.Request(
        url,
        data=data,
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
            "Accept": "application/vnd.github+json, application/json",
            "User-Agent": "WoodpeckerCI-k8s-db-sync/1.0",
            "X-GitHub-Api-Version": "2022-11-28",
        },
        method=method,
    )
    try:
        with urllib.request.urlopen(request, timeout=180) as response:
            return json.loads(response.read().decode())
    except urllib.error.HTTPError as error:
        body = error.read().decode()
        raise RuntimeError(f"request failed with HTTP {error.code}: {body}") from error
    except urllib.error.URLError as error:
        raise RuntimeError(f"request failed: {error}") from error


def sync_schema(env, dry_run):
    try:
        postgres_port = int(env["POSTGRES_PORT"])
    except ValueError as error:
        raise RuntimeError("POSTGRES_PORT must be an integer") from error

    payload = {
        "github": {
            "owner": "takumi3488",
            "repo": "bookmeter-discounts",
            "path": "init.sql",
            "ref": env["CI_COMMIT_SHA"],
            "token": env["GITHUB_TOKEN"],
        },
        "postgresql": {
            "user": env["POSTGRES_USER"],
            "password": env["POSTGRES_PASSWORD"],
            "host": env["POSTGRES_HOST"],
            "port": postgres_port,
            "dbname": "bookmeter_discounts",
            "sslmode": "disable",
            "schema": "public",
        },
        "dry_run": dry_run,
    }
    result = request_json(
        env["K8S_DB_URL"], env["GITHUB_TOKEN"], method="POST", payload=payload
    )
    if result.get("status") == "error":
        raise RuntimeError(f"k8s-db returned an error: {result.get('error')}")
    return result


def operation_lines(operations):
    if not operations:
        return ["- None"]
    lines = [
        f"- `{operation.get('type', 'unknown')}` `{operation.get('object', 'unknown')}`"
        for operation in operations[:MAX_COMMENT_OPERATIONS]
    ]
    if len(operations) > MAX_COMMENT_OPERATIONS:
        lines.append(f"- ... and {len(operations) - MAX_COMMENT_OPERATIONS} more")
    return lines


def build_comment(result, commit_sha):
    operations = result.get("operations") or []
    sql = result.get("sql") or "-- No migration SQL generated."
    prefix = "\n".join(
        [
            COMMENT_MARKER,
            "## k8s-db dry run",
            "",
            f"Commit: `{commit_sha}`",
            f"Status: `{result.get('status', 'unknown')}`",
            f"Operations: `{len(operations)}`",
            "",
            *operation_lines(operations),
            "",
            "<details>",
            "<summary>Generated migration SQL</summary>",
            "",
            "```sql",
        ]
    )
    suffix = "\n```\n</details>"
    available = MAX_COMMENT_LENGTH - len(prefix) - len(suffix) - 2
    if len(sql) > available:
        truncation = "\n-- Migration SQL was truncated."
        sql = sql[: available - len(truncation)] + truncation
    return prefix + "\n" + sql + suffix


def upsert_pr_comment(env, body):
    repo = urllib.parse.quote(env["CI_REPO"], safe="/")
    pull_request = urllib.parse.quote(env["CI_COMMIT_PULL_REQUEST"], safe="")
    base_url = f"https://api.github.com/repos/{repo}/issues/{pull_request}/comments"
    existing = None
    page = 1
    while existing is None:
        comments = request_json(
            f"{base_url}?per_page=100&page={page}", env["GITHUB_TOKEN"]
        )
        existing = next(
            (
                comment
                for comment in comments
                if comment.get("body", "").startswith(COMMENT_MARKER)
            ),
            None,
        )
        if existing is not None or len(comments) < 100:
            break
        page += 1
    if existing:
        request_json(
            existing["url"], env["GITHUB_TOKEN"], method="PATCH", payload={"body": body}
        )
        return "updated"
    request_json(base_url, env["GITHUB_TOKEN"], method="POST", payload={"body": body})
    return "created"


def parse_args():
    parser = argparse.ArgumentParser()
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--dry-run", action="store_true")
    mode.add_argument("--apply", action="store_true")
    parser.add_argument("--comment", action="store_true")
    args = parser.parse_args()
    if args.comment and not args.dry_run:
        parser.error("--comment requires --dry-run")
    return args


def main():
    args = parse_args()
    names = [
        "K8S_DB_URL",
        "GITHUB_TOKEN",
        "POSTGRES_USER",
        "POSTGRES_PASSWORD",
        "POSTGRES_HOST",
        "POSTGRES_PORT",
        "CI_COMMIT_SHA",
    ]
    if args.comment:
        names.extend(["CI_REPO", "CI_COMMIT_PULL_REQUEST"])

    try:
        env = required_env(names)
        result = sync_schema(env, dry_run=args.dry_run)
        operations = result.get("operations") or []
        print("k8s-db sync completed")
        print(f"status: {result.get('status')}")
        print(f"files_processed: {result.get('files_processed')}")
        print(f"operations: {len(operations)}")
        if args.comment:
            action = upsert_pr_comment(env, build_comment(result, env["CI_COMMIT_SHA"]))
            print(f"PR comment {action}")
    except RuntimeError as error:
        print(str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
