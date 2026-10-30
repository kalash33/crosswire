# Beta-1: Tags/Notes UI + Decoder Pane Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the two §14 beta cut-list items that need no backend change — an editable tags/notes surface and a real decoder pane — rendered as text only so the §11.1 XSS gate keeps holding.

**Architecture:** Tags/notes reuse the existing `PATCH /api/v1/exchanges/{id}` route through a small client call, an editor component with optimistic update + rollback, and a new in-memory `patchRow` on the history store. The decoder is a pure bytes-in/bytes-out codec module (URL, Base64, hex, HTML entities, plus a non-verifying JWT view) behind a pane component that replaces the Shell's inline encode-only popover; the existing `decode-selection` chord opens it seeded from the selection.

**Tech Stack:** Svelte 5 (runes), TypeScript, vitest, Playwright. No Rust change beyond the regenerated committed UI bundle. No new dependency.

**Spec:** docs/superpowers/specs/2026-10-30-crosswire-beta-1-tags-notes-decoder-design.md

## Global Constraints

- `unsafe_code = "forbid"`; MSRV 1.88; `cargo build` Node-free; `cargo deny` clean.
- **No new external dependency** (npm or cargo) — codecs are hand-rolled, no `jose`.
- Conventional Commits, ≤72-char subjects, **no attribution trailers** (verify `git log -1 | grep -c -i claude` == 0 after each commit).
- Additive only: no API/schema change; the PATCH route and `tag=` filter are consumed as they are.
- §11.1: every rendered value is Svelte `{…}` text interpolation; no `{@html}`, `innerHTML`, `srcdoc`, dynamic `href`/`src`; the ESLint §11.1 rule set passes with **no new disables**. No `eval`-class API, no network in the decoder.
- FULL GATE before a task is done: `just lint`, `just doc`, `cargo deny check`, `cargo test`, `npm --prefix ui run lint|test|e2e`. **The committed bundle is rebuilt once, in Task 3** (`just ui-build` + `just ui-dist-check`); Tasks 0–2 skip `ui-dist-check` to avoid four bundle churns (CI checks only the PR head).
- Reuse: `apiSend`, `PatchBody`/`ExchangeDto` (`ui/src/lib/api`), `bytesToLatin1`/`latin1ToBytes`/`encodePastedText` (`ui/src/lib/editor/encoding.ts`), `HexPane.svelte`, the keymap's existing `decode-selection`, the XSS corpus fixtures (`ui/e2e/fixtures/xss-corpus.ts`, `xss-detect.ts`).

## Review Focus

- A failed PATCH must leave the previous tags/notes on screen (rollback), and a PATCH response for a previously selected exchange must never land on the current one — Task 0 `tags-notes.spec.ts` (500 → rollback; switch-mid-flight).
- Typing notes must produce exactly one PATCH per pause, not one per keystroke — Task 0 `tags-notes.spec.ts` (debounce count).
- Every codec decode on hostile input returns a warning and never throws or hangs; `+` in URL decode stays a literal byte — Task 1 `codecs.test.ts`.
- Round-trip over 0x00–0xFF is byte-exact per codec, and the DOM selection maps to the exact bytes via Latin-1 — Task 1 `codecs.test.ts` + Task 2 `decoder.spec.ts`.
- A corpus payload *typed* into tags, notes or the decoder input is shown verbatim and never executes — Task 3 `xss-corpus.spec.ts` (typed paths).

---

### Task 0: tags/notes — client, validation, store patch, editor, history action

**Files:**
- Create: `ui/src/lib/api/exchanges.ts`, `ui/src/lib/api/exchanges.test.ts`, `ui/src/lib/tags.ts`, `ui/src/lib/tags.test.ts`, `ui/src/lib/components/TagsNotesEditor.svelte`, `ui/e2e/tags-notes.spec.ts`
- Modify: `ui/src/lib/stores/history.svelte.ts` (add `patchRow`), `ui/src/lib/stores/history.test.ts` (patchRow cases), `ui/src/lib/components/ExchangeDetail.svelte:137-141` (replace the read-only chips with the editor), `ui/src/routes/History.svelte` (a "Tag…" row action beside the existing row actions near line 111)

