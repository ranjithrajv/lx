# Drop-in: checkinstall

Part of the [drop-in roadmap](README.md). **Status:** partial (Phase 6 — the
`lx capture` first cut has landed). Current parity:
[replacements.md](replacements.md),
[landscape.md stage 3](../analysis/landscape.md).

## The contract

`checkinstall` wraps an arbitrary install command (`checkinstall make
install`): it intercepts the files the installer writes, snapshots the
result, and builds a package from that set. It also records an uninstall
log. Common flags: `--install=no`, `--strip`, `--backup`, `--include`/
`--exclude`, `--pkgname`/`--pkgversion`/`--pkgrelease`, `--pkglicense`/
`--pkggroup`/`--maintainer`, `--delayed`.

The key difference from a plain packager is that the user supplies an
*action*, not a tree: `checkinstall <command>` runs the command and
captures what it did.

## Current state

- `lx capture --name <n> --version <v> '<cmd>'` runs `'<cmd>'` with
  `$DESTDIR` set to a staging tree, applies `--exclude` globs, auto-detects
  `depends:` from the captured ELFs, and packages the tree **as-is**
  (`lib/capture.rs`). The captured layout (`usr/…`, `etc/…`) is preserved,
  unlike `--from-dir`.
- The `--from-dir`/`--from-file` + `--prefix` "you supply files" path
  (`lib/build.rs`), `build_mode: source` + `build_system: custom` with
  `$DESTDIR` (`lib/plugins/build_system/custom.rs`), the shlibdeps core
  (`lib/shlibdeps.rs`), per-file metadata (`lib/filemeta.rs`), and rollback
  generations (`lib/rollback.rs`) all already exist.

## Gap checklist

- [x] Capture/exec mode (`lx capture`), including `--exclude` filters and
      auto-populated deps.
- [ ] Filesystem-delta tracking for installers that ignore `DESTDIR`:
      an `installwatch` equivalent (LD_PRELOAD) or an overlay/`fanotify`
      approach, with a before/after path+size+mtime+hash snapshot as the
      pragmatic first version.
- [ ] `--install=no`/`--install=yes`, `--strip`, and a `--backup` uninstall
      log recorded with the package.
- [ ] `lx remove`/`lx rollback` honor the captured file list for files the
      host package manager did not own.
- [ ] Snapshot the pre-install state so a captured install is reversible.

## Insertion points

| Change | File |
|---|---|
| Capture command and excludes | `lib/capture.rs` (landed), `lib/cli.rs` |
| Filesystem-delta tracking | `lib/capture.rs` |
| `--strip`, `--backup`, uninstall log | `lib/capture.rs`, `lib/manifest.rs` |
| Reversal for captured files | `lib/remove.rs`, `lib/rollback.rs` |

## Done when

- `lx capture 'make install'` produces a native package from a project that
  ships only a `Makefile`. **(Met.)**
- The captured package records its file list, and `lx remove`/`lx rollback`
  can reverse it even for files the host manager did not own.
