# lolcat-ultra

A Rust implementation of [lolcat](https://github.com/busyloop/lolcat), optimized for performance. Reads from stdin or
files, then prints rainbow-colored text.

This project exists primarily to help me learn performance optimization in Rust.

## Performance

The hot path, the character-processing loop, has:

- Zero allocations
- Zero runtime initialization
- All lookups from static tables (precomputed during build)
- Stack-allocated buffers only

## Benchmarks (M4 Laptop)

```
$ time yes "test line" | head -n 10000000 | lolcat-ultra -F > /dev/null
```

[busyloop/lolcat](https://github.com/busyloop/lolcat) - The original Ruby implementation _(432.34s)_

[ur0/lolcat](https://github.com/ur0/lolcat) - Another Rust port _(21.16s)_

lolcat-ultra _(0.86s)_
