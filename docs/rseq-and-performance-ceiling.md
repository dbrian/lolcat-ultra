# rseq applicability & the performance ceiling

*Analysis date: 2026-05-31. Machine: Apple M3, macOS (Darwin 25.3.0, arm64), 8 cores.*

This note answers two questions:

1. Do the techniques in Justine Tunney's [restartable sequences (rseq) write-up](https://justine.lol/rseq/) apply to lolcat-ultra?
2. Is there any recoverable performance left, and if so, where?

The findings below were produced by mapping each rseq technique against the
codebase, re-measuring the benchmark per pipeline stage on the actual machine,
and adversarially trying to refute every candidate optimization.

---

## TL;DR

- **rseq does not apply** — on four independent, individually-fatal grounds.
- The **default benchmark is producer-bound**: `yes | head` is the slowest
  stage (~0.37s), and the full pipeline (~0.39s) sits right on that floor. No
  change to lolcat can move that number. The README's operative claim is
  correct.
- lolcat *itself* is **CPU-bound**, not I/O-bound. In isolation it does its
  whole job in ~0.22s wall ≈ 0.21s user. That CPU is real and reclaimable —
  it just never surfaces in the producer-gated pipe benchmark.
- One genuine, safe, portable hot-path win exists (**fuse the per-char ANSI
  write + data byte into a single store**, ~7–10% CPU on file input). It helps
  file/non-pipe workloads only; it is invisible to the official benchmark.
- Two documentation figures are wrong: output volume is **~1.81 GB**, not
  ~1.35 GB; the bench machine reports **M3**, not M4.

---

## 1. rseq does not apply

rseq exists to eliminate **cacheline contention when many threads on many
cores hammer shared per-CPU state** — per-CPU malloc freelists
(tcmalloc/jemalloc/cosmopolitan), lock-free per-CPU counters, and ~1 ns
`cpu_id` reads replacing the ~1 µs `sched_getcpu` syscall. Every one of those
wins presupposes properties lolcat-ultra does not have. Each row below is
independently fatal; all four hold:

| Wall | Why it blocks rseq |
|---|---|
| **Single-threaded** | No threads, locks, atomics, or shared mutable state anywhere in `src/` (grep-confirmed: no `std::thread`/`spawn`/`rayon`/`Mutex`/`RwLock`/`Atomic`/`Arc`). There is no contention to remove — the entire premise of rseq is absent. |
| **No malloc in the hot path** | Per-line work uses a stack `ArrayVec<u8,8192>` (`processor.rs:112`); all rainbow/ANSI tables are `'static`, generated in `build.rs`. The only heap allocs are a one-time 256 KB `BufWriter` and a rarely-used `line_buf`. A per-CPU allocator cannot speed up code that never allocates. |
| **macOS, not Linux** | rseq is a Linux 4.18+ kernel ABI. The bench machine is Darwin/arm64; the syscall does not exist. |
| **`unsafe_code = "forbid"`** | rseq requires hand-written x86-64/arm64 assembly carrying a magic `RSEQ_SIG` word. Impossible under the crate's lint (CI-enforced, alongside `warnings = "deny"`). |

Technique-by-technique, every item in the article maps to *does-not-apply*:

| rseq technique | Verdict | Reason |
|---|---|---|
| Per-CPU freelist malloc | ✗ | No heap allocation in the hot path; nothing to accelerate. |
| Per-CPU counters | ✗ | The only counters (`lines_read`, `phase`, `last_color_idx`) are single-threaded stack locals — already maximal speed. |
| Fast `cpu_id` read (vs `sched_getcpu`) | ✗ | Code never queries the CPU id; no per-CPU structure to index. |
| Per-CPU sharding (anti false-sharing) | ✗ | One thread, zero shared state — no cacheline to shard. |
| `membarrier()` cross-CPU sync | ✗ | No other threads to barrier, no inter-thread ordering to enforce. |
| The abort-handler critical-section mechanism | ✗ | Triple-blocked: macOS, `unsafe`-forbidden, and nothing concurrent to protect. |

