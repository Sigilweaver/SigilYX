# Releasing SigilYX

This documents the actual release flow for the `sigilyx` crate and PyPI
package. Publishing is triggered by creating a GitHub Release; that fires
both `.github/workflows/publish-crate.yml` and
`.github/workflows/publish-pypi.yml`, which sync the crate/package version
from the release tag and publish to crates.io and PyPI respectively.

## Steps

1. **Bump the version** in `sigilyx/Cargo.toml`, `sigilyx-python/Cargo.toml`
   (both the package version and its `sigilyx-core` path-dependency version),
   and `pyproject.toml`. Run `cargo generate-lockfile` (or `cargo build`) so
   `Cargo.lock` picks up the bump.
2. **Update `CHANGELOG.md`**: move the `[Unreleased]` entries under a new
   `## [X.Y.Z] - YYYY-MM-DD` section.
3. **Commit** the bump, e.g. `release: vX.Y.Z`, and push to `main`.
4. **Wait for CI and audit to run** on that commit (both `ci.yml` and
   `audit.yml` trigger on push to `main` when the commit touches
   `Cargo.toml`/`Cargo.lock`, which a version bump always does).
5. **Check release readiness** before tagging or creating the release:
   ```bash
   ./scripts/check-release-ready.sh main
   ```
   This confirms the most recent `ci.yml` and `audit.yml` runs for that
   commit both completed successfully. GitHub Actions has no way for the
   publish workflows to `needs:` a job defined in a separate workflow file,
   so this has to be checked here, before the release is created, rather
   than inside `publish-crate.yml` / `publish-pypi.yml` themselves. Do not
   proceed if the script exits non-zero - fix CI/audit first.
6. **Tag and create the GitHub Release**, e.g.:
   ```bash
   git tag vX.Y.Z
   git push origin vX.Y.Z
   gh release create vX.Y.Z --title vX.Y.Z --notes-from-tag
   ```
   Creating the release triggers `publish-crate.yml` and
   `publish-pypi.yml`, which sync the version from the tag name and publish
   to crates.io and PyPI.
7. **Verify** the new version shows up on
   [crates.io](https://crates.io/crates/sigilyx) and
   [PyPI](https://pypi.org/project/sigilyx/), and that both publish workflow
   runs succeeded in the Actions tab.

## Notes

- `workflow_dispatch` on either publish workflow (with a `tag` input) is
  available as a manual fallback, e.g. to retry a publish after a transient
  failure without cutting a new release. Run `check-release-ready.sh` for
  the target commit first in that case too.
- `audit.yml` only runs on pushes to `main` that touch `Cargo.toml`,
  `Cargo.lock`, or the workflow file itself (plus a weekly schedule), so a
  release commit that doesn't touch those paths (it always does, since the
  version bump is in `Cargo.toml`) is what makes step 5's per-commit check
  meaningful. A non-blocking `cargo audit` finding is expected to stay green
  in CI (see the `ignore:` list in `audit.yml`); this check exists to catch
  a *new*, unaddressed advisory before it ships in a release.
