# The Crosswire API

`crosswire serve` runs a REST and WebSocket API beside the proxy. It is what the
web UI (not built yet) and your scripts drive: list and read recorded
exchanges, send requests through the repeater, hold and edit requests with
intercept, manage scope, mint tokens, and follow a live event stream. This guide
shows how to call it with `curl`. The machine-readable description of every
route is [docs/api/openapi.json](api/openapi.json); the design of the
credentials is [ADR-0004](adr/0004-api-credentials.md), and the security
properties that are tested are listed in [security-gates.md](security-gates.md).

The API is **not frozen** before v0.1.0.

## Envelope and versions

Every response that passes through the API gate carries a
`Crosswire-Schema-Version` header, including the gate's own refusals (`401`,
`421`, `404`, `429`). Its value is the wire-format version of recorded Messages
and Exchanges (currently `2`). The header is not repeated per route in the
OpenAPI document. `GET /api/v1/health` reports the same number and the project
database format:

```json
{"status":"ok","schema_version":2,"format_version":6}
```

When the server runs a proxy, `health` also carries `proxy_addr`, the proxy's
actual bound `host:port` (`[v6]:port` for IPv6, and the real port when
`--proxy-bind` asked for port 0); the key is absent otherwise.

`schema_version` is the shape of the Messages and Exchanges the API returns and
accepts (enum wire codes, tagging, defaults); `format_version` is the project
database's `PRAGMA user_version`. Today `schema_version` 2 pairs with
`format_version` 6. A project written by a pre-release build (`user_version` 1
to 5) is refused on open rather than migrated; re-record the traffic. See
[format.md](format.md) for the wire and on-disk details.

## Two sockets

| Socket | Default | What it carries |
|---|---|---|
| Proxy | `127.0.0.1:8080` (`--proxy-bind`) | Your browser's traffic |
| API | `127.0.0.1:8081` (`--api-bind`) | Everything in this guide, under `/api/v1/` |

They are separate sockets. The proxy never serves the API and the API never
proxies. Both are loopback by default. The API answers only to the `Host`
values `localhost[:port]` and `127.0.0.1[:port]`; any other `Host` gets `421`.
`[::1]` is not accepted, so use `localhost` or `127.0.0.1`. `serve` warns
(on stderr) if you point `--api-bind` at a non-loopback address, then binds it
anyway. The API has no TLS, so keep it on loopback.

## Getting in: bootstrap, then a cookie or a token

There are two credentials. A **session cookie** is for a browser UI and has full
(`admin`) power. A **bearer token** is for scripts and is limited to the scopes
you give it.

### 1. The bootstrap token and the session cookie

`crosswire serve` mints one bootstrap token each time it starts: 256 bits,
single use, valid for 60 seconds. It is delivered once, never in the logs:

```text
$ crosswire serve --project ./demo
proxy listening on 127.0.0.1:8080
ca page at http://crosswire/ (via the proxy) or http://127.0.0.1:8080/
api listening on 127.0.0.1:8081
open http://localhost:8081/#bootstrap=<token> within 60 s
```

With `--bootstrap-file PATH` the same URL is written to a file (mode 0600)
instead, and stdout names only the path. Use that in scripts so the token never
lands in a terminal or a log.

Exchange it for a session cookie:

```sh
TOKEN=...   # the part after #bootstrap=
curl -si -c jar -X POST http://localhost:8081/api/v1/session \
  -H 'content-type: application/json' \
  -d "{\"bootstrap\":\"$TOKEN\"}"
```

```text
HTTP/1.1 200 OK
set-cookie: cw_sid_8081=<id>; HttpOnly; SameSite=Strict; Path=/api

{"expires_in":1800}
```

The cookie is named `cw_sid_<api port>`, so two sidecars on different ports do
not collide. The session expires after 30 minutes without a request (every
authenticated request refreshes it, and `expires_in` is in seconds). A second
exchange of the same token, or one after 60 seconds, gets
`401 bootstrap_invalid`. Sessions live in memory: restarting `serve` ends them,
and you bootstrap again from the new run's token.

A cookie-authenticated request must also prove it is same-site: send
`Sec-Fetch-Site: same-origin` (every browser does for a same-origin `fetch`) or an `Origin`
exactly equal to `http://localhost:<port>` or `http://127.0.0.1:<port>`.
Otherwise it is `403 origin_forbidden`. `curl` sends neither, so add the header
yourself:

