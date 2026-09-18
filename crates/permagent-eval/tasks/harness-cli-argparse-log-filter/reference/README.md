# logfilter

A small CLI for filtering application log lines by level and/or date.

    python3 -m logfilter <path-to-log> [--level LEVEL] [--since YYYY-MM-DD] [--last N]

Spec:

- `--level` matches case-insensitively against one of `debug`, `info`,
  `warn`, `error`; an unrecognized level is a usage error.
- `--since DATE` (`YYYY-MM-DD`) keeps only lines on/after that date; an
  unparseable date is a usage error.
- `--last N` keeps only the last N matching lines; `N` must be positive.
  `--since` and `--last` are mutually exclusive.
- A missing/unreadable log file is a runtime error, distinct from a usage
  error.

Exit codes: `0` success, `1` runtime error (e.g. file not found), `2` usage
error (bad/conflicting arguments).
