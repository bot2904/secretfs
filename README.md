# secretfs

A FUSE filesystem that acts as a filtering proxy over a directory, transparently replacing secrets with stable placeholders on read and restoring them on write. Designed for safely mounting sensitive project directories into Docker containers or other untrusted environments.

## The Problem

You have a project directory containing configuration files, scripts, or documents with embedded secrets (API keys, passwords, tokens). You need to mount this directory into a Docker container — but you don't want the container to see the real secrets.

## How secretfs Solves It

secretfs mounts between your real directory and the consumer. Every file read through the mount has secrets swapped out for placeholders. If the consumer writes files back, placeholders are restored to the originals before hitting disk.

```
 your-app / docker container
        │ reads files
        │
   /mnt/project/          ← mount point (filtered view)
        │
   ┌────┴─────────────┐
   │    secretfs       │   secrets.yaml defines what to redact
   │                   │   "AKIAIOS..." → <|SECRET:0001|>
   │                   │   "ghp_Xf9..."  → <|SECRET:0002|>
   └────┬─────────────┘
        │
   /data/project/          ← real files on disk (untouched)
```

Secrets and placeholders are mapped bidirectionally for the lifetime of the mount. The same secret always gets the same placeholder, so tools processing the files see consistent values.

## Quick Start

### 1. Build

```bash
cargo build --release
# Binary: target/release/secretfs
```

**Build dependencies:** Rust toolchain and a FUSE library (platform-specific, see below).

#### Debian/Ubuntu

```bash
apt-get install libfuse3-dev fuse3 pkg-config build-essential
```

#### macOS

Install [macFUSE](https://osxfuse.github.io/) (requires allowing a kernel extension):

```bash
brew install --cask macfuse
brew install pkg-config
```

After installation, open **System Settings → Privacy & Security** and allow the macFUSE kernel extension. A reboot is required.

> **Note:** On Apple Silicon (M1/M2/M3/M4), you may need to [enable kernel extensions](https://support.apple.com/guide/mac-help/change-startup-disk-security-settings-mchl768f7291/mac) by booting into Recovery Mode and lowering the security policy.

### 2. Create a Secrets Config

```yaml
# secrets.yaml
secrets:
  # Exact strings to redact
  - literal: "AKIAIOSFODNN7EXAMPLE"
  - literal: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
  - literal: "super-secret-db-password"

  # Regex patterns — each unique match gets its own placeholder
  - pattern: "sk-[a-zA-Z0-9]{32,}"
  - pattern: "ghp_[a-zA-Z0-9]{36}"
```

See [`config.example.yaml`](config.example.yaml) for a full example.

### 3. Mount

```bash
mkdir -p /mnt/filtered

secretfs --source ./my-project --mount /mnt/filtered --config secrets.yaml
```

In another terminal:

```bash
$ cat /mnt/filtered/config.env
AWS_ACCESS_KEY=<|SECRET:0001|>
AWS_SECRET_KEY=<|SECRET:0002|>
DB_PASSWORD=<|SECRET:0003|>
```

The real files in `./my-project/` are untouched.

### 4. Unmount

Press `Ctrl+C` in the secretfs terminal, or:

```bash
# Linux
fusermount3 -u /mnt/filtered

# macOS
umount /mnt/filtered
```

## Docker Integration

The primary use case — mounting a filtered view into a container:

```bash
# Start secretfs in the background
secretfs \
  --source ./project \
  --mount /tmp/project-filtered \
  --config secrets.yaml \
  --allow-other &

# Run a container with the filtered mount
docker run --rm -v /tmp/project-filtered:/workspace myimage

# Clean up
fusermount3 -u /tmp/project-filtered   # Linux
umount /tmp/project-filtered            # macOS
```

> **Note:** `--allow-other` requires `user_allow_other` to be set in `/etc/fuse.conf` (Linux) or `/etc/fuse.conf` created with that option on macOS.

> **macOS + Docker:** Docker Desktop for Mac runs in a Linux VM. FUSE mounts on the macOS host are not automatically visible inside the VM. You may need to share the mount point via Docker Desktop's file sharing settings, or run secretfs inside a Linux container instead.

## Configuration

The config file is YAML with a `secrets` list. Each entry is one of:

| Type | Key | Description | Example |
|------|-----|-------------|---------|
| Literal | `literal` | Exact string match | `literal: "my-password"` |
| Pattern | `pattern` | Regex; each unique match gets its own ID | `pattern: "sk-[a-zA-Z0-9]{32,}"` |

**Matching rules:**

- Literal secrets are matched longest-first (so `"secret-long"` matches before `"secret"`)
- Regex patterns are applied after literals; each distinct match is assigned a unique placeholder
- Both text and binary files are processed (binary files: only ASCII-compatible regions)
- If the original file already contains `<|SECRET:...|>`, it is escaped to `<|ESCAPED_SECRET:...|>` and restored on write-back

## CLI Reference

```
secretfs --source <DIR> --mount <DIR> --config <FILE> [OPTIONS]

Arguments:
  -s, --source <DIR>     Source directory to mirror
  -m, --mount <DIR>      Mount point (must exist, should be empty)
  -c, --config <FILE>    Secrets config file (YAML)

Options:
      --allow-other      Allow other users to access the mount
  -f, --foreground       Run in foreground (default: true)
  -h, --help             Print help
  -V, --version          Print version

Environment:
  RUST_LOG=debug         Enable debug logging (shows every FUSE operation)
```

## Limitations

- Files are fully loaded into memory on open (not suitable for multi-GB files)
- Secret mappings live in memory only — they don't survive remounts
- No hot-reloading of the secrets config
- No `mmap` support (DIRECT_IO is used to bypass kernel page cache)
- No extended attribute support

See [`TODO.md`](TODO.md) for planned improvements.

## Further Reading

- [`DESIGN.md`](DESIGN.md) — architecture, design decisions, and internals
- [`TODO.md`](TODO.md) — implemented features and future ideas
- [`config.example.yaml`](config.example.yaml) — example secrets configuration

## License

MIT
