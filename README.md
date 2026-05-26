# Actually New Real Time Log Analyzer

Real-time streaming log analysis CLI written in Rust.

## Features

- Streams log files line-by-line without loading into memory
- Parallel processing via worker pool (crossbeam channels)
- Filters by level, keyword, or source — applied before stats are counted
- Anomaly detection: repeated messages, error spikes, message floods
- Rule-based analysis via JSON / YAML / TOML config
- Session management (save, resume, list, delete)
- Live statistics updated every 500 ms

## Quick start

```bash
cargo build --release

# Analyze a file
cargo run -- logs/test.log

# With config and filter
cargo run -- -c test_config.json --filter "level=ERROR" logs/test.log

# Resume a saved session
cargo run -- -s my_session
```

## Commands

| Command | Description |
|---|---|
| `log_analyzer [FILES]` | Analyze one or more log files |
| `log_analyzer start [FILES]` | Same, explicit subcommand |
| `log_analyzer stats [-s SESSION]` | View session info |
| `log_analyzer load-config -c FILE` | Validate a config file |
| `log_analyzer list-sessions` | List all saved sessions |
| `log_analyzer remove-session NAME` | Delete a session |

## Options

| Flag | Default | Description |
|---|---|---|
| `-c, --config` | — | Path to JSON / YAML / TOML config |
| `-s, --session` | — | Session name (creates or resumes) |
| `-w, --workers` | 4 | Number of parallel workers |
| `--filter` | — | `level=ERROR,keyword=fail,source=auth` |

## Log format

```
YYYY-MM-DD HH:MM:SS LEVEL Message [key=value ...]
```

Timestamp is optional. `service=` or `source=` param sets the source field.

## Anomaly detection

Controlled by `anomaly_threshold` in config (default: 20, window: 60 s).

| Type | Trigger |
|---|---|
| `RepeatedMessage` | Same message ≥ threshold times in window |
| `ErrorSpike` | ERROR/CRITICAL count ≥ threshold in window |
| `MessageFlood` | Total messages ≥ threshold × 10 in window |

## Config example (JSON)

```json
{
  "rules": [
    {
      "name": "connection_issues",
      "pattern": "connection|timeout",
      "action": "report",
      "severity": "error",
      "enabled": true
    }
  ],
  "settings": {
    "workers": 4,
    "anomaly_threshold": 15
  }
}
```

Actions: `count` · `warn` · `report` · `ignore`

## Architecture

```
LogReader ──[crossbeam bounded(1000)]──► Analyzer (rules)
                                     └──► Worker × N (rules, parallel)

SharedStatistics  ← updated in Reader for every line
  └─ AnomalyDetector  ← called here, sees 100% of messages
```