The same reasoning rules out the broader Justine systems toolkit for this
program: `splice`/`vmsplice`/`tee`, `io_uring`, and `F_SETPIPE_SZ` are all
Linux-only, and several are structurally impossible for a byte-*transforming*
filter — you cannot splice input bytes through untouched when a 19-byte escape
must be inserted before every character.

---

## 2. The benchmark is producer-bound (the ceiling is real)

Per-stage timing of the benchmark, N = 10,000,000 lines, 3× medians:

| Measurement | Command | real | user | sys |
|---|---|---:|---:|---:|
| **Producer floor** | `yes "test line" \| head -n 10M > /dev/null` | **0.37s** | 0.37 | 0.01 |
| **Full pipeline** | `yes "test line" \| head -n 10M \| lolcat-ultra -F > /dev/null` | **0.39s** | — | — |
| **lolcat isolation** | `lolcat-ultra -F < file > /dev/null` | **0.22s** | 0.21 | ~0.00 |
| Fast producer | `cat file \| lolcat-ultra -F > /dev/null` | 0.22s | 0.21 | 0.02 |
| Control (fast consumer) | `yes "test line" \| head -n 10M \| cat > /dev/null` | ~0.39s | — | — |
| File read alone | `cat file > /dev/null` | 0.01s | — | — |

What this proves:

- `yes | head` is CPU-bound at ~0.37s and is the **slowest stage**. The full
  pipeline lands ~0.02s above it (last pipe-buffer drain + final flush).
- Swapping the slow producer for a fast one (`cat`) collapses the pipeline to
  **lolcat's own 0.22s** — lolcat is faster than the producer can feed it.
- The control (`yes|head|cat`, an effectively instant consumer) **still takes
  ~0.39s**. An arbitrarily fast consumer cannot lower the wall time. The floor
  is the producer, full stop.

**Therefore: no hot-path optimization — SIMD, fewer branches, PGO, better
batching — can move the official benchmark's wall time.** This confirms the
README's operative claim ("further optimization of the hot path will not move
the benchmark needle"). Nothing survived adversarial verification as a
benchmark-mover.

### Precision: lolcat is CPU-bound, not I/O-bound

The README phrasing "lolcat is I/O bound" is slightly misleading as a property
of lolcat. In isolation — reading a file, writing to `/dev/null` — lolcat is
**fully CPU-bound**: 0.22s wall ≈ 0.21s user, sys ≈ 0. It only *appears*
I/O-bound inside the `yes|head` pipeline because it idles ~40% of its wall
blocked on the pipe waiting for a slower upstream. That is *producer-bound*,
not I/O-bound. The distinction matters: it means lolcat has ~0.21s of genuine,
reclaimable CPU work that is simply hidden in this particular benchmark.

Throughput in isolation: ~454 MB/s in, ~8.2 GB/s out, 45.5M lines/s,
~539 instructions/line (~5.39B instructions for the run).

---

## 3. The one real lever

Because lolcat is CPU-bound whenever it is *not* gated by `yes|head` (file
input, `cat file | lolcat`, or any fast source), the ~0.21s of CPU is a real
target. Most micro-rewrites regressed when measured:

| Attempt | Result |
|---|---|
| `for &b in bytes` to drop the index bounds check | **+5% (worse)** — the `i < len` guard already lets LLVM elide the check; the iterator perturbed register allocation. |
| `usize::MAX` sentinel instead of `Option<usize>` | **+10% (worse)** — `Option<usize>` already niche-optimizes to a plain integer compare. |
| `target-cpu=native` | **~12–15% CPU regression** on this M3 — native tuning/auto-vectorization pessimizes the fixed-size byte-copy loop. |
| Larger `BufReader` for file input (8 KB → 256 KB) | **No real win** — the apparent ~5% was a cold-vs-warm page-cache artifact. Harmless consistency cleanup at most. |

One change held up under measurement:

### Fuse the ANSI sequence + data byte into a single store

