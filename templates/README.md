# Templates

Pre-built starter configs ported verbatim from
[debian-multiarch-builder](https://github.com/ranjithrajv/debian-multiarch-builder)'s
`templates/` directory (Rust, Go, C/C++, Node.js, Python, Ruby). They use
that action's legacy key names (`summary:`, `license:`, `download_pattern:`,
`architecture_map:`, `dependencies:`) — all of which `lx` accepts and folds
into their modern equivalents at load time.

Start from one with:

```sh
lx init --template rust/eza            # writes package.yaml in the cwd
lx init --template go/hugo --output path/to/package.yaml
lx init --template                     # no match lists all names
```

Then edit `package_name`, `github_repo`, and the download pattern to match
the upstream release you are packaging, and check it resolves against a real
release without building:

```sh
lx validate package.yaml
```

Available templates: `rust/{eza,bat,generic,ripgrep}`, `go/{hugo,kubectl,generic}`,
`c/{neovim,generic}`, `nodejs/generic`, `python/generic`, `ruby/generic`.

Note: templates whose `download_pattern` has neither `{arch}` nor an
`architecture_map` (e.g. `go/hugo`) deliberately leave architecture patterns
unset so build-time auto-discovery decides — mirroring how the bash action
behaved for those configs.

Two deliberate deviations from the verbatim ports, both fixing templates
that could never match real upstream assets:

- `rust/bat.yaml` leaves armhf unpinned (bat's ARM assets end in
  `-gnueabihf`, which no single-suffix `{arch}` pattern can produce;
  auto-discovery resolves them instead) and validates against a current
  release.
- `rust/eza.yaml` ships unchanged, but note its `v{version}` pattern does
  not match any real eza asset name (eza releases have always been
  versionless) — it is kept as the canonical legacy-config example.

