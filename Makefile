.PHONY: build pgo-build pgo-clean pgo-instrument pgo-profile pgo-optimize benchmark bench help

# Default target
help:
	@echo "Available targets:"
	@echo "  build         - Standard release build"
	@echo "  pgo-build     - Full PGO (Profile-Guided Optimization) build"
	@echo "  pgo-clean     - Clean PGO data and cargo artifacts"
	@echo "  pgo-instrument- Build with profiling instrumentation"
	@echo "  pgo-profile   - Run workload to generate profile data"
	@echo "  pgo-optimize  - Build optimized binary using profile data"
	@echo "  benchmark     - Run legacy pipe benchmark (producer-bound; kept for history)"
	@echo "  bench         - Run file-input throughput benchmark vs Ruby lolcat"
	@echo "  clean         - Clean cargo build artifacts"

# Paths
PGO_DATA_DIR := /tmp/pgo-data
MERGED_PROFILE := $(PGO_DATA_DIR)/merged.profdata
LLVM_PROFDATA := $(shell rustc --print sysroot)/lib/rustlib/$(shell rustc -vV | grep host | cut -d' ' -f2)/bin/llvm-profdata

# Standard release build
build:
	cargo build --release

# Full PGO build process
pgo-build: pgo-clean pgo-instrument pgo-profile pgo-optimize
	@echo "PGO build complete! Binary: target/release/lolcat-ultra"

# Clean PGO data and cargo artifacts
pgo-clean:
	@echo "Cleaning PGO data and cargo artifacts..."
	rm -rf $(PGO_DATA_DIR)
	cargo clean

# Step 1: Build with profiling instrumentation
pgo-instrument:
	@echo "Building with profiling instrumentation..."
	RUSTFLAGS="-Cprofile-generate=$(PGO_DATA_DIR)" cargo build --release

# Step 2: Run workload to generate profile data
pgo-profile:
	@echo "Running workload to generate profile data..."
	@mkdir -p $(PGO_DATA_DIR)
	yes "test line" | head -n 10000000 | ./target/release/lolcat-ultra -F > /dev/null
	@echo "Merging profile data..."
	$(LLVM_PROFDATA) merge -o $(MERGED_PROFILE) $(PGO_DATA_DIR)
	@echo "Profile data merged: $(MERGED_PROFILE)"

# Step 3: Build optimized binary using profile data
pgo-optimize:
	@echo "Building with PGO optimization..."
	cargo clean
	RUSTFLAGS="-Cprofile-use=$(MERGED_PROFILE)" cargo build --release

# Legacy pipe benchmark. Producer-bound: `head` caps the pipeline at ~0.39s,
# so lolcat-ultra improvements do not move this number. Use `make bench` instead.
benchmark:
	@echo "Running legacy pipe benchmark (producer-bound)..."
	time yes "test line" | head -n 10000000 | ./target/release/lolcat-ultra -F > /dev/null

# File-input throughput benchmark: measures lolcat-ultra itself (no pipe
# producer) and normalizes to lines/s + MB/s for comparison with Ruby lolcat.
bench: build
	./bench/compare.sh

# Clean cargo artifacts only
clean:
	cargo clean
