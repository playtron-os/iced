#!/usr/bin/env python3
"""Publish the staged -kora crates to crates.io, fork by fork in dependency order.

Usage: publish.py <out-dir> [--dry-run]

Run stage.py into <out-dir> without --local first. Versions crates.io already
has are skipped, so a failed run can simply be started again. When crates.io
rate-limits new crates, this waits until the time it names and carries on.
"""

import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from email.utils import parsedate_to_datetime
from pathlib import Path

from stage import FORKS, kora_name

USER_AGENT = "kora-publish (https://github.com/playtron-os/iced)"


def published(name, version):
    url = f"https://crates.io/api/v1/crates/{name}/{version}"
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    try:
        urllib.request.urlopen(request)
        return True
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return False
        raise


def retry_delay(output):
    """Seconds to wait when crates.io refused a publish for its rate limit, else None."""
    if "429" not in output and "too many" not in output.lower():
        return None
    match = re.search(r"try again after (.+? GMT)", output)
    if not match:
        return 600
    wait = parsedate_to_datetime(match.group(1)).timestamp() - time.time()
    return max(wait, 0) + 15


def publish_fork(workspace, crates, version, dry_run):
    while True:
        pending = [c for c in crates if not published(c, version)]
        if not pending:
            print(f"{workspace.name}: all {len(crates)} crates are on crates.io", flush=True)
            return
        print(f"{workspace.name}: publishing {' '.join(pending)}", flush=True)
        command = ["cargo", "publish", "--allow-dirty"]
        if dry_run:
            command.append("--dry-run")
        for crate in pending:
            command += ["-p", crate]
        result = subprocess.run(command, cwd=workspace, capture_output=True, text=True)
        output = result.stdout + result.stderr
        print(output[-4000:], flush=True)
        if result.returncode == 0:
            if dry_run:
                return
            continue
        if "already exists" in output:
            # The API can trail the index for a moment after a publish.
            time.sleep(30)
            continue
        delay = retry_delay(output)
        if delay is None:
            raise SystemExit(f"{workspace.name}: cargo publish failed")
        print(f"{workspace.name}: rate-limited, waiting {int(delay)}s", flush=True)
        time.sleep(delay)


def main():
    stage = Path(sys.argv[1])
    dry_run = "--dry-run" in sys.argv[2:]
    for fork, (version, crates) in FORKS.items():
        if crates:
            publish_fork(stage / fork, [kora_name(c) for c in crates], version, dry_run)


if __name__ == "__main__":
    main()
