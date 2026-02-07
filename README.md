# secretfs

A FUSE filesystem that transparently replaces secrets with placeholders.

## Use Case

Mount a directory containing sensitive files into a Docker container (or any environment) where secrets should not be exposed. Any process reading files through the mount sees placeholders instead of real secrets. When files are written back, placeholders are automatically restored to the original secrets.

## How It Works

```
┌──────────────────────┐
│  Application         │
│  (reads/writes)      │
│         │            │
│    mount point       │
│   /mnt/filtered/     │
└────────┬─────────────┘
         │ FUSE
┌────────┴─────────────┐
│   secretfs            │
│                       │
│  secrets.yaml         │
│  → "AKIAIOS..." →    │
│    <|SECRET:0001|>    │
└────────┬─────────────┘
         │ real I/O
   /data/source/
```

### Read Path

1. Application opens a file through the mount point
2. secretfs reads the real file from the source directory
3. All secrets (literal strings and regex matches) are replaced with stable placeholders like `<|SECRET:0001|>`
4. The transformed content is served to the application

### Write Path

1. Application writes a file through the mount point
2. secretfs replaces all placeholders back with their original secrets
3. The restored content is written to the source file

### Placeholder Format

Placeholders use the format `<|SECRET:XXXX|>` where `XXXX` is a hex ID. If the original file already contains text matching this format, it is escaped to `<|ESCAPED_SECRET:XXXX|>` and restored on write-back.

## Installation

```bash
# Build from source
cargo build --release

# The binary is at target/release/secretfs
```

### Dependencies

- Linux with FUSE3 support
- `libfuse3-dev` (build time, for the system FUSE library)
- `fuse3` (runtime)

## Usage

### 1. Create a secrets configuration file

```yaml
# secrets.yaml
secrets:
  # Literal strings to replace
  - literal: "AKIAIOSFODNN7EXAMPLE"
  - literal: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"

  # Regex patterns — matched dynamically
  - pattern: "sk-[a-zA-Z0-9]{32,}"
  - pattern: "ghp_[a-zA-Z0-9]{36}"
```

### 2. Mount

```bash
# Create mount point
mkdir -p /mnt/filtered

# Mount source directory
secretfs --source /data/real-files --mount /mnt/filtered --config secrets.yaml

# In another terminal, files are now filtered:
cat /mnt/filtered/config.env
# → API_KEY=<|SECRET:0001|>

# Unmount with Ctrl+C or:
fusermount3 -u /mnt/filtered
```

### 3. Docker Integration

```bash
# Mount filtered view into a Docker container
secretfs --source ./project --mount /tmp/project-filtered --config secrets.yaml --allow-other &

docker run -v /tmp/project-filtered:/workspace myimage
```

Note: `--allow-other` requires `user_allow_other` in `/etc/fuse.conf`.

## Configuration Reference

The config file is YAML with a top-level `secrets` list. Each entry is one of:

| Type | Field | Description |
|------|-------|-------------|
| Literal | `literal` | Exact string to match and replace |
| Pattern | `pattern` | Regex pattern; each unique match gets its own placeholder |

### Matching Rules

- Literal secrets are matched **longest first** to prevent partial matches
- Regex patterns are matched greedily (longest match wins)
- Mappings are **per-mount** and exist only in memory (not persisted)
- Both text and binary files are processed (binary files: ASCII-range regions only)

## CLI Reference

```
secretfs --source <DIR> --mount <DIR> --config <FILE> [OPTIONS]

Options:
  -s, --source <DIR>     Source directory to mirror
  -m, --mount <DIR>      Mount point (must exist, should be empty)
  -c, --config <FILE>    Secrets config file (YAML)
      --allow-other      Allow other users to access the mount
  -f, --foreground       Run in foreground (default: true)
  -h, --help             Print help
  -V, --version          Print version
```

## Limitations

- File content is fully loaded into memory on open (not suitable for very large files)
- Secret mappings are not persisted across remounts
- No hot-reloading of secrets config
- No `mmap` support
- No extended attributes support

## License

MIT
