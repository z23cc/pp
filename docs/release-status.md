# Release status

`pp` is preparing its first `0.1.0` release. The crate metadata is release-shaped, but publication should wait until the verification checklist below passes.

## Crates.io

- Package: `pp-cli`
- Binary: `pp`
- Current release target: `0.1.0`
- Publication status: not published yet

## Temporary typify patch

`Cargo.toml` currently patches the transitive typify crates to a fork:

- `typify`
- `typify-impl`
- `typify-macro`
- repository: `https://github.com/z23cc/typify`
- pinned rev: `1e4213a8e76f2bcc54ba1f70c04816aa388b5f08`

Reason: temporary upstream patch for `oxidecomputer/typify#1011` / `#1012` plus the nullable-composition inner-name fix needed by GitHub-scale generation.

Removal condition:

1. `progenitor` depends on a released typify version containing the fixes.
2. The `[patch.crates-io]` entries can be removed from `Cargo.toml`.
3. Fast verification passes.
4. Deep verification, including the GitHub-scale regression described in `docs/plans/typify-patch-and-slicing-2026-05-16.md`, still passes.

## Release checklist

Before publishing `0.1.0`:

1. Run fast verification from `docs/verification.md`.
2. Run the standard generated-workspace smoke profile.
3. Run the deep fixture dogfood profile.
4. Re-run at least one large-spec or documented slice check when the fixture is available.
5. Confirm `CHANGELOG.md` has the intended `0.1.0` entry.
6. Confirm README badges and installation instructions match the publication state.
7. Tag the release only after generated artifact validation passes.
