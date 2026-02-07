# Design

This document describes the architecture, components, and design decisions behind secretfs.

## Architecture Overview

secretfs is a passthrough FUSE filesystem. It mirrors a source directory tree at a mount point, intercepting file I/O to perform secret↔placeholder substitution. Directory operations (listing, creation, deletion, rename) pass through to the source directory unchanged.

```
                ┌──────────────────────────────────────────┐
                │              secretfs process            │
                │                                          │
                │  ┌────────────┐     ┌────────────────┐  │
                │  │   Config   │────▶│  Transformer   │  │
                │  │ (secrets   │     │  filter_read()  │  │
                │  │  .yaml)    │     │  filter_write() │  │
                │  └────────────┘     └───────┬────────┘  │
                │                             │           │
                │  ┌────────────┐     ┌───────┴────────┐  │
                │  │   Secret   │◀───▶│  HandleTable   │  │
                │  │   Mapper   │     │  (open files)  │  │
                │  └────────────┘     └───────┬────────┘  │
                │                             │           │
                │  ┌──────────────────────────┴────────┐  │
                │  │         SecretFs (FUSE impl)      │  │
                │  │  lookup · getattr · readdir        │  │
                │  │  open · read · write · release     │  │
                │  │  create · unlink · mkdir · rmdir   │  │
                │  │  rename · setattr · readlink       │  │
                │  └───────────────────────────────────┘  │
                └──────────────┬───────────────────────────┘
                               │
              FUSE protocol    │    real filesystem I/O
              (/dev/fuse)      │    (source directory)
                               │
```

## Components

### `config.rs` — Configuration Parsing

Loads a YAML file defining which secrets to redact. Supports two kinds:

- **Literal** — exact string match (`literal: "AKIAIOS..."`)
- **Pattern** — regex match (`pattern: "sk-[a-zA-Z0-9]{32,}"`)

Validates at load time: non-empty literals, compilable regex patterns, at least one secret defined.

Uses `serde` with `#[serde(untagged)]` enum deserialization so the YAML is clean (no type tags).

### `mapper.rs` — Bidirectional Secret↔Placeholder Map

The `SecretMapper` holds two `HashMap`s behind an `Arc<RwLock>`:

```
secret string  ↔  "<|SECRET:XXXX|>"
```

Where `XXXX` is a zero-padded hex counter (`0001`, `0002`, ...).

**Thread safety:** Uses a double-checked locking pattern — fast `read()` lock for lookups, `write()` lock only when registering new secrets. The `Arc<RwLock>` allows the mapper to be shared between the `Transformer` and `HandleTable`.

**Literal secrets** are pre-registered at startup (deterministic IDs). **Regex matches** are registered lazily on first encounter.

### `transformer.rs` — Content Transformation

The core of the system. Two entry points:

- `filter_read(bytes) → bytes` — replaces secrets with placeholders
- `filter_write(bytes) → bytes` — replaces placeholders with secrets

**Read path (3 steps):**

1. **Escape pre-existing placeholders.** If the original file contains `<|SECRET:...|>`, replace with `<|ESCAPED_SECRET:...|>` so it won't be confused with our placeholders on write-back.
2. **Replace literal secrets.** Longest first (sorted by length descending). This prevents `"secret"` from matching inside `"secret-long"`.
3. **Replace regex matches.** Walk through the (already literal-replaced) content, find regex matches, register each unique match with the mapper, replace with its placeholder.

**Write path (2 steps):**

1. **Replace all known placeholders** with their original secrets (sorted by length descending).
2. **Unescape** `<|ESCAPED_SECRET:...|>` back to `<|SECRET:...|>`.

**Binary file handling:** If content isn't valid UTF-8, the transformer finds contiguous regions of text-like bytes (printable ASCII, whitespace, high bytes that could be UTF-8). Regions ≥4 bytes are processed through the string-based filter. Non-text regions pass through unchanged. Regions are processed in reverse order so byte offset shifts from replacement don't invalidate subsequent region boundaries.

