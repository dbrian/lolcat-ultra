# lolcat-ultra

A Rust implementation of [lolcat](https://github.com/busyloop/lolcat), optimized for performance. Reads from stdin or
files, then prints rainbow-colored text.

This project exists primarily to help me learn performance optimization in Rust.

## Performance

We achieve performance by moving work out of the hot path. At build time we precompute rainbow tables and ANSI sequences to avoid runtime formatting. At runtime we use fixed-point integer math in the hot path (no floating point operations per character), and process lines zero-copy from the read buffer where possible.

### Current ceiling

The benchmark pipes `yes "test line"` through `head -n 10000000` into lolcat-ultra:

```
yes "test line"  0.00s user  2% cpu   0.39s total
head -n 10000000 0.39s user 99% cpu   0.39s total   ← pipeline floor
lolcat-ultra -F  0.22s user 55% cpu   0.40s total
```

lolcat-ultra's **CPU time is 0.22s**, but wall time is 0.40s — it spends roughly half its time blocked waiting for `head` to write to the pipe. The pipeline floor is ~0.39s, set entirely by `head`'s throughput. lolcat-ultra cannot exit before `head` closes the pipe.

The gap between the current 0.396s and the ~0.39s floor is about 6ms (~1.5%). That headroom is essentially noise: it represents the time to drain the last pipe buffer and flush output after `head` exits, not recoverable CPU work. Further optimization of the hot path will not move the benchmark needle.

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

## Benchmarks (M4 Laptop)

```
$ time yes "test line" | head -n 10000000 | lolcat-ultra -F > /dev/null
```

[busyloop/lolcat](https://github.com/busyloop/lolcat) - The original Ruby implementation _(432.34s)_

[ur0/lolcat](https://github.com/ur0/lolcat) - Another Rust port _(21.16s)_

lolcat-ultra - Standard release build _(0.40s)_

lolcat-ultra - PGO optimized build _(0.39s)_
