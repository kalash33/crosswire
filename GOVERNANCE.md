# Governance

Crosswire is a pure community project. There is no company behind it and nothing is gated. This file says who decides, how, and what happens if the maintainer goes away. The people in each role are listed in [MAINTAINERS.md](MAINTAINERS.md).

## Roles

- **Maintainer.** Merges pull requests, cuts releases, owns the repository, the GitHub organisation and the crates.io names. Votes on RFCs and on role changes.
- **Committer.** Has write access. Reviews and merges pull requests in the areas they know. Does not cut releases or change repository settings.
- **Triager.** Labels and closes issues, asks for reproducers, and points people to the right place. Has no write access to code.
- **Contributor.** Anyone who opens an issue, a pull request or a review. No invitation is needed.

## Decision making

- **One maintainer.** While [MAINTAINERS.md](MAINTAINERS.md) lists fewer than two maintainers, the maintainer decides (a benevolent dictator). The maintainer still explains the decision in the issue or pull request.
- **Two or more maintainers.** Decisions use lazy consensus. A proposal in an issue or pull request is accepted if no maintainer objects within a reasonable time. An objection must give a reason.
- **RFCs.** A change that needs wider agreement gets an issue labelled `rfc`. The timebox is 14 days. After that, the maintainers vote and the result is written in the issue.
- **Design-locked crates.** Public API changes to the crates that [CONTRIBUTING.md](CONTRIBUTING.md) lists as design-locked need an RFC first. Fixes, docs, corpus and tests do not.
- **Architecture.** Decisions that shape the code are recorded as ADRs in `docs/adr/`.

## Contributor ladder

Roles are earned by work, not by time served.

1. **Contributor to triager.** Anyone who has helped in issues for a while can be invited by a maintainer.
2. **Triager to committer.** Proposed by a maintainer after steady, good reviews and merged work. Maintainers vote.
3. **Committer to maintainer.** Proposed by a maintainer after sustained ownership of an area. Maintainers vote.

A maintainer may invite a contributor after three merged pull requests. The invitation is an offer, not an entitlement. A role can be given up at any time.

## Releases

- **release-plz runs on every push to `main` and opens or refreshes a release pull request when there is something to release.** It proposes the version bump and the `CHANGELOG.md` entry.
- **Tag only green commits.** The release workflow's UI drift gate stops the binaries from being built, but the release entry itself is still created; if the gate fails, delete that release, fix `main`, and tag again. Cut a tag only on a commit whose `ci` run is green.
- **Merging that pull request releases.** The merge bumps the version, updates `CHANGELOG.md` and tags `vX.Y.Z`.
- **The tag triggers the cargo-dist release workflow.** It builds the tier-1 binaries, attaches GitHub build-provenance attestations and publishes the GitHub release.
- **Pre-releases** are `vX.Y.Z-rc.N` tags cut by a maintainer.
- **Cadence.** A tagged release or pre-release every 4 to 6 weeks.
- **Pre-release tags are invisible to release-plz.** It only sees tags like `vX.Y.Z`, so until `v0.1.0` is cut, the release PR it opens proposes a stale or duplicate pre-release version. The maintainer closes that PR unmerged, or edits the version in the PR to the intended one.
- **Only server commits open a release PR.** release-plz opens one only for commits under `crates/crosswire-server`. Commits in other crates still appear in the notes through `changelog_include`. To release changes made elsewhere, edit the version in an existing release PR or push a server commit.
- **Fold the curated notes before merging.** release-plz inserts its generated version block directly under `## [Unreleased]`. Before merging a release PR, the maintainer folds the curated `[Unreleased]` entries into that block so they are not left stranded beneath it.
- **Token.** The `RELEASE_PLZ_TOKEN` repository secret is a fine-grained personal access token with contents and pull-requests write. The maintainer rotates it.

To check a release, see "Verifying releases" in [SECURITY.md](SECURITY.md).

## Ownership and escrow

The project must survive its maintainer being unavailable.

- **A named backup owner** is set as the GitHub successor and will be the second crates.io owner. The name goes in [MAINTAINERS.md](MAINTAINERS.md).
- **A GitHub team of at least two people** administers the repository before the first crates.io publish.
- **Credentials live in a shared vault** that the maintainer and the backup owner can both open.
- **No long-lived secrets by design.** crates.io trusted publishing and Sigstore OIDC sign and publish from CI. The one exception is `RELEASE_PLZ_TOKEN`, which is rotated.

## Archival plan

If there is no tag for six months, the backup owner:

1. Adds an "unmaintained" banner to the README.
2. Pins the last release.
3. Either hands the organisation to a volunteer who has 3 or more merged pull requests, or archives the repository.

Credentials are escrowed so the backup owner can cut a final release.

## Amendments

Change this file by pull request; the decision rule above applies.