**Interfaces:**
- Consumes: `apiSend<T>(method, path, body)` (`api/client.ts`); `PatchBody`, `ExchangeDto` (`api/types.ts`, `stores/history.svelte.ts`); the `history` store instance and its row type (`tags: string[]`, `notes` absent on rows — add `notes?: string` to the row type only if the list DTO carries it; otherwise `patchRow` patches `tags` on rows and keeps `notes` for the detail).
- Produces:
  - `patchExchange(id: string, body: PatchBody): Promise<ExchangeDto>` — `apiSend('PATCH', \`/api/v1/exchanges/${encodeURIComponent(id)}\`, body)`.
  - `tags.ts`: `MAX_TAG_LEN = 64`, `MAX_TAGS = 32`, `addTag(tags: readonly string[], raw: string): { ok: true; tags: string[] } | { ok: false; error: string }` — trims; rejects empty (`"tag is empty"`), > 64 chars (`"tag is longer than 64 characters"`), control chars `/[\u0000-\u001f\u007f]/` (`"tag contains control characters"`), duplicates case-sensitively (`"tag already present"`), and a 33rd tag (`"at most 32 tags"`). **Before fixing these limits, read `crates/crosswire-store/src/control.rs:290-320` (`exchange_set_tags`/`exchange_set_notes`): if the store enforces its own bounds, mirror them exactly and note it in the report.**
  - `history.patchRow(id: string, patch: { tags?: string[]; notes?: string }): void` — updates only the matching in-memory row; a miss is a no-op.
  - `TagsNotesEditor.svelte` props `{ id: string; tags: string[]; notes: string; onSaved: (ex: ExchangeDto) => void }`; test ids `tag-chip`, `tag-remove`, `tag-input`, `tag-error`, `notes-input`, `tags-notes-error`. Tags PATCH immediately on add/remove with the **full replacement list**; notes PATCH debounced **400 ms** after the last keystroke and on blur, carrying `notes` only. Optimistic update; on error restore the previous value and show the message in `tags-notes-error`. A response is discarded when `id` changed while in flight.
  - History "Tag…" action (`data-testid="row-tag"`): selects the focused/selected row (as the existing host-to-scope action does) and focuses `[data-testid="tag-input"]` on the next tick. No new keymap action.

