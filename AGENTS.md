# AGENTS.md

## Key Design Constraints (DO NOT BREACH)

- **Round-trip fidelity:** `filter_write(filter_read(original)) == original`.
- **Placeholder format:** `<|SECRET:XXXX|>` (hex, zero-padded).
- **Escaping:** Pre-existing `<|SECRET:...|>` becomes `<|ESCAPED_SECRET:...|>` on read; unescaped on write.
- **Longest match first:** Literal secrets in `Transformer.literals` must be sorted by length descending.
- **Inode consistency:** Operations creating/deleting/moving files **must** call `cache_inode`, `uncache_inode`, or update the cache in `fs.rs`.
- **DIRECT_IO:** `FOPEN_DIRECT_IO` is mandatory to bypass kernel caching of transformed content.

## Implementation Guide

### Adding FUSE operations (`fs.rs`)
1. Resolve path: `self.real_path(ino)`.
2. On create: `self.cache_inode(ino, path)`.
3. On remove: `self.uncache_inode(ino)`.
4. On rename: Update cache with new path.

### Adding secret sources
1. Add variant to `SecretDef` (`config.rs`).
2. Update `Config::load` validation.
3. Update `Transformer::new` to register literals or patterns.

### Adding transformation logic (`transformer.rs`)
1. Update `filter_read_str` and `filter_write_str` symmetrically.
2. Binary variants delegate to string variants; usually no separate changes needed.
3. Always add a round-trip test.

## Manual Testing (FUSE)
```bash
# Terminal 1: mount
mkdir -p /tmp/source /tmp/mount
RUST_LOG=debug cargo run -- -s /tmp/source -m /tmp/mount -c config.example.yaml

# Terminal 2: verify
echo "secret" > /tmp/source/test.txt
cat /tmp/mount/test.txt      # Check placeholder
echo "modified <|SECRET:0001|>" > /tmp/mount/test.txt
cat /tmp/source/test.txt     # Check original restored

# Unmount
fusermount3 -u /tmp/mount
```

## Project Standards
- **Commits:** Conventional Commits with scopes: `config`, `mapper`, `transformer`, `handle`, `fs`, `cli`.
- **Limitations:** Files fully buffered on open (no huge files); no `mmap`; no persistent mappings; no hot-reload.
