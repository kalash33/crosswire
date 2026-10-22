# crosswire-ca

A local certificate authority and a per-host leaf certificate issuer, for a
TLS-intercepting proxy. Part of
[Crosswire](https://github.com/kalash33/crosswire/blob/main/README.md), an open-source Rust web security testing platform.

`Ca` is a P-256 root persisted through a `KeyStore`. `LeafIssuer` signs one
short-lived leaf per host name or IP literal with that root, caches the leaves,
and hands them out as `rustls` `CertifiedKey`s that a TLS server can present.

## What it does and does not do

- Generates a CA, loads one from PEM (checking that the key matches the
  certificate and that the certificate is a CA), and saves and loads it through
  a `KeyStore`. `FileKeyStore` keeps two files in a directory.
- Mints leaves with a DNS SAN for names and an IP SAN for literals, signed by
  the CA, cached with LRU eviction, and rate-limited on minting.
- Reports the SHA-256 SPKI pin of the CA for Chromium's
  `--ignore-certificate-errors-spki-list`.
- Does **not** install the CA into any operating system, browser or keychain.
  Trust installation, CA rotation and the OS keychain store are not built; the
  spec describes them (section 11.3) for the server plan.
- Does **not** keep a disk cache of leaves; the cache is in memory only.
- Does not do any networking and does not decide which hosts to intercept.

## Key types

| type | what it is |
| --- | --- |
| `Ca` | the root: `generate`, `from_pem`, `load_or_generate`, `cert_pem`, `cert_der`, `key_pem`, `spki_sha256_base64`; cheap to clone |
| `KeyStore` | trait with `load() -> Result<Option<(cert_pem, key_pem)>>` and `save(cert_pem, key_pem)` |
| `FileKeyStore` | `ca.crt` (0644) and `ca.key` (0600) in a 0700 directory |
| `LeafIssuer` | `new(Arc<Ca>, IssuerLimits)`, `issue(&ServerName) -> Result<Arc<CertifiedKey>>`, `cached_len` |
| `IssuerLimits` | `cache_entries` and `mint_rate_per_sec` |
| `CaError` | `Generate`, `Pem`, `Io`, `RateLimited`, `InvalidHost`, `CaExpired`; `#[non_exhaustive]` |

## Parameters and limits

| parameter | value | source |
| --- | --- | --- |
| CA key and signature | ECDSA P-256 with SHA-256 | `Ca::generate` |
| CA subject | `Crosswire CA <subject_suffix>`; the caller chooses the suffix | `Ca::generate` |
| CA constraints | `CA:TRUE`, path length 0, `keyCertSign` and `cRLSign` | `Ca::generate` |
| CA validity | 3650 days (`CA_VALIDITY_DAYS`), starting one day in the past | `ca.rs` |
| Leaf validity | at most 398 days (`LEAF_VALIDITY_DAYS`) and never past the CA's `notAfter` minus one day; `notBefore` one day in the past | `issuer.rs` |
| Leaf usage | `digitalSignature`, EKU `serverAuth`, authority key identifier | `LeafIssuer::mint` |
| Leaf key | one P-256 key pair per `LeafIssuer`, reused for every host | `LeafIssuer::new` |
| Leaf chain | the leaf, then the CA certificate | `LeafIssuer::mint` |
| Leaf common name | the host, omitted when longer than 64 bytes (the SAN carries it) | `LeafIssuer::mint` |
| Serial numbers | 16 random bytes, first byte masked so the number is positive | `random_serial` |
| Cache size | 4096 leaves (`IssuerLimits::default().cache_entries`) | `issuer.rs` |
| Mint rate | 50 new leaves per second, token bucket; cache hits are free (`mint_rate_per_sec`) | `issuer.rs` |

`issue` returns `CaError::RateLimited` when a new leaf is needed and the bucket
is empty, `CaError::InvalidHost` for a name that cannot be certified, and
`CaError::CaExpired` when the CA is too close to expiry to sign a valid leaf.

## Security notes

- The CA private key is reachable only through `Ca::key_pem`, which exists for
  the key store. Never log, export or sync it. `Ca`'s `Debug` prints only the
  SPKI pin.
- `FileKeyStore::save` writes each file to a temporary name created with its
  final mode (0600 for the key), fsyncs it, then renames it, so the key is never
  visible with looser permissions. The directory is created 0700 and set to
  0700 even if it already exists, so point it at a directory dedicated to the
  CA. On Windows no mode bits are applied.
- `save` writes `ca.crt` before `ca.key`. A crash between the two leaves an
  orphan `ca.crt`; `load` then returns an error (`ca.crt and ca.key must both
  exist`). Delete the orphan and regenerate.
- Keep the CA out of project directories. This crate takes whatever directory
  it is given and does not check.
- A leaf is signed for whatever name `issue` is asked for. Deciding which names
  are allowed is the caller's job.

## Example

```rust,ignore
use std::sync::Arc;
use crosswire_ca::{Ca, FileKeyStore, IssuerLimits, LeafIssuer};
use rustls::pki_types::ServerName;

let store = FileKeyStore::new("/home/me/.config/crosswire/ca");
let ca = Ca::load_or_generate(&store, "my-laptop")?;
println!("SPKI pin: {}", ca.spki_sha256_base64());

let issuer = LeafIssuer::new(Arc::new(ca), IssuerLimits::default());
let name = ServerName::try_from("example.com")?;
let leaf = issuer.issue(&name)?; // Arc<rustls::sign::CertifiedKey>
```

The `rustls` types in the signature are `rustls` 0.23; add the same version to
your own dependencies to name them.

## Status

Pre-alpha, `publish = false`; the API is not frozen. The crate depends only on
registry crates, so `cargo package -p crosswire-ca` works.

Design: [spec section 11.3](https://github.com/kalash33/crosswire/blob/main/docs/superpowers/specs/2026-10-22-crosswire-design.md)
(CA handling).
