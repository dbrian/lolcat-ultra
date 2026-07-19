# lolcat-ultra

A Rust implementation of [lolcat](https://github.com/busyloop/lolcat), optimized for performance. Reads from stdin or
files, then prints rainbow-colored text.

This project exists primarily to help me learn performance optimization in Rust.

## Performance

We achieve performance by moving work out of the hot path, then parallelizing what remains:

- **Build time**: the rainbow color table and ANSI escape sequences are precomputed in `build.rs`. TrueColor sequences are stored as fixed-width 20-byte entries (19 content bytes + 1 padding byte) so the hot loop copies them with a single compile-time-sized copy — the following character overwrites the padding byte.
- **Fixed-point phase math**: no floating-point operations anywhere per character or per line. The rainbow phase advances by precomputed u64 increments (per character and per line).
- **Chunk classification**: each input chunk is scanned once with a branchless, vectorizable pass. Chunks that are pure ASCII with no ESC/tab/CR take a fused colorize path with zero per-line dispatch, classification, or bounds checks (`chunks_exact_mut(20)`); everything else falls back to a general per-line path that handles UTF-8, tabs, and embedded ANSI escapes.
- **Parallel pipeline**: the reader thread assembles newline-aligned chunks and fans them out round-robin to worker threads. Each worker colorizes independently — a chunk's starting phase is derived from the running line count — and a writer thread reassembles output in dispatch order. All safe Rust (`unsafe_code = "forbid"`): scoped threads and sync channels.
- **Single-copy output**: workers accumulate colored output in a persistent 1MB buffer with index-based writes; the only copies are lookup table → buffer → fd. (A zero-copy buffer handoff was tried and is *slower*: recycled buffers return cache-cold and every store pays a DRAM round-trip.)

### Why the old pipe benchmark was retired

The original benchmark piped `yes "test line"` through `head -n 10000000` into lolcat-ultra:

```
yes "test line"  0.00s user  2% cpu   0.39s total
head -n 10000000 0.39s user 99% cpu   0.39s total   ← pipeline floor
lolcat-ultra -F  0.22s user 55% cpu   0.40s total
```

That benchmark is **producer-bound**: `head` sets a ~0.39s pipeline floor, so lolcat-ultra spends roughly half its wall time blocked on the pipe and no hot-path optimization can move the number. lolcat-ultra's real cost is its 0.22s of CPU time, which the pipe benchmark hides.

The current benchmark (`make bench`, implemented in `bench/compare.sh`) fixes this by reading pre-generated input files, so the tool under test is the only bottleneck. It measures two corpora — short pure-ASCII lines (the fast path) and mixed multibyte UTF-8 lines (the slow path) — with [hyperfine](https://github.com/sharkdp/hyperfine), and reports throughput (lines/s and MB/s). Normalizing to throughput lets us compare against the original Ruby lolcat on a smaller input slice instead of waiting minutes per iteration.

## Building

**Standard build:**

```bash
cargo build --release
```

**PGO (Profile-Guided Optimization) build:**

> ⚠️ **Not currently recommended.** The profiling workload below is ASCII-only, so PGO
> treats the UTF-8 paths as cold and regresses them badly (measured +60% on the UTF-8
> corpus, and it no longer beats the standard build on ASCII either). Use the standard
> build unless the profile workload is made representative of your input mix.

```bash
make pgo-build
```

Or manually:

```bash
# 1. Install llvm tools
rustup component add llvm-tools-preview

# 2. Build with instrumentation
RUSTFLAGS="-Cprofile-generate=/tmp/pgo-data" cargo build --release

# 3. Run representative workload
yes "test line" | head -n 10000000 | ./target/release/lolcat-ultra -F > /dev/null

# 4. Merge profile data
~/.rustup/toolchains/stable-*/lib/rustlib/*/bin/llvm-profdata merge -o /tmp/pgo-data/merged.profdata /tmp/pgo-data

# 5. Rebuild with optimization
RUSTFLAGS="-Cprofile-use=/tmp/pgo-data/merged.profdata" cargo build --release
```

## Benchmarks (Apple M3 laptop)

File-input throughput via `make bench` (hyperfine, output to `/dev/null`). The
ASCII corpus is 10M lines of `test line`; the UTF-8 corpus is ~2.1M mixed
multibyte lines. Ruby lolcat runs on a 50k-line slice of each and is compared
by throughput.

| command                 | corpus | mean    | Mlines/s | MB/s  |
|-------------------------|--------|---------|----------|-------|
| `cat` (I/O floor)       | ascii  | 0.009s  | 1137.1   | 11371 |
| lolcat-ultra            | ascii  | 0.041s  | 241.9    | 2419  |
| lolcat-ultra            | utf8   | 0.036s  | 58.5     | 2660  |
| Ruby lolcat (busyloop)  | ascii  | 2.210s  | 0.023    | 0.2   |
| Ruby lolcat (busyloop)  | utf8   | 7.269s  | 0.007    | 0.3   |

lolcat-ultra is **~10,700x faster** than the original Ruby
[busyloop/lolcat](https://github.com/busyloop/lolcat) on ASCII input and
**~8,500x faster** on UTF-8 input. (Before the 2026-07 optimization run —
single-threaded, per-line processing — the means were 0.225s and 0.193s;
the run brought them down ~5.3x via the pipeline and fused paths described
above.)

Historical pipe-benchmark numbers (`yes "test line" | head -n 10000000 | lolcat -F > /dev/null`),
kept for reference: busyloop/lolcat _(432.34s)_, [ur0/lolcat](https://github.com/ur0/lolcat)
_(21.16s)_, lolcat-ultra _(0.40s, 0.39s with PGO)_.
