# lolcat-ultra

A Rust implementation of [lolcat](https://github.com/busyloop/lolcat), optimized for performance. Reads from stdin or
files, then prints rainbow-colored text.

This project exists primarily to help me learn performance optimization in Rust.

## Performance

We achieve performance by moving work out of the hot path. At build time we precompute rainbow tables and ANSI sequences to avoid runtime formatting. At runtime we use fixed-point integer math in the hot path (no floating point operations per character), and process lines zero-copy from the read buffer where possible.

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

For maximum performance, use Profile-Guided Optimization. This analyzes actual runtime behavior and optimizes the binary accordingly:

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
| `cat` (I/O floor)       | ascii  | 0.010s  | 1048.8   | 10488 |
| lolcat-ultra            | ascii  | 0.237s  | 42.2     | 422   |
| lolcat-ultra            | utf8   | 0.193s  | 10.8     | 493   |
| Ruby lolcat (busyloop)  | ascii  | 2.206s  | 0.023    | 0.2   |
| Ruby lolcat (busyloop)  | utf8   | 7.182s  | 0.007    | 0.3   |

lolcat-ultra is **~1,860x faster** than the original Ruby
[busyloop/lolcat](https://github.com/busyloop/lolcat) on ASCII input and
**~1,560x faster** on UTF-8 input.

Historical pipe-benchmark numbers (`yes "test line" | head -n 10000000 | lolcat -F > /dev/null`),
kept for reference: busyloop/lolcat _(432.34s)_, [ur0/lolcat](https://github.com/ur0/lolcat)
_(21.16s)_, lolcat-ultra _(0.40s, 0.39s with PGO)_.
