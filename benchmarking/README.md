# Benchmarking `lx`

This directory keeps **records** of how fast `lx` is compared to the tools
it can replace. The comparison set is not "every packager" — it is exactly
the tools listed in
[`docs/drop-in/replacements.md`](../docs/drop-in/replacements.md),
so a number here always answers a real substitution question: *if I swap
tool X for `lx`, what does the swap cost in wall-clock time?*

- [`run.sh`](run.sh) — reproduces the measurements on the current machine.
- [`benchmarks.md`](benchmarks.md) — the full catalogue of benchmarks to
  record, with levels and status.
- [`results.md`](results.md) — the record log. One dated section per run.

## What gets measured

The full backlog — every `lx` job worth timing against the tool it replaces,
grouped by replacement level, with a status per row — is the
[**benchmark catalogue**](benchmarks.md). This file is the harness; the
catalogue is the list.

Only operations with an **offline, local equivalent** are automated. Three
rows run today:

| # | `lx` operation | Compared against | Level | Status |
|---|---|---|---|---|
| 1 | `lx build --from-dir` (pack a `.deb`) | `dpkg-deb --build`; `fpm -s dir -t deb`; `nfpm pkg -p deb` | functional / feature parity | automated |
| 2 | `lx deps resolve` | `dpkg-shlibdeps` | **drop-in** (command) | automated |
| 3 | `lx repo` (apt index) | `dpkg-scanpackages`; `apt-ftparchive packages` | functional | automated |

Everything else in the catalogue is network-, host-tooling-, or CI-gated.
Automated rows run with no network access, so results are reproducible and
not dominated by release-download latency.

## Ground rules for a fair number

Benchmarks across different tools are easy to make meaningless. Every
automated run follows these rules:

1. **Same input.** One staged payload / ELF / package directory is built
   once per run and reused by every tool; only the tool under test changes.
2. **Release build of `lx`.** Run `cargo build --release` first, and point
   the harness at `target/release/lx` (or a system `lx`). A debug build is
   not comparable and the harness warns if one is used.
3. **Warm-up + repeats.** Each command runs once to warm caches, then N
   times (default 5, override with `RUNS=`). The record reports the
   **median** and **min** wall-clock time; medians are reported for the
   comparison, min is kept to spot noise.
4. **Same machine, same run.** All tools for a given row are timed back to
   back in a single `run.sh` invocation. Cross-run numbers are not
   compared in the same table.
5. **Offline.** No benchmark fetches a release.
6. **Disclosed work.** `lx` does more per invocation than the tool it is
   compared against — generated control/changelog/copyright, ELF dependency
   inference, extra index files, etc. Those are part of the measured time
   and are called out per row in the report (and therefore in
   [`results.md`](results.md) once recorded). Read a comparison as "the
   whole `lx` command vs. the whole reference command", not "the compression
   step alone".

## Running

```sh
# from the repository root
cargo build --release
./benchmarking/run.sh                 # prints a Markdown report to stdout
RUNS=10 ./benchmarking/run.sh         # more repeats
LX_BIN=/usr/bin/lx ./benchmarking/run.sh
BENCH_DIST=sid ./benchmarking/run.sh  # suite used for the one-.deb comparison
```

The harness auto-detects each reference tool and records `not installed`
for any it cannot find, so it is safe to run on a machine that only has
`dpkg` — you simply get a shorter table.

To record a run, append its output to [`results.md`](results.md):

```sh
./benchmarking/run.sh >> benchmarking/results.md
```

The report carries its own dated `## Run …` heading and an environment
block (host, architecture, `lx` and reference-tool versions), so a record is
self-describing.

## Reading the results

- **Medians, not averages.** A single slow iteration (scheduler, I/O)
  should not move the headline number.
- **Do not compare across machines or lx versions** unless the
  environment header matches. The header records `lx --version`, kernel,
  architecture, and the reference tools' versions.
- **A win here is not a product claim.** These measure local CPU/IO work;
  they say nothing about correctness, metadata richness, or supply-chain
  features. Those live in the replacement doc and the comparison docs.
- **`lx` losing a row is expected and fine.** Some reference tools do less
  (e.g. `dpkg-scanpackages` writes an uncompressed `Packages`; `lx repo`
  also gzips it and writes `Release` with checksums). The per-row Notes
  keep that from reading as a regression.

## Adding a benchmark

1. Pick a row from the [catalogue](benchmarks.md) and set its status to
   `automated`.
2. Add a `bench_*` function to [`run.sh`](run.sh) that stages its input,
   times each available tool with `measure`, and emits rows via `run_one`.
3. Call it from `main`, run it, record the output in `results.md`, and set
   the catalogue row's status to `recorded`.
