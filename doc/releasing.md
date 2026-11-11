# Versioning and releasing

A release is a `vX.Y.Z` tag pushed to GitHub, on a commit whose `Cargo.toml` has that
version, and a GitHub Release with the Linux and Windows packages attached. You set the
version and push the tag; the [Release workflow](../.github/workflows/release.yml) builds
the packages and makes the release as a draft, which you then publish.

## Setting the version

`make version-check` lists the commits since the last release (its `vX.Y.Z` tag) and
suggests the next version. Only commits that change the app count (`src`, `locales`,
`templates`, `plugin-sdk`, `Cargo.toml` and `Cargo.lock`); tooling, CI, docs and tests
never call for a release. It suggests minor when a commit message adds something
("Added...", "New...") or the plugin interface changed (major from 1.0.0 on), patch
otherwise, counting from the last release; a version `Cargo.toml` was already bumped to is
suggested when it is that far.

```sh
make version-check            # what changed since the last release, and the next version

make version-bump-patch       # increase patch version, commit "Bump version to X.Y.Z+1"
make version-bump-minor       # increas minor version, commit "Bump version to X.Y+1.Z"
make version-bump-major       # increas major version, commit "Bump version to X+1.Y.Z"

make version-bump V=2.0.0     # an exact version
```

Patch, minor and major count from the current version. `SDK=...` sets the plugin SDK's
version too. It asks before committing, and makes no tag and pushes nothing. See
[`scripts/version.sh`](../scripts/version.sh) for the details.

## Releasing

1. If `Cargo.toml` isn't at the version `make version-check` suggests yet, set it on a
   branch and merge it through a pull request (`main` is protected), for example:

   ```sh
   git switch -c release/v0.10.1
   make version-bump-patch
   git push -u origin release/v0.10.1
   gh pr create --fill
   ```

2. Once the pull request is merged, tag the merged `main` with the same version and push
   the tag:

   ```sh
   git switch main
   git pull
   git tag v0.10.1
   git push origin v0.10.1
   ```

   The workflow checks that the tag matches the version in `Cargo.toml` and stops if not.

3. Wait for the **Release** workflow to finish (Actions tab). It builds both packages and
   creates a **draft** release for the tag, with the packages attached and notes
   generated from the commits.

4. On GitHub, open **Releases**, check the draft (notes, the two packages), and click
   **Publish release**.

## Building the packages without releasing

Run the **Release** workflow by hand (Actions > Release > Run workflow): it only builds the
packages and keeps them as the run's artifacts for 7 days. Locally, `make dist` builds the
same packages into `dist/`.
