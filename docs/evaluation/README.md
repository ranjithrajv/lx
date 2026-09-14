# Evaluation

Part of the [lx docs](../README.md).

Point-in-time evaluations of `lx` against a named engineering quality. Each
one fixes its rubric first, walks the repository for evidence, and records
the gaps with the same weight as the wins. They are working documents, not
marketing pages: a criterion `lx` fails is the most useful thing in them.

| Evaluation | Question it answers |
|---|---|
| [Interoperability](interoperability.md) | Can `lx` exchange artifacts, metadata, and control with the rest of the Linux packaging ecosystem — in both directions? |
| [Composability](composability.md) | Can `lx`'s capabilities be selected, combined, and reused — by operators, and by people adding a plugin? |

## Method

Every evaluation follows the same three steps:

1. **Define the rubric** — the criteria and what "pass" means, written down
   before looking at the code.
2. **Gather evidence** — the module, trait, command, and test that
   demonstrates each criterion. Where the evidence is a test that shells
   out to a reference tool, it is named by file and function.
3. **Record the gaps** — the criteria that are partial or unmet, with the
   reason and any tracked follow-up.

Verdicts use one scale:

| Verdict | Meaning |
|---|---|
| ✅ | Demonstrated by code and a standing test, or by a live reference-tool check |
| 🟡 | Implemented but partial, unproven, or narrower than the criterion |
| ❌ | Not present |

## Scope and honesty

These evaluate the **repository as it stands**, not a release. `lx` is
pre-1.0 with no tagged release yet, so a ✅ means "the mechanism exists and
is exercised", not "proven in years of field use". The gaps are
load-bearing: read them alongside the
[maturity table](../analysis/landscape.md#technology-readiness-trl) for how
proven each surface is, and the benchmark
[ground rules](../../benchmarking/README.md#ground-rules-for-a-fair-number)
for what a passing check does and does not claim.

A note on test evidence: some criteria are enforced by a **standing test**
that runs whenever the reference tool is on `PATH` (e.g. real `dpkg-deb`),
while others were verified **once, empirically**, and recorded in a
[decision doc](../decisions/README.md) (e.g. a live `dpkg-source -x`
reconstruction). The evaluations say which is which, so a regression in the
ungated checks is not silently assumed to be caught.

## Adding an evaluation

1. Write the rubric first: the criteria, and what evidence would satisfy
   each.
2. Walk `lib/`, `tests/`, and the
   [architecture docs](../architecture/overview.md); name files and tests.
3. Add the doc here, add its row to the table above, and link it from the
   [hub](../README.md).
