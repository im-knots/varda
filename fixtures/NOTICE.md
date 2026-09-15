# Bundled fixture profiles

These fixture definitions come from the [Open Fixture Library](https://open-fixture-library.org/),
vendored at commit `b967624d1ba9109577eacb6e8bab407bcc4f4e87`.

653 definitions across 134 manufacturers. Varda reads them through
[`src/internal/dmx/ofl.rs`](/src/internal/dmx/ofl.rs) and normalizes them into its own
`FixtureProfile`; see [/spec/dmx-output.md](/spec/dmx-output.md) § Profile Format.

## Licence

Open Fixture Library is MIT licensed. `LICENSE-OFL` in this directory is that licence, copied
verbatim from the upstream repository, and it is the only licence file the repository carries:
there is no separate grant covering the fixture data specifically.

Individual definitions credit their contributors in each file's `meta.authors` field.

## Why these are bundled rather than fetched

A rig is patched at load-in, which is precisely when a venue has no usable network. Fetching
definitions on demand would make the fixture picker empty exactly when an operator needs it.

## Updating

The vendored set is a pinned snapshot and goes stale as Open Fixture Library adds fixtures.
Refresh it as part of the release checklist:

1. Pick the upstream commit to pin and record its SHA above.
2. Replace this directory's contents from that commit's `fixtures/` tree.
3. Run `cargo test --lib dmx::ofl` — the corpus test parses every bundled definition and fails
   on any that neither normalizes nor rejects with a named reason.

## Local overrides

A definition in `<workspace>/.varda/fixtures/` is searched **before** this directory, so an
operator correcting a channel map for tonight's rig never has to wait for a release.
