# Crosswire redaction golden + copy-as-curl / copy-raw — design spec

> **Status:** design for the first "Harden & prove (alpha.2 DoD)" sub-plan.
> Closes §13 security gate #14 (redaction golden) and ships the §7.6
> copy-as-curl and copy-raw actions. Authority: the product spec
> `docs/superpowers/specs/2026-10-22-crosswire-design.md` (§7.6, §9.4, §11.4,
> §13). Where this document and the product spec disagree, the product spec wins.

## 1. Goal

A pentester can copy any recorded request as a runnable **curl** command or as
its **raw bytes**, in a Burp-compatible *unredacted* form (the default, for
replay) and an explicit *redacted* form (for sharing). Every copy is audited
with which form was taken. A CI **golden** proves that the redacted copies, and
the process logs, contain none of a seeded project's secrets — while the
unredacted copies still do, so the switch is provably real, not a no-op.

This is the §13 DoD gate "redaction golden": *copy-as-curl (redacted), copy-raw
and logs of a seeded project contain none of the seeded secrets.*

## 2. Scope

**In scope**
- Inline-body redaction in `crosswire-model` (the value-pattern half of §11.4
  applied to inline request/response body bytes).
- A documented, tested shell-quoting routine and a `Message → curl` renderer
  for POSIX `sh` and PowerShell, in `crosswire-tools`.
- Raw-request reconstruction (head via `write_exact` + body), redacted and
  unredacted.
- Two audited API routes (curl, raw) with a `redacted` parameter; new
  `copy.curl` / `copy.raw` audit actions recording the variant.
- UI clipboard actions (Burp shortcut = unredacted; a second action =
  redacted) with keymap bindings, in history, repeater and intercept detail.
- The golden gate test over a real seeded `crosswire serve`.

**Out of scope (tracked, not regressed)**
- Blob/pack (non-inline) body redaction — the existing deferral stands; the
  golden seeds inline bodies only (a blob-body golden lands with the exporter
  redaction work).