### `handle.rs` — File Handle Lifecycle

The `HandleTable` manages open file state:

```
open(source_content, flags) → fh_id
  → reads source file
  → transforms content (filter_read)
  → stores transformed content as read_content
  → initializes write_buf as clone of read_content
  → returns file handle ID

read(fh, offset, size) → bytes
  → serves slice of read_content

write(fh, offset, data)
  → writes into write_buf at offset
  → marks handle as dirty

flush(fh) → Option<restored_bytes>
  → if dirty: filter_write(write_buf), clear dirty flag
  → caller writes restored bytes to source file

release(fh) → Option<restored_bytes>
  → same as flush but also removes handle from table
```

Each handle is wrapped in `Arc<Mutex<FileHandle>>` so the FUSE layer can hold a reference while the handle table mutex is released.

### `fs.rs` — FUSE Filesystem

Implements the `fuser::Filesystem` trait. This is a passthrough filesystem with transformation hooks on file I/O.

**Inode management:** FUSE requires stable inode numbers. We use the real filesystem's inode numbers and maintain a `HashMap<u64, PathBuf>` cache (the "inode cache") that maps inodes to paths. The cache is populated during `lookup` and `readdir` operations and cleaned up on `unlink`/`rmdir`/`rename`.

**FUSE root inode:** FUSE always uses inode 1 for the root. We map inode 1 → source directory path at mount time. The real inode of the source directory is also cached.

**Implemented FUSE operations:**

| Operation | Behavior |
|-----------|----------|
| `lookup` | Stat child, cache inode, return attrs |
| `getattr` | Stat from cache, compute transformed size |
| `setattr` | Passthrough chmod/chown/truncate |
| `readdir` | List source dir, cache all entry inodes |
| `open` | Read source file, transform, store in HandleTable |
| `read` | Serve slice from transformed content |
| `write` | Buffer into write_buf, mark dirty |
| `flush` | Reverse-transform and write back if dirty |
| `release` | Same as flush + remove handle |
| `create` | Create empty file, open handle |
| `unlink` | Delete file, uncache inode |
| `mkdir` | Create dir, cache inode |
| `rmdir` | Remove dir, uncache inode |
| `rename` | Rename, update inode cache |
| `readlink` | Passthrough |
| `access` | Simple existence check |
| `opendir` / `releasedir` | Passthrough |

**DIRECT_IO flag:** All files are opened with `FOPEN_DIRECT_IO`. This tells the kernel not to cache file content in the page cache. This is essential because our transformed content would be stale if the kernel cached it — the source file could change, or the same file opened multiple times would need fresh transformation.

### `main.rs` — CLI Entrypoint

Uses `clap` derive API for argument parsing. Validates source and mount directories exist. Loads config, creates the mapper + transformer + filesystem, and calls `fuser::mount2()` which blocks until unmount.

## Design Decisions

### Why Rust?

- FUSE filesystems need to be reliable — a crash corrupts nothing but hangs all I/O until unmount
- The `fuser` crate implements the FUSE protocol in pure Rust (no C `libfuse` dependency at the protocol level, just links to `libfuse3` for the mount syscall)
- Memory safety prevents a class of bugs that are common in filesystem code (buffer overflows, use-after-free)
- Performance matters for a filesystem — every `stat()` and `read()` call goes through our code

### Why Full-File Buffering (Not Streaming)?

Secrets and placeholders may differ in length. A 20-character AWS key becomes a 16-character `<|SECRET:0001|>`. This means byte offsets in the transformed content don't correspond to offsets in the source file. Streaming with offset translation would be complex and error-prone.

Instead, on `open()` we read the entire source file, transform it, and cache the result. Reads are served from this cache. This is simple, correct, and supports arbitrary `seek()`/`pread()` patterns. The tradeoff is memory usage (one copy per open file), which is acceptable for the target use case (config files and documents, not multi-GB blobs).

### Why an Inode Cache Instead of Path Walking?

