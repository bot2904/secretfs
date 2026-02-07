# AGENTS.md

Instructions for AI agents working on this repository.

## Project Overview

**secretfs** is a FUSE passthrough filesystem written in Rust that transparently replaces secrets with placeholders on read and restores them on write. It is designed for mounting directories with sensitive content into Docker containers.

Read [`README.md`](README.md) for usage and [`DESIGN.md`](DESIGN.md) for architecture and design decisions before making changes.

## Repository Structure

```
src/
  main.rs          CLI entrypoint, arg parsing, mount
  config.rs        YAML config parsing and validation
  mapper.rs        Bidirectional secret↔placeholder mapping (thread-safe)
  transformer.rs   Content transformation engine (read/write filters)
  handle.rs        File handle state, lifecycle, write-back
  fs.rs            FUSE filesystem implementation (fuser::Filesystem trait)
```

## Build & Test

```bash
cargo build          # Debug build
cargo build --release   # Release build
cargo test           # Run all unit tests
RUST_LOG=debug cargo run -- --source /tmp/src --mount /tmp/mnt --config secrets.yaml
```

**System dependencies:** `libfuse3-dev`, `fuse3`, `pkg-config`, `build-essential` (Debian/Ubuntu).

## Key Design Constraints

- **Round-trip fidelity is critical.** `filter_read` followed by `filter_write` must produce the exact original bytes. Every transformation change must preserve this property. Write tests for it.
- **Placeholder format is `<|SECRET:XXXX|>`** (hex, zero-padded). Do not change this without updating escape logic, mapper, and all tests.
- **Longest match first** for literal secrets. The `literals` vec in `Transformer` is sorted by length descending. Maintain this invariant.
- **Pre-existing placeholders must be escaped.** `<|SECRET:...|>` in source content becomes `<|ESCAPED_SECRET:...|>` on read and is unescaped on write. This prevents false positives during write-back.
- **Inode cache must be kept consistent.** Every operation that creates, deletes, or moves a file must update the inode cache in `fs.rs`. Forgetting this causes `ENOENT` on subsequent operations.
- **DIRECT_IO is required.** Do not remove the `FOPEN_DIRECT_IO` flag. It prevents the kernel from caching transformed content, which would break consistency.

## Making Changes

### Adding a new FUSE operation

1. Implement the method on `impl Filesystem for SecretFs` in `fs.rs`
2. Use `self.real_path(ino)` to resolve the inode to a real path
3. For file-creating operations: call `self.cache_inode(ino, path)` after creation
4. For file-removing operations: call `self.uncache_inode(ino)` before removal
5. For file-renaming operations: update the cache with the new path

### Adding a new secret source

1. Add a new variant to `SecretDef` in `config.rs`
2. Handle it in `Config::load` validation
3. Handle it in `Transformer::new` — either pre-register (like literals) or add to a pattern list

### Changing the placeholder format

1. Update `PLACEHOLDER_PREFIX` / format string in `mapper.rs`
2. Update `PLACEHOLDER_PREFIX` and `ESCAPE_MARKER` in `transformer.rs`
3. Update all tests that assert on placeholder format
4. Verify round-trip tests still pass

### Adding transformation logic

1. Make changes in `transformer.rs`
2. Both `filter_read_str` and `filter_write_str` must be updated symmetrically
3. Binary variants (`filter_read_binary` / `filter_write_binary`) delegate to the string variants per-region, so they usually don't need separate changes
4. Write a round-trip test: `assert_eq!(filter_write(filter_read(original)), original)`

## Testing

All core logic has unit tests. The FUSE layer (`fs.rs`) does not have automated tests because it requires `/dev/fuse` and a real mount. To test FUSE changes manually:

```bash
# Terminal 1: mount
mkdir -p /tmp/source /tmp/mount
echo "my key is AKIAIOSFODNN7EXAMPLE" > /tmp/source/test.txt
RUST_LOG=debug cargo run -- -s /tmp/source -m /tmp/mount -c config.example.yaml

# Terminal 2: verify
cat /tmp/mount/test.txt      # Should show placeholder
echo "modified <|SECRET:0001|>" > /tmp/mount/test.txt
cat /tmp/source/test.txt     # Should show original secret restored

# Unmount
fusermount3 -u /tmp/mount
```

## Commit Style

Use [Conventional Commits](https://www.conventionalcommits.org/): `<type>(<scope>): <summary>`

Types: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`, `perf`

Scopes: `config`, `mapper`, `transformer`, `handle`, `fs`, `cli` (or omit for cross-cutting changes)

## Current Limitations

See [`TODO.md`](TODO.md) for the full list. Key ones to know:

- Files are fully buffered in memory on open — don't try to handle huge files without implementing streaming first
- No `mmap` support — programs that mmap files through the mount will fail
- Secret mappings are per-mount and in-memory only — they don't persist
- No hot-reload of secrets config