```sh
curl -s -b jar -H 'Sec-Fetch-Site: same-origin' http://localhost:8081/api/v1/counts
```

```json
{"exchanges":0,"committed":0,"error":0,"markers":0,"bytes_inline":0,"bytes_packs":0,"history":0,"repeater":0,"intruder":0}
```

### 2. API tokens

For scripts, mint a token. Tokens work with no cookie and no same-site header,
because a bearer token is not something a web page can attach by accident.

From the command line (while `serve` is **stopped**, because the project lock is
exclusive):

```sh
crosswire token create --project ./demo --name ci --scopes read,send
```

```text
id: 01J...
name: ci
scopes: read,send
secret: cwt_...
(the secret is shown once and cannot be recovered)
```

Or over the API with a session (or an `admin` token):

```sh
curl -s -b jar -H 'Sec-Fetch-Site: same-origin' -X POST http://localhost:8081/api/v1/tokens \
  -H 'content-type: application/json' -d '{"name":"ci","scopes":["read"]}'
```

```json
{"token":{"id":"01M5...","name":"ci","scopes":["read"],"created_at":1792872126508,"revoked_at":null,"last_used_at":null},"secret":"cwt_..."}
```

The secret is `cwt_` plus 32 random bytes in base64url. Only its SHA-256 is
stored, so it cannot be shown again; if you lose it, `crosswire token rotate
--project DIR ID` revokes the old token and mints a replacement with the same
name and scopes. `crosswire token list` and `GET /api/v1/tokens` show metadata
only. `crosswire token revoke` and `DELETE /api/v1/tokens/{id}` revoke one.

Use it as a bearer token:

```sh
curl -s -H "Authorization: Bearer $SECRET" 'http://localhost:8081/api/v1/exchanges?limit=5'
```

```json
{"items":[],"next_cursor":null}
```

### Scopes

| Token scope | Grants |
|---|---|
| `read` | List and read exchanges, markers, counts, scope, intercept status, repeater drafts, the token list, and the event stream |
| `send` | `read`, plus send repeater requests and save or delete drafts, forward or drop held requests, and edit exchange tags and notes |
| `admin` | `send`, plus change scope, strict enforcement, intercept on/off and rules, and create or revoke tokens |

A call a token's scopes do not cover is `403 insufficient_scope`:

```sh
curl -s -H "Authorization: Bearer $READ_SECRET" -X PUT http://localhost:8081/api/v1/intercept/enabled \
  -H 'content-type: application/json' -d '{"enabled":true}'
```

```json
{"error":{"code":"insufficient_scope","message":"the credential does not grant the required scope"}}
```

### Unauthenticated routes

Exactly five routes need no credential: `GET /api/v1/health`,
`GET /api/v1/openapi.json`, `GET /api/v1/ca.der`, `GET /api/v1/ca.pem` and
`POST /api/v1/session` (which takes the bootstrap token instead). They still
obey the `Host` rule.

### Failures and rate limits

Authentication failures (a bad bootstrap token, a bad cookie or bearer token,
a bad first WebSocket frame) are counted per peer address. After ten in a
minute the API answers `429 rate_limited` to that peer's authenticated requests until the window
passes, and every failure writes an `auth.fail` (or `session.bootstrap`, result
`fail`) row to the project's audit log. The API never sends an
`Access-Control-*` header, so a page on another origin cannot read a response
even if it could reach the port.

Every error is JSON in one shape:

```json
{"error":{"code":"origin_forbidden","message":"a cookie-authenticated request must be same-origin"}}
```

## Reading recorded traffic

All of these need `read`.

| Route | What it does |
|---|---|
| `GET /api/v1/exchanges` | A page of rows, newest first |
| `GET /api/v1/exchanges/{id}` | One exchange with its bodies |
| `GET /api/v1/exchanges/{id}/body/{side}` | One body as raw bytes; `side` is `req` or `resp` |
| `GET /api/v1/markers` | Gap and failure markers, newest first |
| `GET /api/v1/counts` | Totals, including a split by namespace |

`GET /api/v1/exchanges` takes `limit` (1 to 1000, default 100), `after` (the
`next_cursor` of the previous page), and the filters `method`, `host` (exact),
`host_contains`, `status_min`, `status_max`, `namespace`, `state`, `scope_state`
and `tag`. Paging is a keyset cursor on the exchange id, so a page does not move
when new rows arrive:

