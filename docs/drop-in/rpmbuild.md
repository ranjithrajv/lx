# Drop-in: rpmbuild

Part of the [drop-in roadmap](README.md). **Status:** open (Phase 4).
Current parity: [replacements.md §2.4](replacements.md),
[`decisions/2026-09-11-rpm-config.md`](../decisions/2026-09-11-rpm-config.md).

## The contract

`rpmbuild -bb|-ba|-bs|-bp|-bc|-bi <spec>` evaluates a `.spec`: the header
(`Name`, `Version`, `Release`, `License`, `URL`, `Source*`, `Patch*`,
`BuildRequires`, `Requires`, `Provides`, `Obsoletes`, `Conflicts`,
`BuildArch`) and the `%prep`/`%build`/`%install`/`%check`/`%files`/
`%changelog` sections, expanding `%{…}` macros and `%if`/`%ifarch`/
`%bcond_with` conditionals. `%files` directives (`%doc`, `%license`,
`%config(...)`, `%attr(...)`, `%dir`, `%exclude`, `%ghost`, `%defattr`)
decide payload placement and metadata. Subpackages (`%package`,
`%files <sub>`) yield several RPMs from one spec.

## Current state

- The in-process `.rpm` writer is real (`lib/rpmarchive.rs`, the `rpm`
  crate): relations, compression, scriptlets, and ELF-based
  `auto_provides`/`auto_requires` (`auto_elf_relations`).
- `rpm.defines` is **accepted but warned-ignored** — there is no macro
  engine (`warn_ignored_defines`).
- Triggers are best-effort: the trigger dependency is emitted and the
  script is attached as a `%post` scriptlet (`apply_triggers`), not a real
  `%triggerin`/`%triggerun`.
- `lx` *emits* a minimal `.spec` for a `.src.rpm` (`lib/source.rs`,
  `render_rpm_spec`) but cannot *consume* one.
- `--install-build-deps` installs host toolchains, but nothing reads
  `BuildRequires` from a spec.

## Gap checklist

- [ ] Parse a `.spec` header and sections into build steps.
- [ ] Macro expansion: `%{_bindir}`, `%{_libdir}`, `%{_topdir}`,
      `%{buildroot}`, `%{?dist}`, `%global`, `%define`, `%{name}`/
      `%{version}`/`%{release}`, and `%configure`, `%cmake`, `%meson`,
      `%make_build`, `%make_install`, `%pyproject_*`.
- [ ] Conditionals: `%if`, `%ifarch`, `%ifos`, `%bcond_with(out)`, `%else`,
      `%endif`.
- [ ] Subpackages: `%package`, `%description -n`, `%files <sub>` → multiple
      outputs.
- [ ] `%files` directives, applied through the existing per-file metadata
      (`lib/filemeta.rs`) and RPM file options.
- [ ] Real trigger scriptlets, not the dependency-plus-`%post` stand-in.
- [ ] `BuildRequires` → host deps (`lib/builddeps.rs`); `BuildRoot`,
      `%clean`, `--nodeps`, `--short-circuit`, `--rebuild`.
- [ ] `Source*`/`Patch*`, `%patch`/`%autopatch`, `%autosetup`.
- [ ] A `rpmspec --eval`-style render/eval command and linting.

## Insertion points

| Change | File |
|---|---|
| `spec` build-system plugin (parse, evaluate, drive build) | new `lib/plugins/build_system/spec.rs`; register in `lib/plugins/build_system/mod.rs` |
| Macro expansion and conditionals | new `lib/spec.rs` (or an evaluator shelled out to `rpmspec`) |
| Apply macros instead of warning | `lib/rpmarchive.rs` (`warn_ignored_defines`) |
| `BuildRequires` into the install path | `lib/builddeps.rs` |
| `rpm:` config additions (`spec:`, `build_requires:`, `macros:`) | `lib/config.rs` |
| Multi-output | `lib/build.rs`, `lib/plugins/mod.rs` |

## Done when

- `lx build` accepts a `.spec` (including one with conditionals and
  subpackages) and produces RPMs that `rpm -qip` and `rpmlint` accept.
- `rpm.defines` are applied, not warned.
