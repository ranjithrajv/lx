# GitHub Action

Part of the [lx docs](../README.md).

`action.yml` wraps `lx build` as a composite action — a drop-in
replacement for `debian-multiarch-builder`'s action.yml (same input/output
names):

```yaml
- uses: ranjithrajv/lx@v1
  with:
    config-file: package.yaml
    version: v0.24.0
    build-version: '1'
    # architecture: all          # or a single arch; default: all
    # lintian-check: 'true'
    # pinned-metadata: release-metadata.json
```

Outputs: `packages` (space-separated `.deb` filenames), `source-packages`
(`.dsc` filenames), `summary-path` (`build-summary.json`). It needs no host
`apt-get` dependency install at all — building, source packages, and
`lintian` (when the runner has one) are all native or host-tool-backed
rather than shelled-out `tar`/`jq`/`yq`/`dpkg-*` in a container. The action
builds `lx` from source by default (cached via `Swatinem/rust-cache`); once
a release exists, set the `lx-version` input (e.g. `lx-version: v0.1.0`) to
download the matching musl-static `lx-<tag>-<triple>.tar.gz`, verify its
`.sha256`, and use it instead of compiling (`v*` tags are cut by
`.github/workflows/release.yml`).

## Attribution & telemetry

Pin an exact, greppable ref so usage is attributable via code search
(`uses: ranjithrajv/lx@v1`):

```yaml
- uses: ranjithrajv/lx@v1   # keep major tag moving; cite full version in issues
```

Anonymous run telemetry is opt-out (`telemetry-enabled: 'true'` default):
one POST per run with `action ref, arch, job status, runner arch` only —
no repo, user, or PII. It fires only when the `LX_TELEMETRY_URL` repo
Variable is set (your collector endpoint), so forks/private users emit
nothing by default. Set `telemetry-enabled: 'false'` to disable entirely.

