# crosswire-core

Small data types shared by the Crosswire crates. Part of
[Crosswire](../../README.md).

It holds:

- `Span`: a half-open byte range (`u32` start and end).
- `Diagnostic`, `DiagnosticCode`, `Severity`: a finding about input, located by a
  span. `DiagnosticCode` has 25 stable snake_case names (`as_str()`) and a
  default severity per `Profile`; the catalogue is in
  [docs/h1-diagnostics.md](../../docs/h1-diagnostics.md).
- `Profile`: `Strict` or `Lenient` (the default).
- `Change`, `ChangeOp`, `ChangeReason`: a record of one edit to a message.
- `EncodingHint`, `FieldHint`: per-field encoding hints.

Dependencies: `bytes` only (`serde` is optional). The crate is `no_std` + `alloc`
when the default `std` feature is off.

| feature | default | effect |
| --- | --- | --- |
| `std` | yes | enables `bytes/std` |
| `serde` | no | `Serialize`/`Deserialize` for the types above, and `bytes/serde` |
