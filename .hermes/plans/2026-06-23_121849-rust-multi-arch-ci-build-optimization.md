# Rust Multi-Arch CI Build Optimization Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Eliminate the 40-minute cold Docker build on every PR by fixing the broken cache architecture and adopting the fastest known multi-arch Rust build strategy.

**Architecture:** The current Dockerfile uses `cargo-chef` + `--mount=type=cache`, but the mount-type cache is ephemeral on GitHub Actions runners and invisible to the `type=registry` layer cache export. This means compiled dependencies are thrown away after every run. The fix is a two-pronged approach: (1) switch the Dockerfile to produce real Docker layers for the dependency compilation (so `type=registry` cache captures them), and (2) adopt `sccache` for cross-run compilation caching that works both inside Docker and for native test builds.

**Tech Stack:** Rust 1.93, Docker buildx, GitHub Actions (native amd64 + arm64 runners), cargo-chef, sccache, GHCR registry-scoped cache

---

## Current State Analysis

### The Problem (Confirmed)

Every Docker build — PR AND master — takes ~40 minutes per arch. The cache is **never** warm. This is a structural bug, not a configuration issue.

**Root cause:** The Dockerfile mixes two incompatible caching mechanisms:

```
# Dockerfile (current — BROKEN caching)
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo chef cook --release --recipe-path recipe.json
```

- `--mount=type=cache` creates BuildKit volumes that are **ephemeral** on GitHub Actions — destroyed after the job finishes
- `type=registry` cache export (in the workflow) captures Docker **image layers** only — it does NOT see `--mount=type=cache` volumes
- Result: the `cargo chef cook` output (compiled dependencies, ~4-5 GB for duckdb-sys) is computed fresh every run and never cached

**Evidence:**
- Last 5 post-merge (master) runs: all 38-41 minutes
- Last PR docker builds: amd64=39min, arm64=32min
- Total wasted compute per PR: ~71 minutes of CI time on Docker alone

### Current Setup

```
Dockerfile stages:
  1. admin-ui-builder  (Node.js — fast, ~2 min, properly cached)
  2. chef              (cargo-chef install)
  3. planner           (cargo chef prepare — recipe.json)
  4. builder           (cargo chef cook + cargo build --release)
  5. runtime           (debian-slim + binary)

Workflow (pr-integration.yml):
  - matrix: amd64 on ubuntu-latest, arm64 on ubuntu-24.04-arm (native runners)
  - cache-from: type=registry,ref=...-cache:buildcache-{arch} + buildcache-master-{arch}
  - cache-to: type=registry,ref=...-cache:buildcache-{arch},mode=max
  - build-args: RUST_VERSION=1.93
```

---

## Proposed Approach

### Strategy: Fix Docker layer caching + add sccache

**Tier 1 (highest impact): Fix the Dockerfile to produce cacheable layers**

