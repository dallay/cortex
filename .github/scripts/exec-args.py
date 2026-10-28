#!/usr/bin/env python3
"""Safely run argument-bearing Make targets without shell-evaluating ARGS."""

import os
import shlex
import subprocess
import sys


def main() -> int:
    if len(sys.argv) < 2 or sys.argv[1] not in {"huginn", "push"}:
        print("usage: make-args.py {huginn|push} [command...]", file=sys.stderr)
        return 2

    mode = sys.argv[1]
    arguments = []
    for raw in sys.argv[2:]:
        arguments.extend(shlex.split(raw))
    if not arguments:
        arguments = shlex.split(os.environ.get("ARGS", ""))
    if mode == "huginn":
        return subprocess.run(["cargo", "run", "-p", "huginn", "--", *arguments], check=False).returncode

    push_status = subprocess.run(["git", "push", *arguments], check=False).returncode
    if push_status:
        return push_status

    clean_status = subprocess.run(["cargo", "clean"], check=False).returncode
    if clean_status:
        return clean_status
    return subprocess.run(
        ["cargo", "llvm-cov", "clean", "--workspace"], check=False
    ).returncode


if __name__ == "__main__":
    raise SystemExit(main())
