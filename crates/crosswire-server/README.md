# crosswire-server

The `crosswire` binary: it runs the MITM proxy from
[`crosswire-proxy`](https://github.com/kalash33/crosswire/blob/main/crates/crosswire-proxy/README.md)
against a project directory from
[`crosswire-store`](https://github.com/kalash33/crosswire/blob/main/crates/crosswire-store/README.md),
manages the interception CA, and serves the loopback REST + WebSocket API the
web UI and scripts drive. Part of
[Crosswire](https://github.com/kalash33/crosswire/blob/main/README.md), an open-source Rust web security testing platform.

With this binary the project is a sidecar: `crosswire serve` runs the proxy and
the API side by side, `crosswire browser` opens a browser wired to the proxy,
and `crosswire token` mints the scoped tokens scripts authenticate with.

## What it does and does not do

- `crosswire serve` opens (or creates) a project, loads (or generates) the CA,
  binds the proxy listener **and** a separate loopback API listener (REST +
  WebSocket, repeater, intercept and scope), mints the one-time bootstrap token,
  records into the store, audits its configuration, and stops cleanly on Ctrl-C
  or SIGTERM.
- `crosswire token create|list|revoke|rotate` manages a project's API tokens.
- `crosswire browser` launches a Chromium-based browser configured to use the
  proxy, with a dedicated profile under the project; `crosswire browser
  reset-profile` deletes it.
- `crosswire ca export`, `ca spki` and `ca path` give you the CA certificate,
  its Chromium pin, and where it lives.
- Does **not** install the CA into any operating system, browser or keychain; the
  CA is a plain file store. See
  [docs/ca-install.md](https://github.com/kalash33/crosswire/blob/main/docs/ca-install.md).
- Does **not** switch hosts to passthrough on its own. `--passthrough` is manual.
- Does **not** phone home: the launcher opens only the proxy's own CA page or the
  URL you ask for.
- Unix only for now, because `crosswire-store` is.

## Commands

```
crosswire [--log LEVEL] <COMMAND>

  serve    Run the intercepting proxy and the API, recording into a project
  token    Mint, list, revoke or rotate a project's API tokens
  browser  Launch a browser pre-configured to use the proxy
  ca       Inspect or export the interception CA
```

`--log` is global and defaults to `info`. `RUST_LOG`, when set and valid, wins.
Logs go to **stderr**; stdout carries command output only.

### `crosswire serve`

| flag | default | meaning |
| --- | --- | --- |
| `--project PATH` | required | project directory, created if missing |
| `--proxy-bind ADDR` | `127.0.0.1:8080` | listen address; port 0 picks a free port |
| `--allow-non-loopback` | off | permit a non-loopback bind (needs `--proxy-auth`) |
| `--proxy-auth USER:PASS` | none | require `Proxy-Authorization: Basic`; split at the first `:`, both parts non-empty |
| `--passthrough HOST` | none | tunnel this host, or `.suffix`, without interception; repeatable |
| `--allow HOST:PORT` | none | let the proxy reach a destination the gate refuses (loopback, link-local, cloud metadata); repeatable; `[v6]:port` for IPv6 |
| `--accept-invalid-cert HOST` | none | accept an invalid upstream certificate from this host; repeatable; audited |
| `--ca-dir PATH` | the platform config dir (`crosswire ca path`) | where `ca.crt` and `ca.key` live |
| `--capture-cap BYTES` | 52428800 (50 MiB) | bytes of each body kept; the rest is relayed only |
| `--api-bind ADDR` | `127.0.0.1:8081` | the API listener (a **separate** socket from the proxy); a non-loopback value warns and binds anyway |
| `--bootstrap-file PATH` | none | write the one-time bootstrap URL to a 0600 file instead of printing it on stdout |
| `--enforce-scope-strictly` | off | refuse out-of-scope repeater sends outright instead of recording them with an out-of-scope audit row |

The proxy's other limits keep their defaults; they are in the
[`crosswire-proxy` README](https://github.com/kalash33/crosswire/blob/main/crates/crosswire-proxy/README.md).

Startup runs in this order, so a bad configuration creates nothing:

1. Build the proxy configuration and validate it (a refused non-loopback bind, a
   bad passthrough entry). Nothing exists on disk yet.
2. Open or create the project and open the store. If the previous run did not
   close cleanly, a warning is logged; the store has already written the gap
   marker.
3. Load the CA, or generate one when neither `ca.crt` nor `ca.key` exists
   (audit `ca.generate`).
4. Load the persisted scope from `settings` (empty if none); strict mode comes
   from `--enforce-scope-strictly`. Request intercept always starts **disabled**.
5. Bind the proxy listener, then build the destination gate so it refuses the
   proxy's own port even when `--proxy-bind` asked for port 0.
6. Audit each `--allow` (`allow.add`) and each `--accept-invalid-cert`
   (`cert.override.config`).
7. Start the proxy (its handler is the shared intercept service), audit
   `listener.bind`, and record the proxy address in `settings.proxy_bind` so
   `crosswire browser` can find it.
8. Bind the API listener, build its shared state over the same store, scope,
   intercept and a repeater, and mint the single-use bootstrap token.
9. Print four lines on stdout (below), then serve the proxy and the API
   concurrently under one shutdown signal until a signal arrives.
10. On shutdown, log the counters, audit `listener.stop`, and close the store.
    `Store::close` runs on every path once the store is open, so the project is
    marked cleanly closed even after an error.

The audit log is `audit.jsonl` in the project, actor `cli`, result `ok`. The
`serve` actions are `ca.generate` (only when the CA was created), `allow.add`,
`cert.override.config`, `listener.bind` and `listener.stop`; the API layer adds
`session.bootstrap`, `token.create`, `token.revoke`, scope, intercept and
repeater rows at runtime, and the repeater's `repeater.send` /
`repeater.send.out_of_scope` rows flow through `serve`'s audit hook. `ca export`
and `ca spki` write no audit rows: there is no project to write them to.

### The API and the bootstrap token

The API is a **loopback** REST + WebSocket server on its own socket
(`--api-bind`, default `127.0.0.1:8081`). It serves only the `Host`s
`localhost[:port]` and `127.0.0.1[:port]`; a cookie-authenticated request must
also carry `Sec-Fetch-Site: same-origin` (or a matching `Origin`). Point the web
UI at `localhost`/`127.0.0.1`, not `[::1]`.

On startup `serve` mints one short-lived (60 s), single-use bootstrap token and
delivers it out of band — never in the logs, and on stdout exactly once:

```text
open http://localhost:8081/#bootstrap=<token> within 60 s
```

Opening that URL lets the UI exchange the token (via `POST /api/v1/session`) for
an `HttpOnly` session cookie. The token lives only in the URL fragment, which
browsers never send to a server. With `--bootstrap-file PATH` the URL is written
to a 0600 file instead, and stdout names only the path:

```text
bootstrap url written to /path/to/file (valid for 60 s)
```

### `crosswire token`

Tokens are the `Authorization: Bearer` credentials scripts use (the cookie is
for the browser UI). Each command opens the project, so it can run only while
`crosswire serve` is **not** holding it (the project lock is exclusive).

| command | does |
| --- | --- |
| `token create --project DIR --name NAME --scopes read,send` | mints a token; prints its secret **once** (audit `token.create`) |
| `token list --project DIR` | lists metadata only — never the secret |
| `token revoke --project DIR ID` | revokes a token (audit `token.revoke`) |
| `token rotate --project DIR ID` | revokes `ID` and mints a replacement with the same name and scopes, printing what it invalidated (audit `token.revoke` + `token.create`) |

Scopes are `read`, `send` and `admin`. The secret is `cwt_` followed by base64url
and is shown once: only its SHA-256 is stored.

### `crosswire browser`

Launches a Chromium-based browser (Chrome → Chromium → Edge → Brave, per OS, or
`--browser-path`) wired to the proxy, with a dedicated 0700 profile at
`<project>/browser-profile` (excluded from exports, see
[docs/format.md](https://github.com/kalash33/crosswire/blob/main/docs/format.md)).

| flag | default | meaning |
| --- | --- | --- |
| `--project PATH` | required | project directory (holds the profile) |
| `--proxy ADDR` | `settings.proxy_bind` | proxy to point the browser at; pass this while `serve` is running (the recorded address is readable only when the project is not locked) |
| `--browser-path PATH` | auto-detect | a specific browser binary |
| `--headless` | off | launch headless (`--headless=new`) |
| `--url URL` | none | open this URL on launch |
| `--ca-dir PATH` | `crosswire ca path` | CA directory, for the certificate pin |
| `-- FLAG...` | none | extra flags passed straight to the browser |

The browser trusts the proxy via `--ignore-certificate-errors-spki-list`
(the CA's pin), so no system trust store is touched. The fixed flags are
`--proxy-server`, `--proxy-bypass-list=<-loopback>`, `--user-data-dir`,
`--ignore-certificate-errors-spki-list`, `--no-first-run` and
`--no-default-browser-check`. When no browser is found the launcher prints the
proxy address and `http://crosswire/` and exits **3**.

`crosswire browser reset-profile --project DIR` deletes the profile directory
and audits `browser.profile_reset` (so it, too, needs the project unlocked).

### `crosswire ca`

| command | does |
| --- | --- |
| `ca export --format der\|pem [--out PATH] [--ca-dir PATH]` | writes the CA certificate to the file, or to stdout; generates the CA when there is none |
| `ca spki [--ca-dir PATH]` | prints the base64 SHA-256 of the CA's SubjectPublicKeyInfo and a newline, for Chromium's `--ignore-certificate-errors-spki-list` |
| `ca path` | prints the default CA directory, `<config dir>/crosswire/ca` (`~/.config/crosswire/ca` on Linux, `~/Library/Application Support/crosswire/ca` on macOS) |

### Exit codes

| code | meaning |
| --- | --- |
| 0 | a clean stop on Ctrl-C or SIGTERM, or a command that succeeded |
| 2 | bad usage or configuration: a clap error, a `--ca-dir` inside `--project`, a non-loopback bind without `--allow-non-loopback` or without `--proxy-auth`, a bad `--passthrough`, `--accept-invalid-cert`, `--allow`, `--proxy-auth`, `--scopes` or `--log`, a bad token id, or no config directory |
| 1 | a runtime failure: opening the project or store (for example a token/browser command on a project `serve` is holding), CA I/O, binding, writing an audit row or stdout, closing the store |
| 3 | `crosswire browser`: no supported browser was found (the manual fallback was printed) |

Errors are printed to stderr as `crosswire: error: ...`.

## Example

```text
$ crosswire serve --project ./demo
proxy listening on 127.0.0.1:8080
ca page at http://crosswire/ (via the proxy) or http://127.0.0.1:8080/
api listening on 127.0.0.1:8081
open http://localhost:8081/#bootstrap=<token> within 60 s
```

Logs, on stderr (each line also carries a timestamp, omitted here):

```text
INFO crosswire::serve: proxy and api listening addr=127.0.0.1:8080 api_addr=127.0.0.1:8081
```

From another terminal, open the browser at the proxy, or mint a token for a
script:

```text
$ crosswire browser --project ./demo --proxy 127.0.0.1:8080
$ crosswire token create --project ./demo --name ci --scopes read,send
id: 01J...
name: ci
scopes: read,send
secret: cwt_...
(the secret is shown once and cannot be recovered)
$ curl -s -x http://127.0.0.1:8080 http://crosswire/ca.spki
sFSsabiQ82cQbtbCpDqRxuUtAp3pbbJe6OMDbuWb/do=
```

(`crosswire token` needs the project unlocked, so run it before `serve` or after
stopping it.)

Ctrl-C stops it (the counts depend on what you browsed):

```text
^C
INFO crosswire::serve: shutting down
INFO crosswire::serve: proxy stopped accepted=5 exchanges=0 tunnels=0
$ echo $?
0
```

The project then holds `project.db`, `packs/`, `journal/`, `project.lock`,
`audit.jsonl` and, once a browser is launched, `browser-profile/` (excluded from
exports); the format is in
[docs/format.md](https://github.com/kalash33/crosswire/blob/main/docs/format.md).

## Security notes

- The proxy binds loopback by default. A non-loopback bind needs both
  `--allow-non-loopback` and `--proxy-auth`; the credentials are visible in the
  process arguments, so use this on a host you control.
- `--allow` and `--accept-invalid-cert` loosen the destination gate and upstream
  certificate checks for named hosts only, and each is written to the audit log.
- `serve` refuses a `--ca-dir` inside `--project` (both resolved, symlinks
  and `..` included), so the key never ends up in a project that is shared.
- The CA private key (`ca.key`, 0600) is the most sensitive file Crosswire
  creates: anyone holding it can mint certificates your browser trusts. Keep it
  out of project directories and backups you share, and remove the CA from any
  browser or system store when you finish testing.
- A project records credentials, cookies and tokens exactly as they crossed the
  proxy. Treat it as sensitive data; redaction applies when you export.

## Status

Pre-alpha, `publish = false`, version `0.0.0` in `Cargo.toml`; this is the
binary of the `0.1.0-alpha.2` loopback sidecar (proxy + API). The command line is
not frozen.

Design: [spec section 12](https://github.com/kalash33/crosswire/blob/main/docs/superpowers/specs/2026-10-22-crosswire-design.md)
(configuration and `crosswire serve`) and
[ADR-0003](https://github.com/kalash33/crosswire/blob/main/docs/adr/0003-proxy-relay.md).
