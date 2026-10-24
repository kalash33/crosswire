# ADR-0004: API credentials, bootstrap sessions and same-site enforcement

Status: Accepted

## Context

`crosswire serve` now runs a REST and WebSocket API (`crosswire-api`, spec
sections 8 and 11.2) beside the proxy. That API can read every recorded
exchange, send requests from the tester's machine, hold and edit live traffic,
and mint credentials. Anything that can call it can do what the tester can do.

The API listens on loopback, and two kinds of caller reach it:

- **The web UI**, a page in the tester's own browser. That browser is also the
  one that visits hostile sites while the proxy is recording. A hostile page can
  make the browser send requests to `127.0.0.1:8081`, to `localhost` on a
  different port, or to a DNS name that has been rebound to `127.0.0.1`.
- **Scripts and CI jobs**, which have no browser, may run long after the
  sidecar was started, and need a credential that survives a restart.

Loopback is a network location, not an identity. Other local users, other local
processes and every page in the browser can reach it. This ADR records how the
API decides who is calling, what it rejects before it looks at a credential, and
what it chose not to build. The properties described here are the ones the
tests in [security-gates.md](../security-gates.md) prove.

## Decision

1. **Two sockets.** The proxy (default `127.0.0.1:8080`) and the API (default
   `127.0.0.1:8081`) are separate listeners. The proxy serves only the
   `http://crosswire/` info page and relays; the API serves only `/api/v1/`.
   A browser pointed at the proxy cannot reach the API through it, and the API
   cannot be used to proxy. The API's port is part of its cookie name, so two
   sidecars on one machine do not share a session.
2. **Host check first.** Before any route or credential logic, the API accepts
   only `Host: localhost[:port]` or `127.0.0.1[:port]` and answers `421`
   otherwise. A page on a rebound DNS name sends its own name in `Host`, so it
   fails here. The same check covers the WebSocket upgrade.
3. **Bootstrap, then a cookie.** On every start `serve` mints one bootstrap
   token (256 bits, single use, 60 second lifetime). It is delivered out of
   band: printed once on stdout inside a URL fragment
   (`http://localhost:<port>/#bootstrap=<token>`), or written to a mode 0600
   file with `--bootstrap-file`. It is never logged and never on a command line.
   `POST /api/v1/session` exchanges it for the session cookie
   `cw_sid_<port>` (`HttpOnly`, `SameSite=Strict`, `Path=/api`). The cookie is
   a random 256-bit id with a 30 minute idle expiry that every authenticated
   request refreshes. The fragment is never sent to a server, so the token does
   not appear in the API's own request line or in any `Referer`.
4. **Same-site enforcement on cookie requests.** A request authenticated by
   cookie, including the WebSocket upgrade, must carry
   `Sec-Fetch-Site: same-origin` or an `Origin` exactly equal to
   `http://localhost:<port>` or `http://127.0.0.1:<port>`. Anything else is
   `403`, before the session is looked up. Browsers set `Sec-Fetch-Site`
   themselves and a page cannot forge it. `SameSite=Strict` is the second layer,
   not the only one: `localhost:3000` and `localhost:8081` are the same *site*,
   so `SameSite` alone would let a page on another local port ride the cookie.
5. **No CORS, ever.** The API emits no `Access-Control-*` header on any
   response, including errors. A cross-origin page can still send a request, but
   the browser will not let it read the answer, and step 4 refuses the
   cookie-bearing ones anyway. There is no allowlist to configure and so none to
   misconfigure.
6. **Scoped tokens for scripts.** A token has one or more of `read`, `send`,
   `admin`, each including the ones before it. Its secret is `cwt_` followed by
   32 random bytes in base64url, shown once at creation. The project stores only
   the SHA-256 of the 32 raw bytes, with the scopes, a name and timestamps, in
   the `tokens` table. The API hashes what a caller presents and looks the hash
   up, so the secret is never compared as plaintext; a revoked token is `401`.
   Tokens are created, listed, revoked and rotated with `crosswire token` or the
   API (`admin` scope). Bearer requests are not subject to step 4, because a
   page cannot attach an `Authorization` header to a request the browser makes
   on its own. A session cookie has full (`admin`) power; a token has exactly the
   scopes it was minted with. Each mutating route checks its required scope, and
   the `read`-token-cannot-mutate gate walks all of them.
7. **Constant-time comparison, rate limits and audit.** The bootstrap token and
   session ids are compared in constant time. Authentication failures are
   counted per peer address; ten in a minute answer `429` to that peer. Every
   failure is written to the audit log (`auth.fail`, or `session.bootstrap` with
   result `fail`), as is every success of `session.bootstrap`, `token.create`
   and `token.revoke`.
8. **The WebSocket may authenticate by first frame.** Browsers cannot set
   headers on a WebSocket, and a script may not hold a cookie. With no cookie or
   bearer header on the upgrade, the first text frame must be
   `{"bearer":"cwt_..."}` within 5 seconds, and a failure counts toward the rate
   limit like any other.
9. **Five routes need no credential:** `health`, `openapi.json`, `ca.der`,
   `ca.pem` and the session exchange. They expose nothing that is not already
   public to anyone who can reach the port (the CA certificate is the public
   half), and they obey the Host check.
