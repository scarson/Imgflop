# Imgflop
A local Imgflip clone. This is not a serious project.

## Local Development

### Prerequisites
- Rust toolchain (stable)
- Docker Desktop (for `rust.testcontainers`-based tests where applicable)

### Build
```bash
cargo build
```

### Test
```bash
cargo test
```

### Quality Gates
```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 80
```
