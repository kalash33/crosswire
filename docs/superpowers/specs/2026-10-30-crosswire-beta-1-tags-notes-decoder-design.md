# Crosswire beta-1: tags/notes UI + decoder pane — design spec

> **Status:** first of three beta sub-plans (§14 "beta — daily tools"). The §13 security
> gates are all proven; this sub-plan ships the two cut-list items that need **no backend
> change** (tags/notes 8 h, decoder pane 12 h). Authority: the product spec
> `docs/superpowers/specs/2026-10-22-crosswire-design.md` (§7.0 history columns and
> context menu, §7.6 small daily tools, §11.1 untrusted content rendering). Where this
> document and the product spec disagree, the product spec wins.
>
> **The beta block, decomposed (each its own spec → plan → execution):**
> beta-1 tags/notes UI + decoder pane (this) → beta-2 response intercept → beta-3
> match-and-replace (`crosswire-rules`). The order is the §14 cut-list order.

## 1. Goal

Give the operator the two daily tools a Burp user reaches for most: **annotate** an
exchange (tags and notes, editable in the detail view, visible and filterable in history)
and **decode** any selected bytes (URL, Base64, hex, HTML entities, JWT view) without
leaving the app — both rendered as text only, so the §11.1 XSS gate keeps holding.

## 2. What already exists (this sub-plan builds only what is missing)

| Piece | Exists | Missing |
|---|---|---|
| Tags/notes storage + model | `Exchange.notes`, `exchange_tags` table (Plan 6), `Store::exchange_set_tags / exchange_set_notes` | — |
| Tags/notes API | `PATCH /api/v1/exchanges/{id}` with `{ tags?, notes? }` (`PatchBody` in `ui/src/lib/api/types.ts`); `GET /exchanges?tag=` filter | — |
| Tags in the UI | read-only chips in `ExchangeDetail` header; a read-only **Tags column** in the `History.svelte` rows; `filter-tag` input wired to `tag=`; the XSS corpus already seeds payloads into `tags` | an **editor** (add/remove tags, notes), the row "Tag…" action, an in-memory row patch after a save (the history store has no row-update method today) |
| Decoder | `Shell.svelte` registers `decode-selection` (`Ctrl+Shift+D`) and shows an **encode-only** popover (Base64 / URL / hex of the DOM selection, text-only) | **decoding**, hex and HTML entities both ways, a JWT view, chained steps, typed/pasted input, copy-out, a pane component instead of inline Shell code |

No Rust changes. No new npm or cargo dependency (the codecs are hand-rolled; no `jose`).

## 3. Tags and notes (§7.0, §7.6)

- **Client:** `ui/src/lib/api/exchanges.ts` → `patchExchange(id, body: PatchBody): Promise<void>` over the existing `apiSend('PATCH', …)`. The route returns `200` with an **empty body**, so the client resolves on success and the editor applies the values it sent (it never reads a response body).
- **Editor** (`ui/src/lib/components/TagsNotesEditor.svelte`, mounted in the `ExchangeDetail` header, replacing the read-only chips): tag chips each with a remove button; an "add tag" input (Enter commits); a notes `<textarea>`.
  - Tags save **immediately** on add/remove (one PATCH carrying the full replacement list — the API semantics are "replace"). Notes save **debounced** (400 ms after the last keystroke, and on blur) with one PATCH carrying `notes` only.
  - **Optimistic with rollback, saves serialized per field:** the chip list / notes update at once. Each field keeps ONE save in flight; edits made meanwhile coalesce into a single pending value that is sent when the in-flight save settles, so neither the client nor the server can apply two REPLACE saves out of order. On a failure of the latest save the previous value is restored and an inline error line shows the message; a failed notes save rolls back only if the text is unchanged since it was sent, otherwise the typed text is kept (still dirty). Every acknowledged save reports the exchange id it was sent for, so a save that completes after the user switched rows still updates the old row and never lands on the new one; a pending notes save is flushed to its own exchange when the editor unmounts.
  - **Validation (client, mirroring the store's limits — the plan confirms them; if the store has none these are the spec's choice):** a tag is trimmed, non-empty, ≤ 64 chars, no control characters, case-preserved, deduplicated case-sensitively; at most 32 tags per exchange. Rejected input shows a chip-style message, never a throw.