10. **Sessions are memory only; tokens are on disk.** The session table lives in
    the process. Restarting `serve` ends every session, and the next run prints a
    new bootstrap token. Tokens are rows in the project database, so they
    survive a restart, a crash and a copy of the project.

## Why this shape

- **Why not trust loopback.** Every page in the tester's browser can reach
  `127.0.0.1`, and the tester's browser is the thing the proxy exists to point at
  hostile content. DNS rebinding, a malicious page on another local port and a
  local process running as another user all arrive from "localhost". The Host
  check, the Origin or Fetch-Metadata check, and a credential are three
  independent refusals; a bug in one does not open the API.
- **Why a fragment-delivered bootstrap token instead of a password the user
  types.** The UI has to start with no credential and must not ask the tester to
  copy a long secret around. The token is single use and expires, so a copy that
  leaks from a terminal scrollback after a minute is worthless, and one that is
  used twice is a visible failure (`bootstrap_invalid`, audited) rather than a
  silent second session.
- **Why hash tokens at rest.** The project directory is something a tester
  shares, backs up and attaches to a ticket. Storing only the hash means a copy
  of `project.db` does not hand out working credentials. The secret is 256 bits
  of randomness, so a plain SHA-256 is enough; a slow password hash would only
  cost time on every request.
- **Why `Sec-Fetch-Site` with an `Origin` fallback.** `Sec-Fetch-Site` is set by
  the browser and cannot be overwritten from script. Older or unusual clients
  that send only `Origin` are accepted if it matches the API's own origin
  exactly. A request with neither (a `curl` command with a cookie) is refused, so
  the cookie cannot be used by accident from a tool; scripts use tokens.
- **Why no CORS rather than a tight CORS policy.** The UI is served from the API
  origin, so it never needs a cross-origin read. A policy can only widen access.
- **Why sessions are not persisted.** A session is tied to a browser tab and to
  the bootstrap token minted for this run. Persisting them would need either a
  stored secret to protect (the thing hashing avoids for tokens) or a long-lived
  cookie that outlives the process that issued it. Losing a session costs the
  tester one reload of a freshly printed URL. Losing a script's token would cost
  a re-deployed secret in CI, so tokens persist.

## Alternatives considered

- **A password-authenticated key exchange (PAKE) between the UI and the API.**
  It would protect the credential from an observer on the wire, but the wire is
  loopback and the observer that matters (another local process) can read the
  process's memory and files anyway. It adds a protocol, a dependency and a
  browser-side implementation to maintain, for no change in the threat model.
  Rejected.
- **Implicit trust of loopback, with no credential.** This is what several local
  developer tools do, and it is the reason browser-to-localhost attacks work
  against them. Given that this API can send traffic and edit requests, it was
  rejected outright.
- **A CORS allowlist for the UI origin.** Unnecessary because the UI shares the
  API's origin, and it replaces a refusal with a policy that has to be correct.
- **The bootstrap token in a query string, or typed by the user.** A query
  string reaches the server's request line, logs and `Referer`. A typed token
  needs a place to type it before the UI is trusted. The fragment avoids both.
- **A persistent session cookie or stored session table.** See above; it trades
  a small convenience for a stored secret.
- **An OS keychain for tokens.** Deferred, not rejected: it is on the list of
  work still ahead (spec section 17). The hashed table is the baseline it would
  sit beside.

## Consequences

- **A script needs a token, and `crosswire token` needs `serve` stopped.**
  `serve` holds the project's exclusive lock, so `token create`, `list`, `revoke`
  and `rotate` (and `browser reset-profile`) open the project only while it is
  not running. While `serve` is up, mint tokens through the API with a session or
  an `admin` token. `crosswire browser` takes `--proxy` while `serve` runs for the
  same reason.
- **Local peers share one rate-limit bucket.** Every local client has the peer
  address `127.0.0.1`, so ten failures from one local process lock out the others
  for the minute. That is a denial of service against a process that is already on
  the machine, and it keeps guessing at a token costly. The limiter does not yet
  evict idle peers; that is on the pre-0.1.0 backlog.
- **The bootstrap URL is a secret for one minute.** Anyone who can read the
  terminal or the `--bootstrap-file` in that window can start a session.
  `--bootstrap-file` is mode 0600 for that reason. A session is full `admin`.
- **The API is plain HTTP on loopback.** The cookie is therefore not `Secure`.
  Binding the API to a non-loopback address prints a warning and is not recommended.
- **Intercept rules are not persisted.** Scope and strict enforcement are stored
  in the project's `settings` table and loaded when `serve` opens. The intercept
  rules type is not serialisable yet, so intercept always starts disabled with
  the default rules (pre-0.1.0 backlog).
- **Gates still ahead.** [security-gates.md](../security-gates.md) maps each
  section 13 gate to a test. Three gates cannot be proven without the UI or the
  export tooling and have no test yet: the XSS corpus (Plan 5), preview
  zero-egress (Plan 5/6) and the redaction golden for copy-as-curl and export
  (Plan 5/6). The idle-egress gate instruments the repeater's transport, the one
  dialing primitive, not the whole `serve` binary.