- HAR export and its own redaction golden — v0.2a.
- The preview zero-egress isolation layer (gate #4) — its own later sub-plan.
- FTS/MCP/plugin redaction consumers — those paths don't exist yet; this
  sub-plan only wires the consumers that do (copy + logs).

## 3. Global constraints (from the product spec, verbatim values)

- `unsafe_code = "forbid"` workspace-wide; MSRV 1.88; `cargo build` Node-free.
- Conventional Commits, ≤72-char subjects, no attribution trailers.
- `cargo deny` clean; any new dep justified and in the lockfile.
- No query-semantics or wire-format change; additive only.
- Full gate before a task is done: `just lint`, `just doc`, `cargo deny check`,
  `cargo test`, and the UI e2e where the UI changes.
- Redaction placeholder is the existing one in `redact.rs` (header values
  replaced whole; value patterns replaced where they match) — this spec does
  **not** change the placeholder text or the default policy set.

## 4. Redaction policy today, and the one extension

`RedactionPolicy` (`crosswire-model/src/redact.rs`) already redacts **heads**
(default header names: Authorization, Proxy-Authorization, Cookie, Set-Cookie,
X-API-Key, X-Auth-Token, X-Amz-Security-Token; value patterns: `Bearer `,
`access_token=`, `api_key=`, `@jwt`) by span, re-parsing the rebuilt head and
labelling it `Normalized`. Diffs are redacted. **Inline bodies are not** — the
type docs say so.

**Extension:** `redact_message` also redacts an **inline** body
(`Body` variants that carry bytes in-process, not a blob/pack reference) by
applying the `value_patterns` to the body bytes with the existing
`replace_ranges` / `find_matches` machinery (the same code `scrub` uses). Header
names do not apply to a body. A blob/pack body is left untouched and the result
is still labelled so a consumer knows the body may be unredacted (an existing
`truncated_at`-style marker or an explicit `body_redacted: bool` on the copy
response — chosen in the plan). The redaction remains pure and total (no I/O,
no partial application).

## 5. curl rendering (`crosswire-tools`, new module)

`fn to_curl(req: &Message, shell: Shell) -> String` where
`enum Shell { Posix, PowerShell }`.

- Method via `-X` when not GET; URL reconstructed from `target_addr` +
  request-target (scheme from how the exchange was recorded — tls ⇒ https);
  each header as `-H`; an inline body as `--data-binary` (never `-d`, which
  mangles newlines); a body that is a blob reference renders a
  `# body omitted (NNN bytes in pack)` comment line, not the bytes.
- **Quoting routine (documented, the gate):**
  - POSIX: wrap each argument in single quotes; every embedded `'` becomes
    `'\''`. No other character is special inside single quotes, so this is
    total against `$()`, backticks, `;`, newlines, `|`, `&`, globs.
  - PowerShell: wrap in single quotes; every embedded `'` becomes `''`.
  - The routine is a single function per shell with a doc comment stating the
    rule and why it is complete.
- Determinism: header order preserves the recorded order; no locale or env
  dependence.

## 6. Raw reconstruction

`fn to_raw(req: &Message) -> Vec<u8>` returns the head bytes (`write_exact`,
already in `crosswire-h1`) followed by the inline body bytes; a blob body is
omitted with no marker in the bytes (the response carries `body_omitted` out of
band). The redacted raw is `to_raw(&policy.redact_message(req))`; the unredacted
raw is `to_raw(req)`.

## 7. API

Two read routes on an existing exchange (Read scope; **audited despite being
GET**, because the copy choice is recorded per §7.6):

- `GET /api/v1/exchanges/{id}/curl?shell=sh|pwsh&redacted=bool`
  → `text/plain` curl command. Default `shell=sh`, `redacted=false`.
- `GET /api/v1/exchanges/{id}/raw?redacted=bool`
  → `application/octet-stream` raw request bytes. (The existing
  `/body/{side}` route is unchanged; this returns the whole request.)

Each emits an audit row: action `copy.curl` or `copy.raw`, target = exchange
ULID, with `{ redacted: bool, shell?: "sh"|"pwsh" }` detail. These join the
§9.4 audited-action set and the audit-completeness gate's asserted list.

Unknown `shell`, or an id that is not a recorded exchange, is the stable error
envelope (400 / 404), not a panic.

## 8. UI

In the history, repeater and intercept request views:

- **Copy as curl** — Burp-compatible default shortcut copies the **unredacted**
  curl (`shell` chosen by the running OS, or a setting); a second menu action
  "Copy as curl (redacted)" copies the redacted form.
- **Copy raw** — same two-variant pairing.
- Both call the audited endpoint, then write the returned text/bytes to the
  clipboard. The keymap gains `copy-as-curl` (reserved-name already present in
  the keymap module); the existing `copy-raw`/decode reservations are bound.
- No captured bytes are rendered as HTML (the §11.1 lint already bans it); the
  copied text never enters the DOM as markup.

## 9. The golden gate (the deliverable)

A Rust integration test over a real `crosswire serve` (mirroring
`crosswire-server/tests/gates.rs`):

1. Seed a project with exchanges whose **headers** carry each default-policy
   secret (an `Authorization: Bearer <jwt>`, a `Cookie:`, an `X-API-Key:`) and
   whose **inline body** carries `access_token=<secret>` and `api_key=<secret>`.
   The secret values are unique, high-entropy sentinels.
2. Assert the **redacted** curl (both shells) and the **redacted** raw contain
   **none** of the sentinels.
3. Assert the **unredacted** curl and raw **do** contain them (the switch is
   real).
4. Assert the process **logs** (stdout/stderr captured for the run, plus
   `audit.jsonl`) contain **none** of the sentinels — captured bytes and
   secrets never reach logs (§11.4: "Logs and crash reports never contain
   captured bytes").
5. A quoting sub-test: a header value containing `'`, `$(id)`, a backtick, a
   newline and `;` renders a curl string that, parsed by a shell tokenizer
   (a hostile-arg corpus asserted structurally — not executed), yields exactly
   the original bytes as one argument for both `sh` and PowerShell.

`docs/security-gates.md` gate #14 is updated from "no test" to the test path,
and the stale entries that review flagged (gates #3, #15, #16, #18 wording) are
corrected in the same pass.

## 10. Review focus (inputs the tests above must also cover)

- A secret that spans a header **and** a body (both redacted independently).
- A value pattern at the very end of the body buffer (no trailing delimiter).
- A request with **no** body (curl has no `--data-binary`; raw is head-only).
- A blob-body exchange (curl/raw omit the body with the out-of-band flag; no
  bytes leak; no panic).
- A tunnel/CONNECT exchange (nothing to redact; curl route returns a typed
  error or a documented minimal form, not a panic).
- `redacted=true` with a policy that matches nothing (output equals unredacted;
  still audited as redacted).

## 11. Acceptance

- Gate #14 golden green in CI (not maintainer-only): redacted curl/raw/logs
  carry no sentinel; unredacted carry them.
- copy-as-curl (sh + PowerShell) and copy-raw work from the UI, both variants,
  each audited; the audit-completeness gate's list includes `copy.curl` /
  `copy.raw`.
- Inline-body redaction covered by `crosswire-model` unit tests.
- Full gate green (`just lint`, `just doc`, `cargo deny check`, `cargo test`,
  UI e2e for the new actions).
- No wire/schema change; `docs/security-gates.md` accurate.