Today the hot loop emits color and data as two operations with two capacity
checks: `try_extend_from_slice(ansi_19)` then `buf.push(byte)`
(`write_ansi_truecolor`, `processor.rs:29-32`; the table is already
`[[u8;20];2048]` — 19 content bytes + 1 pad — in `build.rs`). Building a
`[u8;20] = ansi(19) ++ char` and doing **one** `try_extend_from_slice`
collapses the two capacity checks into one.

- **Achievable in safe Rust** (no `unsafe` needed); all 11 tests pass.
- Measured **~7–10% user-time reduction** on file input (~0.20s → ~0.18–0.19s)
  via interleaved A/B.
- **Zero effect on the official pipe benchmark** (still 0.39s — producer-gated).

Caveats and scope:
- Treat this as a **promising lead, not a settled win** — it was measured with
  wall-clock A/B, not the criterion harness, and exploratory agents partly
  disagreed about its magnitude. Confirm with `cargo bench` before keeping.
- The fusion applies only when a color is actually emitted. With the
  `last_color_idx` dedup, runs that reuse the previous color still push the lone
  byte; the fused store is for the color-change case (which, for the default
  config, is every character).
- It improves lolcat's standalone throughput — arguably the more honest "how
  fast is lolcat" number — but by construction it cannot move the
  `yes|head|...` benchmark.

---

## 4. Techniques considered and rejected

| Technique | Verdict | Reason |
|---|---|---|
| `splice` / `vmsplice` / `tee` | ✗ | Linux-only; input must be transformed (can't pass bytes through); output is `/dev/null` (copy already free). |
| `io_uring` | ✗ | Linux-only, needs `unsafe`; only ~6,900 `write()` syscalls already (256 KB `BufWriter`) — syscall overhead is noise. |
| SIMD (NEON) on the color/copy loop | ✗ | `std::simd` is nightly; `core::arch` needs `unsafe`. Loop is memory-bandwidth-bound on 1.81 GB output, not ALU-bound; and it's off the benchmark critical path. |
| Producer/consumer threading | ✗ | The producer is the bottleneck — a reader thread can't make `head` faster. Reintroduces the exact synchronization rseq fights. |
| Huge pages / `madvise` / `MAP_POPULATE` | ✗ | Tiny working set, no large mmap'd heap; Linux concepts. |
| `F_SETPIPE_SZ` | ✗ | Linux-only; can't raise the producer's steady-state rate; lolcat doesn't own the pipe. |
| Larger read/write buffers | ✗ | Already 256 KB both sides; `BufWriter` 1 MB was tried and discarded historically. |
| `writev` / gather I/O | ✗ | A per-char iovec is *more* overhead than the current contiguous-buffer-then-batched-write. |
| Reducing output volume | ✗ | Every char gets a unique truecolor escape by design; `last_color_idx` dedup already never fires for the default config. No lossless reduction exists; `-F` forces TrueColor. |
| `opt-level`/`LTO`/`codegen-units`/`panic=abort` | ✗ | Already at the strongest settings. |
| PGO | ~ | Already wired up (`make pgo-build`); only reshapes the hidden 0.22s CPU — the README's 0.40→0.39 is within noise. |
| BOLT | ✗ | No `llvm-bolt`/`perf` on macOS; the binary is Mach-O (BOLT targets ELF); redundant with LTO+PGO on one tiny inlined loop. |

---

## 5. Documentation corrections found along the way

- **Output volume is ~1.81 GB, not ~1.35 GB.**
  `lolcat-ultra -F < 10M-line-file | wc -c` = 1,810,000,014 bytes (~181 B/line:
  9 chars × ~19-byte truecolor escapes + framing). The README and `program.md`
  both state ~1.35 GB.
- **The bench machine reports Apple M3, not M4** (`machdep.cpu.brand_string`).
  The README and `program.md` say M4 — possibly a second machine or a stale doc.

---

## 6. Recommendation

For the headline `yes | head | lolcat -F > /dev/null` benchmark, the program is
**at its floor** and rseq is categorically inapplicable. If the goal shifts to
lolcat's *standalone* throughput (file/non-pipe input, where lolcat is the
actual bottleneck), the fused-store change is the one safe, portable lever worth
landing — pending criterion confirmation. The two doc figures above are worth
correcting regardless.