- **History:** the existing Tags column keeps rendering the row's tags as text chips (the row DTO already carries `tags`). The history store gains `patchRow(id, { tags?, notes? })`, which updates the matching in-memory row only; `ExchangeDetail` calls it under the saved exchange's id after each acknowledged PATCH, so the list reflects the change without a refetch (also for a row the user has since left). A refetch (filter/paging) remains the source of truth. A row context action **"Tag…"** focuses the detail's add-tag input (keyboard-reachable; no new keymap action).
- **Rendering:** tags and notes are rendered only through Svelte `{…}` text interpolation — never `{@html}`, never as attributes that execute. The XSS corpus spec gains a step that *types* a corpus payload into the tag input and the notes field and asserts (with the live probe) that nothing executes and the stored value round-trips byte-for-byte.

## 4. Decoder pane (§7.6)

- **Pure module** `ui/src/lib/decode/codecs.ts` — bytes in, bytes out, so steps chain:
  - `type Codec = 'url' | 'base64' | 'hex' | 'html'`; `type Direction = 'encode' | 'decode'`.
  - `apply(codec, dir, input: Uint8Array): { bytes: Uint8Array; warning: string | null }` — **never throws** on hostile input; a lossy or malformed decode returns the best-effort bytes plus a warning (e.g. `base64: 3 invalid characters skipped`, `hex: odd length, last nibble dropped`, `url: 2 malformed % sequences kept literally`, `html: 1 unknown entity kept literally`).
  - `base64` decode accepts standard and URL-safe alphabets, padding optional; encode emits standard with padding. `hex` decode accepts upper/lower case, whitespace and a `0x` prefix. `url` decodes `%XX` only (`+` is **not** a space — it's a byte). `html` decodes numeric `&#NNN;` / `&#xHH;` and the core named set (`amp lt gt quot apos nbsp`); encode escapes `& < > " '`.
  - `jwtView(input: Uint8Array): JwtView | null` — splits on `.`, base64url-decodes header and payload, pretty-prints each when it parses as JSON (`JSON.stringify(…, null, 2)` — text), shows the signature as its raw base64url string. Returns `null` when the input is not three `.`-separated segments. **It never verifies and never claims validity**: the view always carries the banner "signature not verified"; `alg: none` or a missing `alg` adds a warning.
- **Pane** `ui/src/lib/components/DecoderPane.svelte` replaces the inline popover code in `Shell.svelte` (Shell keeps the single `decode-selection` registration and now opens the pane). Layout: an **input** box, an ordered list of **steps** (each a codec + direction, add/remove), each step's **output** shown below it, and a **JWT view** section that appears when `jwtView` of the current input is non-null.
  - **Input seeding:** `decode-selection` seeds the input via `selectionBytes()`: inside the CodeMirror editor it reads the document (`sliceDoc`), so the bytes are exact and independent of the rendered viewport or placeholder glyphs; outside the editor, Latin-1 text maps 1:1 and non-Latin-1 text is UTF-8-encoded with the chip. The textarea shows CR (0x0D) as the visible sentinel `␍` so a native textarea cannot normalize it away. Typed or pasted text goes through the editor's `encodePastedText` (UTF-8, with the same non-Latin-1 chip).
  - **Output rendering:** text only via `bytesToLatin1` + `{…}` interpolation, with the editor's non-printable placeholder convention for display; an optional hex rendering reuses `HexPane.svelte`. Nothing in the pane is ever interpreted as markup — the §11.1 lint passes with **no** new exemptions.
  - **Copy** button per output (clipboard write of the Latin-1 string; failure shows a chip, matching `ByteEditor.copyRaw`). Escape or the close button dismisses the pane.
- **Keymap:** no new action. `decode-selection` (`Ctrl+Shift+D`) opens the pane seeded from the selection; with an empty selection it opens with an empty input (so the pane is usable mouse-free as a scratchpad).

## 5. Security (§11.1)

- Every rendered value (tags, notes, decoder outputs, JWT fields, warnings) is text interpolation. The ESLint §11.1 rule set stays active with no new disables.
- The XSS corpus spec is extended to: the decoder pane (seed every corpus payload as input, run each codec in both directions, assert no execution and that the output equals the pure function's result), and the tag/notes inputs (typed payloads, round-trip, no execution).
- The decoder does no network access and no `eval`-class API (`Function`, `setTimeout(string)`, `innerHTML`, `DOMParser` into the live DOM). JSON pretty-printing is `JSON.parse`/`JSON.stringify` only.

## 6. Testing

- **vitest** `codecs.test.ts`: per codec × direction, round-trip over the full 0x00–0xFF byte corpus; the malformed-input cases above each produce the specified warning and never throw; base64 std/url-safe/unpadded equivalence; `jwtView` on a valid token, a two-segment string (`null`), a non-JSON payload (raw text shown, no throw), `alg: none` (warning). `exchanges.test.ts`: `patchExchange` hits `PATCH /api/v1/exchanges/{id}` with the given body. History store: the row-update method patches `tags`/`notes` of the matching row only.
- **Playwright** (API mocked, like `preview.spec.ts`): `tags-notes.spec.ts` — add a tag → one PATCH with the full list → chip appears in the detail **and** the history row; remove it → PATCH → gone; type notes → exactly one debounced PATCH; PATCH 500 → rollback + inline error; `filter-tag` then lists the tagged row. `decoder.spec.ts` — select text in the detail → `Ctrl+Shift+D` → pane opens seeded → add a base64-decode step → output correct → copy; paste a JWT → JWT view with the "not verified" banner → Escape closes.
- `mouse-free.spec.ts` gains the decode step (select → chord → pane → Escape). `xss-corpus.spec.ts` gains the decoder and tag/notes steps (§5).
- No Rust change, but the **full gate** still runs before merge (`just lint`, `just doc`, `cargo deny check`, `cargo test`, UI lint/test/e2e), and the committed UI bundle is rebuilt (`just ui-build`, `just ui-dist-check`).

## 7. Scope

**In:** the tags/notes editor + client + history column/row-update/"Tag…" action; the codec module + decoder pane + JWT view; the Shell refactor that moves the popover into the pane; the tests above; `docs/security-gates.md` XSS-gate note updated to list the new surfaces.

**Out (YAGNI / later):** auto ("smart") decode guessing; the full HTML5 named-entity table; JWT signature verification or key handling; gzip/deflate/brotli; tag autocomplete, rename-across-project or bulk-tagging the multi-selection (0.1.x); a decoder tab/route (the pane is enough for v0.1); response intercept (beta-2); match-and-replace (beta-3).

## 8. Global constraints (verbatim)

- `unsafe_code = "forbid"`; MSRV 1.88; `cargo build` Node-free; `cargo deny` clean.
- **No new external dependency** (npm or cargo). Codecs hand-rolled.
- Conventional Commits, ≤72-char subjects, **no attribution trailers**.
- Additive only: no API/schema change; the PATCH route and `tag=` filter are consumed as they are.
- FULL GATE before a task is done, including the UI e2e; rebuild + `ui-dist-check` the committed bundle.
- Reuse: `apiSend`, `PatchBody`/`ExchangeDto`, `bytesToLatin1`/`latin1ToBytes`/`encodePastedText`, `HexPane`, the keymap's existing `decode-selection`, the XSS corpus fixtures and live probe.

## 9. Review focus (inputs the tests must cover)

- A tag or note containing a corpus payload is stored and shown verbatim and never executes (typed path, not just the seeded-DTO path).
- A PATCH that fails leaves the UI showing the **previous** value (rollback), not the optimistic one; a response for a previously selected exchange never lands on the current one.
- Notes typing produces **one** PATCH per pause, not one per keystroke.
- Every codec's decode on hostile input (invalid chars, odd length, truncated `%`, unknown entity, non-JSON JWT payload, `alg: none`) returns a warning and never throws or hangs; `+` in URL decode stays a byte.
- Round-trip over 0x00–0xFF is byte-exact for every codec; the selection → bytes mapping is exact (Latin-1), and non-Latin-1 typed input is UTF-8-encoded with the chip.
- `Ctrl+Shift+D` keeps working from every view (the Shell registration is unchanged in scope) and Escape closes the pane; no new keymap action, no per-view keydown handler.

## 10. Acceptance

- Tags and notes are editable in the detail view, persist through the existing PATCH route, show in the history rows, and the tag filter finds them.
- The decoder pane decodes/encodes URL, Base64, hex and HTML entities with chained steps and shows a non-verifying JWT view; outputs are text only.
- All new vitest + Playwright specs green; `mouse-free` and `xss-corpus` extended and green; the §11.1 lint passes with no new exemptions; the full CI gate green on ubuntu + macOS.
- No Rust diff beyond the regenerated UI bundle; no dependency change.
