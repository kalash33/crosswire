#!/usr/bin/env python3
"""Checks the h1 golden corpus: pairing, schema, and code coverage.

Usage: python3 -I scripts/corpus-check.py corpus/h1
"""
import json
import os
import sys

CODES = [
    "duplicate_content_length", "conflicting_framing", "obs_fold", "bare_lf",
    "bare_cr", "whitespace_before_colon", "trailing_space_in_name",
    "leading_whitespace_before_start_line", "non_canonical_header_case",
    "invalid_token_char_in_name", "invalid_char_in_value", "missing_host",
    "duplicate_host", "absolute_form_target", "chunk_extension_present",
    "trailer_present", "chunk_size_leading_zeros", "invalid_chunk_size",
    "content_length_non_numeric", "content_length_leading_plus",
    "unknown_transfer_coding", "body_after_no_body_status", "expect_100_continue",
    "http_0_9_request", "version_unusual",
]
RULES = {"transfer_encoding_wins", "first_content_length", "no_body_status",
         "head_method", "connect_success"}
KINDS = {"request", "response"}
FRAMINGS = {"none", "content_length", "chunked", "until_close", "upgrade", "ambiguous"}


def main():
    if len(sys.argv) != 2:
        print("usage: corpus-check.py <corpus dir>", file=sys.stderr)
        return 2
    cases = os.path.join(sys.argv[1], "cases")
    files = set(os.listdir(cases))
    errors = []
    msgs = {f[:-4] for f in files if f.endswith(".msg")}
    gens = {f[:-9] for f in files if f.endswith(".gen.json")}
    expects = {f[:-12] for f in files if f.endswith(".expect.json")}
    for n in sorted(msgs - expects):
        errors.append(f"{n}.msg has no .expect.json")
    for n in sorted(expects - msgs):
        errors.append(f"{n}.expect.json has no .msg")
    covered = set()
    total = 0
    for n in sorted(expects | gens):
        is_gen = n in gens
        path = os.path.join(cases, n + (".gen.json" if is_gen else ".expect.json"))
        try:
            with open(path, encoding="utf-8") as fh:
                e = json.load(fh)
        except (OSError, ValueError) as ex:
            errors.append(f"{n}: cannot parse json: {ex}")
            continue
        total += 1
        if e.get("kind") not in KINDS:
            errors.append(f"{n}: invalid kind {e.get('kind')!r}")
        if e.get("framing") not in FRAMINGS:
            errors.append(f"{n}: invalid framing {e.get('framing')!r}")
        if e.get("kind") == "response" and not is_gen and "ctx" not in e:
            errors.append(f"{n}: response without ctx")
        if "resolved" in e and e["resolved"] not in FRAMINGS - {"ambiguous"}:
            errors.append(f"{n}: invalid resolved {e['resolved']!r}")
        if "rule" in e and e["rule"] not in RULES:
            errors.append(f"{n}: invalid rule {e['rule']!r}")
        if "body" in e and "body_hex" in e:
            errors.append(f"{n}: body and body_hex are mutually exclusive")
        if "pipelined" in e and not isinstance(e["pipelined"], bool):
            errors.append(f"{n}: pipelined must be a bool")
        skip = "known_bug" in e
        for d in e.get("diagnostics", []):
            if d not in CODES:
                errors.append(f"{n}: unknown code {d!r}")
            if not skip:
                covered.add(d)
        if "error" in e:
            if e["error"] not in CODES:
                errors.append(f"{n}: unknown error code {e['error']!r}")
            if not skip:
                covered.add(e["error"])
    missing = [c for c in CODES if c not in covered]
    for c in missing:
        errors.append(f"code not covered: {c}")
    for err in errors:
        print("error:", err, file=sys.stderr)
    print(f"{len(CODES) - len(missing)}/{len(CODES)} codes covered, {total} cases")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
