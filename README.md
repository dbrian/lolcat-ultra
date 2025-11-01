# lolcat-ultra

A Rust implementation of [lolcat](https://github.com/busyloop/lolcat), optimized for performance. Reads from stdin or
files, then prints rainbow-colored text.

This project exists primarily to help me learn performance optimization in Rust.

## Performance

We achive performance by moving work out of the hot path. At build time we precompute rainbow tables and ANSI sequences to avoid runtime formatting. At runtime we use fixed-point integer math (no floating point operations).

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

lolcat-ultra - Standard release build _(0.77s)_

lolcat-ultra - PGO optimized build _(0.71s)_
