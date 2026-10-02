# Actual-binary A/B benchmarks

## Reproduce

Preserve a release binary from commit `25cff9f` before building the updated code.
The original run used `/home/dalton/.jcode/scratch/fjx-ab/fjx-before`.
Build both binaries with the same toolchain and release profile. Then run:

```sh
cargo build --release
FJX_BENCH_BEFORE=/path/to/fjx-before \
FJX_BENCH_AFTER="$PWD/target/release/fjx" \
FJX_BENCH_REPORT=/path/to/benchmark-report.json \
cargo test --test benchmark_ab -- --ignored --nocapture
```

`FJX_BENCH_AFTER` defaults to `target/release/fjx`. The report path is optional.
The ignored integration test invokes the actual binaries, not internal functions.
Normal CI runs the actual-CLI fixture smoke test but skips all timed A/B measurements.

## Method

Each operation has two untimed warmups per binary followed by 15 samples per
binary. A/B execution order alternates. Reported wall time is the median and
covers process creation, CLI execution, HTTP handling where applicable, and
reading captured stdout. Fixture setup, semantic validation, and input-file
creation are outside the timed region. Output goes to files to avoid pipe
backpressure. Child processes have a 10-second deadline. The mock listener is
nonblocking, has a stop signal and one-minute deadline, and bounds stream I/O.
There are no timing assertions or performance thresholds.

The loopback-only fixture serves schema-compatible issue and label objects:
120 issues, representative 4,320-byte bodies, page size 30, and exactly six
titles containing `needle`, with identifiers 20, 40, 60, 80, 100, and 120.
`X-Total-Count` advertises the selected result count. Responses close each
connection. A synthetic fixture token is used, never a real credential.

The four comparisons are:

1. `issue list --help`: old help versus new scoped help, bytes and latency.
2. `issue list --all --limit 30 --json`: old complete typed records versus new
   `--fields number,title,state`. Both return the same 120 identifiers, titles,
   and states in four requests. Projection changes stdout, not HTTP bodies.
3. The same old complete listing versus new `--search needle` plus projection.
   Local filtering of old titles must match the six new identifiers and their
   titles and states. The fixture verifies four requests versus one.
4. Title-and-label triage: old `label list --json`, raw API PATCH using an input
   file containing `{"title":"Revised"}`, and raw API POST using an input file
   containing `{"labels":[7]}` versus new
   `issue edit 42 --title Revised --add-label bug --json`. Both must send exactly
   the same PATCH and POST payloads and paths. Both make three HTTP requests,
   including metadata lookup, and two HTTP writes. The new workflow reduces
   CLI invocations from three to one, not HTTP writes.

## Scope and limitations

These are synthetic loopback measurements, not live Forgejo benchmarks. They
exclude Internet latency, TLS, authentication negotiation, database query costs,
and real server search behavior. The fixture recognizes the requested search
term and implements only the exercised routes. Timing includes fixture work and
local filesystem capture, so small latency differences are noisy and must not be
read as production guarantees.

All size figures are **bytes**, not tokenizer counts or tokens. HTTP body bytes
exclude headers and request bodies. Stdout bytes include trailing newlines and,
for triage, sum all CLI outputs. A raw API workflow or shell `jq` could already
reduce old output. The listing baseline is the built-in typed workflow, not the
best theoretical old API workflow. The old triage baseline does not charge for
an extra `jq` process or script that selects the label ID from the metadata.

## Measured results

Measured on October 2, 2026, on Linux x86_64, AMD Ryzen 5 7600X,
with Rust/Cargo 1.97.1 and optimized release binaries. The baseline is commit
`25cff9f`. Updated sources were at commit `3a110f8` when built.
The JSON report, including all individual samples, was written to
`/home/dalton/.jcode/scratch/fjx-ab/report.json`.

| Operation | Stdout bytes before → after | Response body bytes before → after | HTTP requests before → after | Median wall ms before → after |
| --- | ---: | ---: | ---: | ---: |
| Scoped issue list help | 2,409 → 661 | 0 → 0 | 0 → 0 | 0.755 → 0.747 |
| 120 issues, projected fields | 557,030 → 7,538 | 562,912 → 562,912 | 4 → 4 | 19.865 → 20.043 |
| Six matching issues, search + fields | 557,030 → 336 | 562,912 → 28,105 | 4 → 1 | 20.556 → 2.835 |
| Title + label triage | 4,979 → 78 | 4,800 → 4,800 | 3 → 3 | 4.094 → 2.306 |

Observed improvements and non-improvements:

- Scoped help emits **72.56% fewer stdout bytes**. Its approximately 1% median
  latency difference is too small to establish a meaningful speedup here.
- Projection emits **98.65% fewer stdout bytes** while preserving all 120
  records' selected fields. HTTP traffic is unchanged. Its median was **0.89%
  slower**, so this run does not show a projection latency improvement.
- Search plus projection emits **99.94% fewer stdout bytes**, reduces response
  body bytes by **95.01%**, and reduces requests by **75%**, from four to one.
  Median wall time was **86.21% lower**. This combines server filtering and
  projection, not an independent projection speedup.
- Triage emits **98.43% fewer stdout bytes** and its median wall time was
  **43.67% lower**. CLI invocations fell from three to one. Both paths made
  three HTTP requests and **two identical HTTP writes**. There was no HTTP
  request or response-body reduction.

Binary SHA-256 values for this run:

```text
before ca40c9c38fb2ba7db1684d5b26255d1ddd0aec782da2a2ebe8ab6f559f330b9d
after  aea34b8d38636a9d6160cf6d121294604561dcad4e7d7d51f42195bf159ed1ab
```

These are observations from one local run of 15 samples per binary and scenario,
not statistical confidence intervals or guaranteed production improvements.
