# TODO

## Implemented

- [x] FUSE passthrough filesystem mirroring a source directory
- [x] YAML configuration with literal string and regex pattern secret definitions
- [x] Bidirectional secret↔placeholder mapping (`SecretMapper`)
- [x] Content transformation on read (secrets → placeholders)
- [x] Content transformation on write (placeholders → secrets)
- [x] Stable placeholder format: `<|SECRET:XXXX|>` with hex IDs
- [x] Longest-match-first ordering for literal secrets
- [x] Lazy registration for regex-matched secrets
- [x] Pre-existing placeholder escaping (`<|SECRET:...|>` → `<|ESCAPED_SECRET:...|>`)
- [x] Binary file support (ASCII region detection and replacement)
- [x] Full file buffering on open with offset-based partial reads
- [x] Partial write support (write buffer with dirty tracking)
- [x] Write-back on flush and release
- [x] DIRECT_IO to bypass kernel page cache
- [x] Inode cache for O(1) path lookups (populated on lookup/readdir)
- [x] Inode cache maintenance on unlink, rmdir, rename, create, mkdir
- [x] File operations: open, read, write, flush, release, create, unlink
- [x] Directory operations: opendir, readdir, releasedir, mkdir, rmdir
- [x] Metadata operations: lookup, getattr, setattr (chmod, chown, truncate)
- [x] Rename support with inode cache update
- [x] Symlink readlink support
- [x] Accurate file size reporting in getattr (computed from transformed content)
- [x] CLI with clap: `--source`, `--mount`, `--config`, `--allow-other`
- [x] Debug logging via `RUST_LOG` environment variable
- [x] Config validation (non-empty literals, valid regex, at least one secret)
- [x] Unit tests for config, mapper, transformer, and handle lifecycle

## Future Ideas

### Usability

- [ ] Daemonize mode (`--no-foreground`) with PID file for production use
- [ ] `--dry-run` mode that prints what would be redacted without mounting
- [ ] Summary on unmount showing how many secrets were redacted and how many files were modified
- [ ] Config file validation CLI subcommand (`secretfs check-config secrets.yaml`)

### Secret Sources

- [ ] Load secrets from environment variables (`env: AWS_SECRET_ACCESS_KEY`)
- [ ] Load secrets from files (`file: /run/secrets/db-password`)
- [ ] Load secrets from HashiCorp Vault, AWS Secrets Manager, etc.
- [ ] Named secrets for more readable placeholders (`<|SECRET:aws_key|>` instead of `<|SECRET:0001|>`)

### Hot Reloading

- [ ] Watch the secrets config file for changes and reload without remount
- [ ] Signal-based reload (`SIGHUP` to re-read config)
- [ ] Add new secrets at runtime via a Unix socket or control file

### Performance

- [ ] Streaming/chunked transformation for large files (avoid loading entire file into memory)
- [ ] LRU cache for transformed content (avoid re-transforming unchanged files)
- [ ] Transformed size cache keyed on (inode, mtime) to avoid re-reading files in getattr
- [ ] Benchmark suite comparing passthrough overhead vs raw filesystem

### Correctness

- [ ] `mmap` support (complex: requires coordinating page-level content with transformation)
- [ ] Extended attribute (`xattr`) passthrough
- [ ] Hard link support
- [ ] `statfs` implementation (report source filesystem stats)
- [ ] Proper `utimens` passthrough for atime/mtime setting
- [ ] Handle truncate to non-zero size correctly (requires offset mapping)
- [ ] Integration tests with actual FUSE mount (requires /dev/fuse)

### Security

- [ ] Memory-lock secret values to prevent swapping to disk (`mlock`)
- [ ] Zeroize secret memory on drop
- [ ] Restrict config file permissions (warn if world-readable)
- [ ] Audit log of what was redacted and when
- [ ] Option to make the mount read-only (no write-back, simpler threat model)

### Deployment

- [ ] Dockerfile for running secretfs as a sidecar container
- [ ] systemd unit file template
- [ ] Helm chart for Kubernetes sidecar pattern
- [ ] Pre-built binaries for Linux amd64/arm64
- [ ] Integration guide for Docker Compose
