# Drop-in: checkinstall

Part of the [drop-in roadmap](README.md). **Status:** open (Phase 6).
Current parity: [replacements.md](../architecture/replacements.md),
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

`lx` covers the "you supply the tree" half but not the capture:

- `lx build --from-dir`/`--from-file` (+ `--prefix`) — `lib/build.rs`.
- `build_mode: source` with `build_system: custom` runs `build_commands`/
  `install_commands` under `sh -c` with `$DESTDIR` set —
  `lib/plugins/build_system/custom.rs`.
- Dependency auto-detection from the staged tree: `lib/bindep.rs`,
  `lib/shlibdeps.rs`.
- `--install-build-deps` (`lib/builddeps.rs`).
- Per-file metadata (`lib/filemeta.rs`, `lib/plugins/contents.rs`) and
  rollback generations (`lib/rollback.rs`).

## Gap checklist

- [ ] Capture/exec mode: `lx build --capture '<cmd>'` (or
      `lx checkinstall <args>`) that runs an install command and packages
      the result. The `DESTDIR` path already works for well-behaved
      installers and is the first cut.
- [ ] Filesystem-delta tracking for installers that ignore `DESTDIR`:
      an `installwatch` equivalent (LD_PRELOAD) or an overlay/`fanotify`
      approach, with a before/after path+size+mtime+hash snapshot as the
      pragmatic first version.
- [ ] `--include`/`--exclude` filters to drop build artifacts, caches, and
      `/tmp` before packaging (also closes fpm's `-x` gap).
- [ ] Auto-populate metadata from the capture: ELF deps via
      `lib/shlibdeps.rs`, generated config files → conffiles/`backup`, man
      pages and licenses (staging already handles those), per-file
      modes/owners via `lib/filemeta.rs`.
- [ ] `--install=no`/`--install=yes`, `--strip`, and a `--backup` uninstall
      log recorded with the package.
- [ ] `lx remove`/`lx rollback` honor the captured file list for files the
      host package manager did not own.
- [ ] Snapshot the pre-install state so a captured install is reversible.

## Insertion points

| Change | File |
|---|---|
| Capture command and file-delta tracking | new `lib/capture.rs`; wire in `lib/cli.rs` (`build --capture`) |
| Reuse the `DESTDIR` install path | `lib/plugins/build_system/custom.rs` |
| Include/exclude filters | new `lib/capture.rs`, `lib/plugins/contents.rs` |
| Metadata from the capture | `lib/shlibdeps.rs`, `lib/filemeta.rs` |
| Uninstall log and reversal | `lib/remove.rs`, `lib/rollback.rs`, `lib/manifest.rs` |

## Done when

- `lx build --capture 'make install'` produces a native package from a
  project that ships only a `Makefile`.
- The captured package records its file list, and `lx remove`/`lx rollback`
  can reverse it even for files the host manager did not own.