```sh
curl -s -H "Authorization: Bearer $SECRET" \
  'http://localhost:8081/api/v1/exchanges?host_contains=example.com&status_min=400&limit=50'
```

A row carries `id`, `created_at`, `method`, `host`, `path`, `status`,
`resp_len`, `state` and `tags`. Fetch the whole exchange by id for the heads,
diagnostics, diffs and timing.

`PATCH /api/v1/exchanges/{id}` (needs `send`) replaces an exchange's `tags`
and/or `notes`.

## The event stream

`GET /api/v1/events` upgrades to a WebSocket and streams recorder events as JSON
text frames. There are three ways to authenticate:

- the session cookie (with the same-site rule above, which applies to the
  upgrade request too);
- an `Authorization: Bearer` header on the upgrade request;
- no header at all, then a first text frame `{"bearer":"cwt_..."}` sent within 5
  seconds. This is for browser clients, which cannot set headers on a WebSocket.

A bad or missing credential closes the connection (or returns `401` before the
upgrade). First-frame failures count toward the rate limit above.

Frames are tagged by `type`. Recorder events carry a strictly increasing `seq`:

```text
{"type":"received","id":"01M5...","seq":41}
{"type":"committed","ids":["01M5..."],"seq":42}
{"type":"marker","marker":{...},"seq":43}
{"type":"lagged","missed":3,"seq":47}
{"type":"intercept","held":2}
```

`received` means an exchange entered the write queue; `committed` means it is
durable. A `lagged` frame tells a slow reader that events were dropped. The
other recorder event types are `error` and `limit-exceeded`. The `intercept` frame
(no `seq`) reports how many requests are held right now.

`?after=<seq>` asks to resume from a sequence number, but the stream is a live
broadcast with no replay buffer. The server answers it with one
`{"type":"lagged","missed":0,"seq":<after>}` frame to say that history before
now was not replayed, then streams live events. After a `lagged` frame, or after
reconnecting, re-fetch with `GET /api/v1/exchanges` to catch up.

## Scope

Scope decides which hosts are fair game (spec section 4.6). It is stored in the
project, so it survives a restart.

```sh
# Turn a pasted list into a scope document (not saved)
curl -s -b jar -H 'Sec-Fetch-Site: same-origin' -X POST http://localhost:8081/api/v1/scope/import \
  -H 'content-type: application/json' -d '{"text":"*.example.com\n10.0.0.0/8"}' > scope.json
cat scope.json
# {"include":[{"host":{"type":"suffix","value":{"suffix":"example.com","also_apex":false}}},{"host":{"type":"cidr","value":"10.0.0.0/8"}}],"exclude":[]}

# Save it (the new scope is compiled first; a bad regex leaves the live scope as it was)
curl -s -b jar -H 'Sec-Fetch-Site: same-origin' -X PUT http://localhost:8081/api/v1/scope \
  -H 'content-type: application/json' -d @scope.json
```

The proxy records each exchange's scope verdict (`in`, `out`, `empty` or
`host-mismatch`, filterable as `scope_state`), and the repeater and intercept use
the same scope. The accepted list forms are in the
[`crosswire-scope` README](../crates/crosswire-scope/README.md).
`PUT /api/v1/settings/enforce_scope_strictly` with `{"enabled":true}` makes the
repeater refuse out-of-scope sends outright instead of sending them with an
audit row; the setting is also persisted. Changing scope or strictness needs
`admin`.

## The repeater

`POST /api/v1/repeater/send` (needs `send`) sends one request through the
repeater, records it, and returns the outcome. The request is raw bytes, base64
(standard alphabet):

```sh
RAW=$(printf 'GET / HTTP/1.1\r\nHost: example.com\r\n\r\n' | base64)
curl -s -H "Authorization: Bearer $SECRET" -X POST http://localhost:8081/api/v1/repeater/send \
  -H 'content-type: application/json' \
  -d "{\"host\":\"example.com\",\"port\":443,\"tls\":true,\"raw\":\"$RAW\",\"mode\":\"raw\"}"
```