Remove `--mount=type=cache` from the `cargo chef cook` step so the compiled dependencies become a real Docker layer that `type=registry` cache captures. Keep `--mount=type=cache` only for the `cargo build` step (application code changes frequently; dependencies don't).

**Tier 2 (cross-run compilation cache): Add sccache**

Install sccache in the builder stage and configure `RUSTC_WRAPPER=sccache`. sccache caches compilation artifacts by content hash. Use GitHub Actions cache backend so it persists across runs. This helps both Docker builds and native test builds.

**Tier 3 (build parallelism): mold linker**

Install the `mold` linker in the builder stage. It's 10x faster than `ld` for large Rust projects (parallel linking). Saves 30-60 seconds on the final link step.

**Tier 4 (future): Consider cargo-zigbuild for cross-compilation**

Instead of building on native arm64 runners, cross-compile arm64 on amd64 using cargo-zigbuild (zig as the C toolchain). This eliminates the arm64 runner entirely but requires C dependency cross-compilation setup (duckdb-sys is the challenge). **Defer — native arm64 runners work fine once caching is fixed.**

### goreleaser + zigbuild?

goreleaser is Go-specific (it orchestrates Go cross-compilation, not Rust). `cargo-zigbuild` is the Rust equivalent and can be used standalone or as a cargo subcommand. It's not integrated with goreleaser. Skip goreleaser entirely.

---

## Step-by-Step Plan

### Task 1: Remove --mount=type=cache from cargo chef cook (make deps a real layer)

**Objective:** The `cargo chef cook` output (compiled dependencies) must become a Docker image layer so `type=registry` cache captures it.

**Files:**
- Modify: `Dockerfile:34-36`

**Step 1: Edit the Dockerfile — remove mount from chef cook**

```dockerfile
# BEFORE (broken — ephemeral, never cached in registry):
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo chef cook --release --recipe-path recipe.json

# AFTER (fixed — produces a real Docker layer cached by type=registry):
RUN cargo chef cook --release --recipe-path recipe.json
```

**Step 2: Keep --mount=type=cache on the cargo build step (application code)**

```dockerfile
# This stays the same — app code changes every commit, deps don't.
# The mount cache only helps within a single build (avoids re-fetching crates),
# and the actual dependency compilation is already in the chef cook layer above.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo build --release --package aeterna \
    && cp /app/target/release/aeterna /app/aeterna-bin
```

**Step 3: Verify locally**

```bash
docker build --progress=plain -t aeterna-test . 2>&1 | grep -E 'chef cook|cargo build|DONE'
# First build: chef cook produces a layer (~8-10 min)
# Second build (no Cargo.lock change): chef cook layer is CACHED (instant)
```

**Step 4: Commit**

```bash
git add Dockerfile
git commit -m "fix(docker): make cargo-chef dependency layer cacheable via type=registry

Remove --mount=type=cache from the 'cargo chef cook' step so the compiled
dependencies become a real Docker image layer. The type=registry cache
export only captures layers, not mount-type cache volumes, so the previous
setup discarded ~5GB of compiled dependencies after every CI run."
```

---

### Task 2: Add sccache to the builder stage

**Objective:** Add sccache as RUSTC_WRAPPER so compilation artifacts are cached by content hash across runs.

**Files:**
- Modify: `Dockerfile:18-36` (builder/chef stage)

**Step 1: Install sccache in the chef (base) stage**

```dockerfile
FROM rust:${RUST_VERSION}-bookworm AS chef
RUN cargo install cargo-chef sccache
ENV RUSTC_WRAPPER=sccache
ENV SCCACHE_DIR=/sccache
WORKDIR /app
```

**Step 2: Add sccache mount to cook and build steps**

```dockerfile
FROM chef AS builder
ARG BUILD_DATE
ARG VCS_REF

COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,target=/sccache,sharing=locked \
    cargo chef cook --release --recipe-path recipe.json

COPY . .
RUN --mount=type=cache,target=/sccache,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo build --release --package aeterna \
    && cp /app/target/release/aeterna /app/aeterna-bin
```

Wait — this re-introduces the same problem. sccache with `--mount=type=cache` is also ephemeral on CI.

**Revised approach:** Don't use sccache inside Docker. The Docker layer cache (fixed in Task 1) already handles cross-run caching for Docker builds. sccache is better used for **native builds** (cargo test, cargo check) where there's no Docker layer cache.

**Skip this task for Docker. Use sccache only for native test workflows (Task 4).**

---

### Task 2 (revised): Add the mold linker to the builder stage

**Objective:** Use the mold linker for faster final linking (saves 30-60s on the `cargo build --release` step).

**Files:**
- Modify: `Dockerfile:16-18`

**Step 1: Install mold in the chef base stage**

```dockerfile
FROM rust:${RUST_VERSION}-bookworm AS chef
RUN apt-get update && apt-get install -y --no-install-recommends mold && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef
WORKDIR /app
```

**Step 2: Configure rustc to use mold**

Add `.cargo/config.toml` at the repo root (or use RUSTFLAGS):

```dockerfile
ENV RUSTFLAGS="-C link-arg=-fuse-ld=mold"
```

Or create `.cargo/config.toml`:

```toml
[target.x86_64-unknown-linux-gnu]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=mold"]
```

**Step 3: Verify locally**

```bash
docker build --progress=plain -t aeterna-test . 2>&1 | grep -E 'link|mold|cargo build'
```

**Step 4: Commit**

```bash
git add Dockerfile .cargo/config.toml
git commit -m "perf(docker): use mold linker for faster Rust linking"
```

---

### Task 3: Seed the master cache after the Dockerfile fix

**Objective:** After fixing the Dockerfile, trigger a post-merge run to seed the `buildcache-master-{arch}` refs in the new GHCR namespace.

**Files:**
- No file changes — operational step

**Step 1: Merge a trivial change to master to trigger post-merge**

After the Dockerfile fix is merged to master, post-merge.yml runs automatically. Verify it takes ~40 min for the first run (cold) but writes the cache.

**Step 2: Verify the second run is faster**

Push another trivial commit (or re-run the workflow). The second run should take ~10-15 min (warm dependency layer cache).

**Step 3: Verify PR builds use the warm cache**

Open a PR. The docker-build job should complete in ~10-15 min instead of ~40 min.

**Expected result:**
- First master build after fix: ~40 min (cold — unavoidable)
- Second master build: ~10-15 min (warm deps layer)
- PR builds: ~10-15 min (warm deps layer from master cache)

---

### Task 4: Add sccache to native test workflows

**Objective:** Use sccache for `cargo test` / `cargo check` workflows that run on the host (not in Docker). Swatinem/rust-cache caches the cargo registry + git, but NOT the compiled artifacts in target/. sccache caches the actual `.rlib` outputs by content hash.

**Files:**
- Modify: `.github/workflows/pr-fast.yml` (unit tests job)
- Modify: `.github/workflows/pr-integration.yml` (shipped-smoke job)

**Step 1: Add sccache to the unit-tests job**

```yaml
      - name: Install sccache
        uses: taiki-e/install-action@v2
        with:
          tool: sccache

      - name: Configure sccache
        run: |
          echo "RUSTC_WRAPPER=sccache" >> $GITHUB_ENV
          echo "SCCACHE_GHA_ENABLED=true" >> $GITHUB_ENV

      - uses: actions/cache@v4
        with:
          path: /home/runner/.cache/sccache
          key: sccache-${{ runner.os }}-${{ matrix.arch }}-${{ hashFiles('**/Cargo.lock') }}
          restore-keys: |
            sccache-${{ runner.os }}-${{ matrix.arch }}-
```

**Step 2: Place before the cargo build/test step**

The sccache setup must come AFTER `dtolnay/rust-toolchain` but BEFORE any `cargo` command.

**Step 3: Verify in CI**

The first run populates the cache. The second run should show sccache hit rate > 80% for dependency crates.

**Step 4: Commit**

```bash
git add .github/workflows/pr-fast.yml .github/workflows/pr-integration.yml
git commit -m "perf(ci): add sccache for native Rust test builds"
```

---

### Task 5: Split docker-build to run only on merge, not every PR

**Objective:** The Docker multi-arch build is the single most expensive CI job (~70 min total compute). It doesn't need to run on every PR commit — clippy + unit tests + the new e2e-system workflow already gate code quality. Move Docker builds to a separate, lighter trigger.

**Files:**
- Modify: `.github/workflows/pr-integration.yml`
- Create: `.github/workflows/docker-build.yml` (standalone)

**Step 1: Remove docker-build + docker-manifest jobs from pr-integration.yml**

Delete the `docker-build` and `docker-manifest` jobs from `pr-integration.yml`. Keep `shipped-smoke`.

**Step 2: Create docker-build.yml**

```yaml
name: Docker Build

on:
  pull_request:
    types: [labeled]           # Only when 'docker-build' label is added
    branches: [master]
  push:
    branches: [master]          # Always on master (seeds cache)
  workflow_dispatch:            # Manual trigger

jobs:
  docker-build:
    name: docker-build-${{ matrix.arch }}
    # ... (same matrix + build config as before)
```

**Step 3: Verify**

PRs without the label skip Docker entirely (~40 min saved per PR). Adding the `docker-build` label triggers it. Master pushes always build.

**Step 4: Commit**

```bash
git add .github/workflows/pr-integration.yml .github/workflows/docker-build.yml
git commit -m "ci: move docker build to label-triggered workflow, not every PR"
```

---

### Task 6: Add cargo-zigbuild cross-compilation as an experiment

**Objective:** Test whether cross-compiling arm64 on an amd64 runner is faster than using a native arm64 runner. If yes, it eliminates the arm64 runner wait time (arm64 runners sometimes have queue delays).

**Files:**
- Create: `.github/workflows/docker-build-zigbuild.yml` (experimental)
- Modify: `Dockerfile` (add cross-compile target support)

**Step 1: Install cargo-zigbuild**

```yaml
      - name: Install cargo-zigbuild
        uses: taiki-e/install-action@v2
        with:
          tool: cargo-zigbuild

      - name: Install zig
        run: |
          wget -q https://ziglang.org/download/0.13.0/zig-linux-x86_64-0.13.0.tar.xz
          tar xf zig-linux-x86_64-0.13.0.tar.xz
          echo "$PWD/zig-linux-x86_64-0.13.0" >> $GITHUB_PATH
```

**Step 2: Cross-compile**

```bash
cargo zigbuild --release --target aarch64-unknown-linux-gnu --package aeterna
```

**Step 3: Challenge: duckdb-sys C dependency**

duckdb-sys uses `cc` crate to compile libduckdb. Cross-compiling C requires a cross sysroot. Options:
- `cross` tool (Docker-based, handles sysroots)
- Manual `aarch64-linux-gnu-gcc` + sysroot installation
- `zig cc` as the C compiler (zig bundles cross sysroots)

Test with `zig cc`:

```bash
export CC_aarch64_unknown_linux_gnu="zig cc -target aarch64-linux-gnu"
export CXX_aarch64_unknown_linux_gnu="zig c++ -target aarch64-linux-gnu"
cargo zigbuild --release --target aarch64-unknown-linux-gnu --package aeterna
```

**Step 4: Measure timing**

If the cross-compile on a single amd64 runner is faster than running two native builds in parallel, adopt it. If duckdb-sys cross-compilation is problematic, keep native arm64 runners.

**Step 5: Commit (or abandon if duckdb-sys blocks it)**

```bash
git add .github/workflows/docker-build-zigbuild.yml
git commit -m "experiment: cargo-zigbuild cross-compilation for arm64"
```

---

## Summary of Expected Impact

| Change | Current | Expected | Saved |
|---|---|---|---|
| Fix Dockerfile caching (Task 1) | 40 min/PR | 10-15 min/PR | **25-30 min/PR per arch** |
| mold linker (Task 2) | (included above) | -1 min link | ~1 min |
| sccache for native tests (Task 4) | 8 min unit tests | 3-4 min | ~4-5 min |
| Docker on label only (Task 5) | 70 min total/PR | 0 min (no label) | **70 min/PR when skipped** |
| cargo-zigbuild (Task 6, experimental) | 2 parallel runners | 1 runner | runner queue time |

**Total impact:** PR cycle time drops from ~45 min (gated by Docker) to ~10 min (gated by tests). Docker builds run on-demand via label or automatically on master merge.

## Research Findings (confirmed by domain analysis)

The research agent's report confirms the root cause and adds these key insights:

### The winning architecture: Build OUTSIDE Docker

The single most impactful change is **Option C** from the research: stop compiling Rust inside Docker entirely. Instead:
1. Build the binary on the GHA runner using `Swatinem/rust-cache` (which already works for test workflows)
2. COPY the pre-built binary into a trivial runtime-only Dockerfile

This eliminates all the `--mount=type=cache` / `type=registry` / cargo-chef complexity. Swatinem/rust-cache handles caching perfectly on ephemeral runners because it uses GitHub's blob storage (not BuildKit volumes).

### Profile tuning (quick wins, zero risk)

```toml
# Cargo.toml — add/improve release profile
[profile.release]
debug = 0           # No debug info — faster compile, smaller binary
lto = "thin"        # Faster than "fat", good optimization
codegen-units = 16  # More parallelism during codegen
strip = true        # Strip symbols from binary
```

Set `CARGO_INCREMENTAL=0` in CI — incremental compilation adds overhead and provides zero benefit without persistent target dirs.

### mold vs lld per arch

- **AMD64:** use `mold` linker (10x faster than ld)
- **ARM64:** use `lld` linker (mold's aarch64 support is less mature; lld is solid)

### sccache with S3 backend (for the Docker-build approach)

If we keep building inside Docker, sccache with an S3 backend (not GHA cache — 10GB limit is too small for 900 crates + duckdb-sys) is the only way to get persistent compilation caching inside Docker builds. The S3 bucket is shared across all PRs and branches.

### type=gha vs type=registry

The research recommends `type=gha` over `type=registry` for simpler setup (no registry auth for caching) and native GHA integration. But the 10GB limit is a concern for our layer sizes. If we adopt Option C (build outside Docker), this becomes moot — the runtime Dockerfile is tiny.

---

## Revised Priority (incorporating research)

| # | Action | Impact | Effort | Risk |
|---|--------|--------|--------|------|
| 1 | **Fix Dockerfile: remove --mount=type=cache from cook** | 40→15 min | 1 line | None |
| 2 | **Profile tuning: debug=0, lto=thin, codegen-units=16, CARGO_INCREMENTAL=0** | 15→12 min | Trivial | None |
| 3 | **Add mold (amd64) / lld (arm64) linker** | 12→10 min | Low | Low |
| 4 | **Move build outside Docker (Swatinem + COPY binary)** | 10→6 min | Medium | Medium |
| 5 | **Docker on label-only, not every PR** | 70→0 min/PR | Low | None |
| 6 | **sccache with S3 backend** (if staying in Docker) | 10→6 min | Medium | Low |

---

## Risks

1. **cargo-chef layer invalidation** — any change to `Cargo.toml` (adding/removing a dependency) invalidates the chef cook layer and triggers a full recompile. This is expected and unavoidable. The fix ensures it only happens on dependency changes, not on every commit.

2. **sccache disk usage** — sccache can grow to several GB. The GitHub Actions cache has a 10 GB limit per repo. Use S3 backend if staying in Docker, or Swatinem/rust-cache (which handles eviction gracefully) if building outside Docker.

3. **mold on arm64** — mold's arm64 support should be verified. If it doesn't work, use `lld` as the fallback (`-C link-arg=-fuse-ld=lld`).

4. **cargo-zigbuild + duckdb-sys** — the C extension may not cross-compile cleanly. This is why Task 6 is experimental and can be abandoned without blocking the other improvements.

5. **Build-outside-Docker architecture change** — splitting the Rust build from the Docker build adds a job and changes the CI topology. The runtime Dockerfile becomes trivial (COPY binary + admin-ui assets), but the workflow structure changes. Medium risk, high reward.

6. **Profile tuning** — `lto=thin` may slightly reduce runtime performance vs `lto=fat`. `debug=0` means no stack traces with line numbers in release (but `RUST_BACKTRACE` still works with function names). Acceptable tradeoffs for CI speed.
