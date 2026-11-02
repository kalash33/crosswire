## Summary

<!-- What changes and why. -->

## Linked issue

<!-- Closes #123, or "none". -->

## Checklist

- [ ] Commits are signed off (DCO): `git commit -s`
- [ ] Tests added or updated; `just lint` and `just test` pass locally
- [ ] Docs and `CHANGELOG.md` updated when the change is user-visible
- [ ] No generated artefact edited by hand; if `ui/` changed, the bundle was rebuilt with `just ui-build`
- [ ] No AI-attribution trailers or footers in the commits or in this description (see CONTRIBUTING.md); I can explain every line
- [ ] Wire-layer (`crosswire-h1`) changes have golden corpus coverage
- [ ] Changes to `crosswire-core` or `crosswire-h1` public API link an RFC issue