- [ ] **Step 1: Write the failing unit tests.** `exchanges.test.ts`: `patchExchange_sends_PATCH_with_body` (fetch mocked; asserts method `PATCH`, path `/api/v1/exchanges/<id>`, JSON body equals the given `PatchBody`). `tags.test.ts`: one case per rejection rule above (each asserting `ok === false` and the exact error string) plus `addTag_trims_and_appends` (`addTag(['b'], ' a ')` → `{ ok: true, tags: ['b', 'a'] }`) and `addTag_is_case_sensitive` (`addTag(['A'], 'a')` → `ok === true`, two tags). `history.test.ts`: `patchRow_updates_only_matching_row`, `patchRow_unknown_id_is_noop`.
- [ ] **Step 2: Run** `npm --prefix ui run test` → the new tests FAIL (module/method not found).
- [ ] **Step 3: Implement** `exchanges.ts`, `tags.ts`, `history.patchRow`.
- [ ] **Step 4: Run** `npm --prefix ui run test` → green.
- [ ] **Step 5: Write the failing e2e** `tags-notes.spec.ts` (vite-preview, API mocked like `preview.spec.ts`; the mocked PATCH echoes the merged exchange): `add_tag_patches_full_list_and_updates_row_and_detail` (one PATCH, body `{tags:[…existing,'new']}`, chip in detail **and** the row's Tags cell); `remove_tag_patches_and_removes_chip`; `notes_typing_sends_one_debounced_patch` (type 8 chars quickly → wait 600 ms → exactly 1 PATCH with `{notes}`); `patch_500_rolls_back_and_shows_error` (previous chips restored, `tags-notes-error` visible); `switching_exchange_mid_flight_discards_response` (delay the PATCH, select another row, resolve → the new detail is untouched); `row_tag_action_focuses_tag_input`; `filter_tag_lists_tagged_row` (reuse `filter-tag`).
- [ ] **Step 6: Implement** `TagsNotesEditor.svelte`, mount it in `ExchangeDetail` (replacing lines 137-141; `onSaved` sets `exchange = ex` and calls `history.patchRow(ex.id, { tags: ex.tags, notes: ex.notes })`), add the History "Tag…" action.
- [ ] **Step 7: Run** `npm --prefix ui run lint`, `npm --prefix ui run test`, `npm --prefix ui run e2e` (+ `--repeat-each=3` on the new spec) → green; the §11.1 lint has **no** new disables.
- [ ] **Step 8: FULL GATE** (minus `ui-dist-check`, per Global Constraints).
- [ ] **Step 9: Commit** `feat(ui): editable tags and notes on the exchange detail`.

---

### Task 1: codec module (URL, Base64, hex, HTML entities, JWT view)

**Files:**
- Create: `ui/src/lib/decode/codecs.ts`, `ui/src/lib/decode/codecs.test.ts`

**Interfaces:**
- Consumes: nothing from this plan (pure module; `TextEncoder`/`TextDecoder` are fine for the JWT JSON text only — bytes elsewhere).
- Produces:
  - `type Codec = 'url' | 'base64' | 'hex' | 'html'`; `const CODECS: readonly Codec[]`; `type Direction = 'encode' | 'decode'`.
  - `interface CodecResult { bytes: Uint8Array; warning: string | null }`.
  - `apply(codec: Codec, dir: Direction, input: Uint8Array): CodecResult` — never throws. Warnings (exact text, `N` = count): `base64: N invalid characters skipped`; `hex: odd length, last nibble dropped`; `hex: N invalid characters skipped`; `url: N malformed % sequences kept literally`; `html: N unknown entities kept literally`.
  - Rules: `base64` decode accepts standard + URL-safe alphabets, padding optional, whitespace skipped silently; encode emits standard with padding. `hex` decode accepts upper/lower case, whitespace, and a leading `0x`. `url` decodes `%XX` only — `+` is a literal byte; encode percent-encodes every byte outside `A-Za-z0-9-._~`. `html` decode handles `&#NNN;`, `&#xHH;` and the named set `amp lt gt quot apos nbsp` (nbsp → 0xC2 0xA0 as UTF-8); encode escapes `& < > " '` to `&amp; &lt; &gt; &quot; &#39;`.
  - `interface JwtView { header: string; payload: string; signature: string; warnings: string[] }`; `jwtView(input: Uint8Array): JwtView | null` — `null` unless exactly three `.`-separated segments; header/payload base64url-decoded and pretty-printed with `JSON.stringify(JSON.parse(x), null, 2)` when they parse, else the raw decoded text; `signature` is the raw third segment; `warnings` always starts with `"signature not verified"`; adds `"alg is none"` when `header.alg === 'none'` and `"alg missing"` when absent.

- [ ] **Step 1: Write the failing tests** `codecs.test.ts`: for each codec, `roundtrip_all_bytes` (encode then decode of the 256-byte 0x00–0xFF array equals the input, `warning === null`); `base64_accepts_urlsafe_and_unpadded` (`'-_'`-alphabet and no `=` decode equal to the padded standard form); `base64_invalid_chars_warn` (`'ab!c'` → warning `base64: 1 invalid characters skipped`); `hex_odd_length_warns` (`'abc'` → `[0xab]`, warning `hex: odd length, last nibble dropped`); `hex_accepts_0x_and_whitespace`; `url_plus_is_literal` (`'a+b'` → bytes of `a+b`); `url_malformed_percent_kept` (`'%zz%4'` → literal, warning `url: 2 malformed % sequences kept literally`); `html_numeric_and_named` (`'&lt;&#65;&#x42;&nbsp;'` → `<AB` + C2 A0); `html_unknown_entity_kept` (`'&bogus;'` → literal, warning `html: 1 unknown entities kept literally`); `html_encode_escapes_five`; `jwt_view_valid` (a hand-built HS256 token: header/payload pretty-printed, signature = third segment, `warnings` = `['signature not verified']`); `jwt_view_two_segments_null`; `jwt_view_non_json_payload_raw`; `jwt_view_alg_none_warns`; `apply_never_throws_on_random_bytes` (fuzz 200 random inputs × every codec × direction — no throw).
- [ ] **Step 2: Run** `npm --prefix ui run test -- codecs` → FAIL.
- [ ] **Step 3: Implement** `codecs.ts`.
- [ ] **Step 4: Run** `npm --prefix ui run test` → green; `npm --prefix ui run lint` clean.
- [ ] **Step 5: FULL GATE** (minus `ui-dist-check`).
- [ ] **Step 6: Commit** `feat(ui): bytes-in/bytes-out codec module with JWT view`.

---

### Task 2: DecoderPane + Shell refactor + decode-selection e2e

**Files:**
- Create: `ui/src/lib/components/DecoderPane.svelte`, `ui/e2e/decoder.spec.ts`
- Modify: `ui/src/lib/components/Shell.svelte` (remove the `Decoded` interface, `encodeSelection`, and the `decode-popover` markup/styles at lines 10-33, 48-51, 83-100, 140-182; mount the pane), `ui/e2e/mouse-free.spec.ts` (add the decode step), any existing test referencing `decode-popover|decode-base64|decode-url|decode-hex` (grep `ui/` and update to the pane's test ids)

**Interfaces:**
- Consumes: `apply`, `CODECS`, `jwtView` (Task 1); `bytesToLatin1`, `latin1ToBytes`, `encodePastedText` (`editor/encoding.ts`); `HexPane` (`bytes?: Uint8Array`); `keymap` (existing `decode-selection` registration in Shell).
- Produces:
  - `DecoderPane.svelte` props `{ initial: Uint8Array; onClose: () => void }`. State: `input: Uint8Array`, `steps: { codec: Codec; dir: Direction }[]`. Derived: `outputs[i] = apply(steps[i], outputs[i-1] ?? input)`, `jwt = jwtView(input)`. Test ids: `decoder-pane` (role `dialog`), `decoder-input` (textarea; typed/pasted text → `encodePastedText`, non-Latin-1 shows `decoder-input-chip`), `decoder-step-add`, `decoder-step-{i}-codec` (select), `decoder-step-{i}-dir` (select), `decoder-step-{i}-remove`, `decoder-output-{i}` (text via `bytesToLatin1`), `decoder-warning-{i}`, `decoder-copy-{i}`, `decoder-hex-{i}` (HexPane, toggled), `decoder-jwt`, `decoder-jwt-banner` (first warning), `decoder-close`. Escape on the pane root (the dialog's own dismiss, **not** a keymap chord — the keymap owns app chords only) calls `onClose`.
  - Shell: `let decoderInput = $state<Uint8Array | null>(null)`; the `decode-selection` handler sets `decoderInput = latin1ToBytes(window.getSelection()?.toString() ?? '')` (empty selection → empty array → the pane opens empty); render `{#if decoderInput}<DecoderPane initial={decoderInput} onClose={() => (decoderInput = null)} />{/if}`.

- [ ] **Step 1: Write the failing e2e** `decoder.spec.ts` (mocked API like `preview.spec.ts`, seed one exchange whose response body is `SGVsbG8gd29ybGQ=`): `chord_opens_pane_seeded_from_selection` (select the body text in the detail → `Control+Shift+D` → `decoder-pane` visible, `decoder-input` value equals the selection); `base64_decode_step_outputs_text` (add step base64/decode → `decoder-output-0` text `Hello world`); `chained_steps` (add url/encode after → `decoder-output-1` = `Hello%20world`); `warning_shown_on_malformed` (type `abc` with hex/decode → `decoder-warning-0` text `hex: odd length, last nibble dropped`); `jwt_view_appears_with_banner` (paste a token → `decoder-jwt` visible, `decoder-jwt-banner` text `signature not verified`); `copy_writes_clipboard` (grant clipboard permission; `decoder-copy-0` → `navigator.clipboard.readText()` equals the output); `escape_closes_pane`; `empty_selection_opens_empty_pane`. `mouse-free.spec.ts`: add a step after the existing flow — select text, `Control+Shift+D`, assert `decoder-pane`, `Escape`, assert gone.
- [ ] **Step 2: Run** `npm --prefix ui run e2e -- decoder` → FAIL.
- [ ] **Step 3: Implement** `DecoderPane.svelte`, refactor `Shell.svelte`, update any test still pointing at the old popover ids.
- [ ] **Step 4: Run** `npm --prefix ui run lint`, `test`, `e2e` (+ `--repeat-each=3` on `decoder` and `mouse-free`) → green; **no** new §11.1 disables.
- [ ] **Step 5: FULL GATE** (minus `ui-dist-check`).
- [ ] **Step 6: Commit** `feat(ui): decoder pane with chained codecs and JWT view`.

---

### Task 3: XSS corpus over the new surfaces, gate note, bundle

**Files:**
- Modify: `ui/e2e/xss-corpus.spec.ts` (two new tests), `docs/security-gates.md` (the XSS/§11.1 gate row lists the new surfaces), `crates/crosswire-server/assets/ui/**` (regenerated bundle)

**Interfaces:**
- Consumes: `TagsNotesEditor` test ids (Task 0), `DecoderPane` test ids (Task 2), `apply`/`CODECS` (Task 1, imported into the spec for the expected values), `XSS_CORPUS`, `watch`/`liveProbe` (`fixtures/xss-detect.ts`).

- [ ] **Step 1: Write the failing tests** in `xss-corpus.spec.ts`: `typed_tag_and_notes_payloads_render_as_text` (for each corpus payload: type it into `tag-input` + Enter and into `notes-input`; the mocked PATCH echoes it back; assert the chip's `textContent` and the notes value equal the payload byte-for-byte, the row's Tags cell shows it, and `liveProbe` reports no execution); `decoder_outputs_are_text_for_every_payload` (open the pane, for each payload set `decoder-input`, for each `CODECS × Direction` add the step and assert `decoder-output-0` `textContent` equals `bytesToLatin1(apply(codec, dir, encodePastedText(payload).bytes).bytes)` and `liveProbe` is clean).
- [ ] **Step 2: Run** `npm --prefix ui run e2e -- xss-corpus` → the new tests FAIL only if a surface misrenders (they may pass immediately — that is the point; confirm they *exercise* the surfaces by temporarily asserting a wrong expected value, see it fail, restore).
- [ ] **Step 3: Update** `docs/security-gates.md`: the §11.1/XSS gate row cites the two new tests and lists tags, notes, decoder pane and JWT view as covered surfaces.
- [ ] **Step 4: Rebuild the bundle:** `just ui-build`, then `just ui-dist-check` → clean.
- [ ] **Step 5: FULL GATE** — all of it, including `ui-dist-check` and `--repeat-each=3` on the four new/extended specs.
- [ ] **Step 6: Commit** `test(ui): XSS corpus covers tags, notes and the decoder pane`.

---

## Notes for the executor

- Order 0 → 1 → 2 → 3. Task 0 is independent of 1–2; Task 2 consumes Task 1; Task 3 consumes all three. Keep the order (one implementer at a time).
- Suggested models: all four tasks are UI work from a complete spec → sonnet implementers; opus for the per-task reviews of Task 2 (the Shell refactor touches the single keymap dispatch) and Task 3 (the gate), sonnet/opus for the others by diff size; opus for the whole-branch review.
- Bundle churn: only Task 3 rebuilds `crates/crosswire-server/assets/ui/`. If a reviewer runs `just ui-dist-check` on Tasks 0–2 it will report drift — expected and documented above.
- If the store enforces tag bounds different from `64 / 32`, Task 0 mirrors the store and records the values in its report; the spec's numbers are the fallback.
