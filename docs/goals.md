# Goals

Two, both enforced by the build rather than by intent.

## Never crashes

Model files arrive from strangers. Ayatsuri2D treats every one as hostile.

- Every offset is bounds-checked against the file length before anything is
  dereferenced.
- Every section is checked for `count * element_size <= section size` before it
  is read.
- Index fields are read as signed, because `-1` means "none" and reading it
  unsigned produces 4294967295.
- String reads are bounded to the record width rather than trusting a
  terminator to appear.
- The runtime path forbids `unwrap`, `expect`, `panic!` and raw indexing by
  lint. Errors are typed and recoverable.
- The loader is fuzzed in CI, seeded with a known-malicious model.

The reference implementation does none of this, which is CVE-2023-27566: a
model file is an arbitrary memory write.

## Never lags

Not "fast". **Predictable.** A stream at a steady 45fps looks better than 60fps
that hitches every few seconds, because perception is tuned to outliers.

    Runtime A:  avg 4ms,  p99.9 = 38ms    feels broken
    Runtime B:  avg 9ms,  p99.9 = 10ms    feels perfect

So frame cost must be knowable in advance:

- **Allocation happens at load.** The frame loop allocates nothing.
- **No unbounded work.** Physics iterations, mask passes and draw sorting are
  all bounded when the model is loaded.
- **Pipelines are built at load**, never mid-frame. A shader compile is a
  10-100ms stall.
- **No GPU readback in the frame path.**
- **Structure-of-arrays, linear passes**, so memory access is predictable.

Benchmarks assert **p99.9**, not the mean, and a regression fails the build. A
six-hour soak test watches for drift, because a leak shows up at hour four and
people stream for eight.
