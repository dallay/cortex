#!/usr/bin/env python3
"""Safely run argument-bearing Make targets without shell-evaluating ARGS."""

import os
import shlex
import subprocess
import sys


def main() -> int:
    if len(sys.argv) < 2 or sys.argv[1] not in {"huginn", "push"}:
        print("usage: exec-args.py {huginn|push} [command...]", file=sys.stderr)
        return 2

    mode = sys.argv[1]
    try:
        if len(sys.argv) > 2:
            arguments = sys.argv[2:]
        else:
            arguments = shlex.split(os.environ.get("ARGS", ""))
    except ValueError as error:
        print(f"invalid ARGS: {error}", file=sys.stderr)
        return 2
    if mode == "huginn":
        command = ["cargo", "run", "-p", "huginn", "--", *arguments]
        return subprocess.run(command, check=False).returncode

    push_status = subprocess.run(["git", "push", *arguments], check=False).returncode
    if push_status:
        return push_status

    clean_status = subprocess.run(["cargo", "clean"], check=False).returncode
    if clean_status:
        return clean_status
    return subprocess.run(["cargo", "llvm-cov", "clean", "--workspace"], check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main())
