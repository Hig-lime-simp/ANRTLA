# How to run — Log Analyzer

## Requirements

- Rust (stable)
- Bash (for `.sh` generator scripts)

## Build

```bash
cargo build --release
```

---

## Basic usage

```bash
# Single file
cargo run -- logs/test.log

# Multiple files
cargo run -- logs/test.log logs/test_scenarios/error_spike.log

# With config
cargo run -- -c test_config.json logs/test.log

# With filter (applied before stats — filtered lines never appear in counts)
cargo run -- --filter "level=ERROR" logs/test.log
cargo run -- --filter "level=ERROR,keyword=failed,source=auth" logs/test.log

# With N workers
cargo run -- -w 8 logs/test.log
```

---

## Filter syntax

`--filter "key=value,key=value,..."`

| Key | Example | Description |
|---|---|---|
| `level` | `level=ERROR` | Match by log level |
| `keyword` | `keyword=timeout` | Substring match in message |
| `source` | `source=auth` | Match `service=` param |

---

## Sessions

```bash
# Create session and run
cargo run -- -s my_session logs/test.log

# Resume saved session
cargo run -- -s my_session

# List sessions
cargo run -- list-sessions

# View session info
cargo run -- stats -s my_session

# Delete session
cargo run -- remove-session my_session
```

---

## Config files

Supported formats: **JSON**, **YAML**, **TOML**.

```bash
# Validate config without running analysis
cargo run -- load-config -c test_config.json
cargo run -- load-config -c test_config.yaml
cargo run -- load-config -c test_config.toml
```

### Key config fields

```json
{
  "rules": [
    {
      "name": "connection_issues",
      "pattern": "connection|timeout",
      "action": "report",
      "severity": "error",
      "enabled": true,
      "threshold": 5,
      "time_window": 60
    }
  ],
  "settings": {
    "workers": 4,
    "anomaly_threshold": 15,
    "output_format": "text"
  }
}
```

Actions: `count` · `warn` · `report` · `ignore`

---

## Anomaly detection testing

Scripts in `logs/` write to `logs/test.log` and trigger a specific anomaly type.

### Generate and test each anomaly

```bash
# RepeatedMessage — same message 25 times
bash logs/gen_repeated_message.sh
cargo run -- -c logs/test_scenarios/comprehensive_config.json logs/test.log

# ErrorSpike — 25 different ERROR/CRITICAL lines
bash logs/gen_error_spike.sh
cargo run -- -c logs/test_scenarios/comprehensive_config.json logs/test.log

# MessageFlood — 220 lines total
bash logs/gen_message_flood.sh
cargo run -- -c logs/test_scenarios/comprehensive_config.json logs/test.log

# Generate all three at once (last script wins in logs/test.log)
bash logs/gen_all.sh
```

Anomaly thresholds in `comprehensive_config.json`: `anomaly_threshold = 15`, window = 60 s.

| Anomaly | Trigger |
|---|---|
| `RepeatedMessage` | Same message ≥ 15 times in 60 s |
| `ErrorSpike` | ERROR/CRITICAL ≥ 15 in 60 s |
| `MessageFlood` | Total messages ≥ 150 in 60 s |

---

## Running tests

```bash
cargo test
cargo test -- --nocapture          # with stdout
RUST_LOG=debug cargo test -- --nocapture  # with debug logs
cargo test statistics              # single module
```

---

## Log format

```
YYYY-MM-DD HH:MM:SS LEVEL Message [key=value ...]
```

```
2024-01-15 10:30:00 INFO  App started [service=auth]
2024-01-15 10:30:01 ERROR DB connection failed [service=db host=localhost]
2024-01-15 10:30:02 WARN  High memory [service=monitor memory=85%]
```

Supported levels: `DEBUG` · `INFO` · `WARN` / `WARNING` · `ERROR` · `CRITICAL`