| Field | Meaning |
|---|---|
| `host`, `port`, `tls`, `sni` | Where to connect (the connection identity) |
| `raw` | The request bytes, base64 |
| `mode` | `raw` (default: sent untouched) or `assisted` (bare LF becomes CRLF and `Content-Length` is fixed, both recorded as diffs) |
| `follow` | `off` (default), `in-scope` or `always`; up to 10 hops |
| `verify` | `web-pki` (default) or `accept-invalid` |
| `reuse` | Reuse a kept-alive connection |
| `source` | An exchange id to link, stored as a `source:<id>` tag |
| `allow_out_of_scope` | Send despite a non-empty scope that excludes it (audited); refused when strict enforcement is on |

The response has the new `exchange_id`, the `scope_state`, the `changes` made
in assisted mode, `skipped` notes, `cert_override`, `ttfb_ms`, `total_ms` and
the `redirects` followed. Errors: `403 out_of_scope`, `422 parse_error`,
`502 upstream_error`. The repeater has its own transport
but the same destination gate rules as the proxy: loopback, link-local and
cloud-metadata addresses are refused unless you started `serve` with `--allow`. Repeater exchanges are filed under the
`repeater` namespace (`?namespace=repeater`). Every followed redirect
hop is sent as a plain `GET` with `Connection: close`, whatever the status (a
`307` or `308` included); the original method and body are not repeated.

Drafts (the editor state per tab) are saved with
`PUT /api/v1/repeater/tabs/{id}` (a JSON document, at most 1 MiB), listed with
`GET /api/v1/repeater/tabs`, read with `GET /api/v1/repeater/tabs/{id}` and
deleted with `DELETE`. Reading needs `read`; writing needs `send`.

## Intercept (requests)

Request intercept holds a request before it goes upstream so you can forward it
unchanged, edit it, or drop it. It always starts **disabled** and **its rules
are not saved**: after a restart it is off and the rules are back to their
defaults. Response intercept is not built.

```sh
curl -s -H "Authorization: Bearer $ADMIN_SECRET" http://localhost:8081/api/v1/intercept
```

```json
{"enabled":false,"rules":{"skip_extensions":["png","jpg","jpeg","gif","webp","svg","ico","woff","woff2","ttf","css","js","map"],"skip_out_of_scope":true,"overflow":"forward-newest","on_timeout":"forward"},"held":[]}
```

| Route | Needs | What it does |
|---|---|---|
| `GET /api/v1/intercept` | `read` | Enabled flag, rules and the held requests |
| `PUT /api/v1/intercept/enabled` | `admin` | `{"enabled":true}` |
| `PUT /api/v1/intercept/rules` | `admin` | Replace the rules (`skip_extensions`, `skip_out_of_scope`, `overflow`: `forward-newest` or `forward-oldest`, `on_timeout`: `forward` or `respond-504`) |
| `POST /api/v1/intercept/held/{id}/forward` | `send` | Forward unchanged, or with `{"head":"<base64>","body":"<base64>"}` to edit |
| `POST /api/v1/intercept/held/{id}/drop` | `send` | Drop it and close the client's connection |
| `POST /api/v1/intercept/forward-all` | `send` | Forward every held request |

Out-of-scope requests are never held under the default rules. A held body is
editable up to 4 MiB; a larger or streamed body is held but cannot be edited,
and a request held longer than 120 seconds follows `on_timeout`. The full list
of limits is in the [`crosswire-proxy` README](../crates/crosswire-proxy/README.md).

## Reading the OpenAPI document

```sh
curl -s http://localhost:8081/api/v1/openapi.json
```

The same document is committed at [docs/api/openapi.json](api/openapi.json) and
a test fails if the code and the file drift apart. The route-auth, same-site and
host checks listed in [security-gates.md](security-gates.md) walk that document,
so a new route cannot skip them.

## Install to first capture

`scripts/first-capture.sh` (`just first-capture`) is the end-to-end check of the
flow above: it builds the release binary, starts `serve` on free ports with a
bootstrap file, exchanges the token for a cookie, launches a headless
Chromium-based browser through the proxy to `https://example.com/`, and polls
`GET /api/v1/exchanges` until the first recorded exchange appears. It fails if
that takes 120 seconds or more from launch. It needs network access and a
Chromium-based browser, and CI runs it on Ubuntu. The same flow without a
browser, against a local upstream, is the `first_capture_under_two_minutes`
test in `crates/crosswire-server/tests/first_capture.rs`.
