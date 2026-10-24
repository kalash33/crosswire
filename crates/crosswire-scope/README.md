# crosswire-scope

The grammar, evaluator and program-list importer that decide whether a URL is
in a project's testing scope. Part of
[Crosswire](https://github.com/kalash33/crosswire/blob/main/README.md), an
open-source Rust web security testing platform.

This crate has no dependencies on other Crosswire crates. It returns its own
[`PairVerdict`] for the two-identity case; the proxy and tools crates map that
to the model's `ScopeState`. A `Scope` serialises to JSON (`include` and
`exclude` lists of rules), which is the document the API's `PUT /api/v1/scope`
accepts and the server stores in the project's `scope` setting.

## Model

A [`Scope`] is an ordered list of include [`Rule`]s and exclude [`Rule`]s.
`Scope::compile` builds every regex once (each with a 10 MiB compiled-size
limit) into a [`Compiled`] matcher. `Compiled::evaluate` returns a [`Verdict`]
(`In` / `Out` / `Empty`) for one request [`Input`] (host, port, scheme, path).

A [`Rule`] matches when its host pattern matches **and** every set constraint
also matches; an unset (`None`) constraint matches anything.

### Host comparison

- Host names compare ASCII-case-insensitively.
- A single trailing dot is stripped (`example.com.` == `example.com`).
- A bracketed IPv6 literal (`[::1]`) is unbracketed before matching.
- An IP literal matches **only** `Ip` / `Cidr` rules, never a host-pattern
  rule; conversely a host name never matches an `Ip` / `Cidr` rule.

### Host patterns

| Pattern                       | Matches                                             |
|-------------------------------|-----------------------------------------------------|
| `Exact("example.com")`        | exactly `example.com`                               |
| `Suffix { "example.com", also_apex: false }` | any sub-domain `*.example.com` (not the apex) |
| `Suffix { "example.com", also_apex: true }`  | the apex `example.com` and any sub-domain    |
| `Regex("^(api\|www)\\.example\\.com$")` | host name matching the regex (unanchored `is_match`) |
| `Ip(1.2.3.4)`                 | the IP literal `1.2.3.4`                            |
| `Cidr(10.0.0.0/8)`            | any IP literal inside the block                     |

### Other constraints

| Field    | Type                     | `None` means |
|----------|--------------------------|--------------|
| `ports`  | `Option<BTreeSet<u16>>`  | any port     |
| `scheme` | `Option<Scheme>` (`http`/`https`) | any scheme |
| `path`   | `Option<PathMatch>` (`Prefix` / `Regex`) | any path |

## Precedence (spec §4.7)

1. No rules at all → `Empty`.
2. Try include rules. If none match → `Out`.
3. Try exclude rules. A matching exclude wins over any include → `Out`.
4. Otherwise → `In`.

## Two-identity evaluation

`Compiled::evaluate_pair(connection, message)` evaluates the connection target
and the message (Host header) identity against the same scope and returns a
[`PairVerdict`]:

- both `In` → `In`; both `Out` → `Out`; scope empty → `Empty`.
- the two single verdicts disagree → `HostMismatch`.

## Importing a program list

`parse_program_list` turns a pasted scope list into a `Scope` (every entry is
an include rule). Blank lines and `#` comments are ignored.

| Form                         | Example                             | Rule                                   |
|------------------------------|-------------------------------------|----------------------------------------|
| bare host                    | `example.com`                       | exact host                             |
| wildcard                     | `*.example.com`                     | suffix (sub-domains only)              |
| full URL                     | `https://api.example.com:8443/v1`   | exact host + scheme, port, path prefix |
| CIDR block                   | `10.0.0.0/8`                        | CIDR                                   |
| IP literal                   | `1.2.3.4`, `[::1]`                  | IP                                     |

A line that cannot be parsed yields `ImportError::Line { line, reason }` with
the 1-based line number.

[`Scope`]: crate::Scope
[`Rule`]: crate::Rule
[`Compiled`]: crate::Compiled
[`Input`]: crate::Input
[`Verdict`]: crate::Verdict
[`PairVerdict`]: crate::PairVerdict