The first implementation used `find_path_by_inode()` — a recursive directory walk to find a file by inode number. This is O(n) per FUSE operation and unacceptable for directories with many files.

The inode cache is populated lazily during `lookup` and `readdir` (the operations FUSE always calls before any other operation on a file). This means by the time `open()` or `getattr()` is called, the inode is already cached. Cost: O(1) per lookup, small memory overhead per cached entry.

The cache is maintained on mutations: `unlink`/`rmdir` remove entries, `rename` updates them, `create`/`mkdir` add them.

### Why `<|SECRET:XXXX|>` as Placeholder Format?

Requirements for the placeholder format:

1. **Unlikely to appear in real content** — avoids false positives
2. **Fixed structure** — can be reliably matched for reverse transformation
3. **Human-readable** — developers can see that redaction happened and which ID maps to what
4. **Unique per secret** — different secrets get different placeholders

`<|SECRET:XXXX|>` uses pipe-delimited angle brackets, which almost never appear in code, configs, or prose. The hex ID is zero-padded to 4 digits (supports up to 65535 secrets per mount, extensible).

### Why Escape Pre-Existing Placeholders?

If a source file already contains the literal text `<|SECRET:BEEF|>` (maybe from a previous run's output, or by coincidence), the write path would incorrectly try to replace it with a secret. The escape mechanism (`<|ESCAPED_SECRET:...|>`) ensures round-trip fidelity: read then write-back produces the exact original bytes.

### Why DIRECT_IO?

Without DIRECT_IO, the Linux kernel caches page-level file content. This causes two problems:

1. **Stale reads:** If the source file changes, the kernel would serve cached (old) transformed content
2. **Size mismatch:** The kernel uses `getattr` size for read boundaries, but if it caches content from a previous open, sizes may not match

DIRECT_IO bypasses the page cache entirely. Every `read()` goes through our FUSE handler. The tradeoff is no kernel-level read caching, but for our use case (config files, not hot-path I/O) this is fine.

### Why Longest-Match-First for Literals?

Consider two configured secrets: `"secret"` and `"secret-long"`. If we replace `"secret"` first, the string `"secret-long"` becomes `"<|SECRET:0001|>-long"` — broken. By sorting literals longest-first and replacing in that order, `"secret-long"` is matched and replaced as a unit.

### Binary File Handling

Rather than skipping binary files entirely, we scan for contiguous text-like regions (printable ASCII, tabs, newlines, potential UTF-8 continuation bytes). Regions ≥4 bytes are processed through the same string-based transformer. This catches secrets embedded in binary formats that have ASCII string tables (executables, archives, Office documents, etc.) while leaving the binary structure intact.

Regions are processed in reverse order so that byte-length changes from replacement don't shift the offsets of subsequent regions.

## File Structure

```
secretfs/
├── Cargo.toml              # Dependencies and project metadata
├── Cargo.lock              # Locked dependency versions
├── config.example.yaml     # Example secrets configuration
├── README.md               # User-facing documentation
├── DESIGN.md               # This file — architecture and decisions
├── TODO.md                 # Implemented features and future ideas
├── AGENTS.md               # Agent instructions for working on this repo
├── .gitignore
└── src/
    ├── main.rs             # CLI entrypoint, argument parsing, mount
    ├── config.rs           # YAML config loading and validation
    ├── mapper.rs           # Bidirectional secret↔placeholder map
    ├── transformer.rs      # Content transformation (read/write filters)
    ├── handle.rs           # Open file handle state and lifecycle
    └── fs.rs               # FUSE filesystem implementation
```

## Test Coverage

Unit tests exist for all core components (run with `cargo test`):

| Module | Tests |
|--------|-------|
| `config` | YAML parsing with mixed literal/pattern entries |
| `mapper` | Registration, idempotent re-registration, bidirectional lookup |
| `transformer` | Literal round-trip, regex round-trip, placeholder escaping, longest-match ordering |
| `handle` | Full open→read→write→release lifecycle with transformation |
