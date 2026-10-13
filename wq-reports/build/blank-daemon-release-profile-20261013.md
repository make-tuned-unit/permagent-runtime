# Blank daemon: --profile release (2026-10-13)

## Change
- `isolated_daemon.py`: `PROFILES=('debug','release')`, `PROFILE`, `set_profile(name)`. `DEBUG` (historical name, kept so all existing tests and `fresh_startup_probe.py` work) is now the ACTIVE profile dir `TARGET/<profile>`. Unknown profile raises. `--profile` added to its CLI (default debug), including `sign-libs`. Error texts name the active profile.
- `blank_native_daemon.py`: `--profile {debug,release}` (default debug) calls `set_profile` before any check; evidence JSON records `profile`; `--sha256` help text generic.
- Tests: `reports/record-evidence/test_profile_option.py` (7 tests): debug default, release path/DYLD resolution, unknown profile rejected, release libs fail closed without borrowing debug libs (also `sign-libs`), symlink escape, binary only from active profile, CLI parse.

## Checks preserved (unchanged code, same thresholds)
Ownership manifest (binary path, dylib parent == profile dir), dylib symlink-escape, "only the owned `<profile>/permagentd`", fresh root, free ports 3002 + 43009 (Ollama denied), disk floors (25/20 GiB; internal 12/8), codesign --strict verify of every dylib, bundled defaults/init-config scan, supervisor lifetime and readiness window.

## Tests
`cd reports/record-evidence && python3 -m unittest discover -s . -p 'test_*.py'` -> 51 OK (python3.14). `/usr/bin/python3 -m unittest test_profile_option test_isolated_daemon test_blank_native_daemon_env` -> 22 OK (3.9).

## Dylib findings (release)
`otool -L release/permagentd` needs `@rpath/libsherpa-onnx-c-api.dylib` (plus system frameworks). Release build placed `libsherpa-onnx-c-api.dylib`, `libonnxruntime.dylib`, `libonnxruntime.1.27.0.dylib` (regular files, not symlinks) directly in `release/` (also libsherpa-onnx-cxx-api, not matched/needed). They were present but the two onnxruntime copies had INVALID signatures (`codesign --verify --strict` failed), so the run's verify step would have failed closed. I ran the existing explicit `isolated_daemon.py --profile release sign-libs` (ad-hoc re-sign of only those owned release dylibs; mutates files in the release dir, mtime Jul 7 content bytes changed by signature). No debug dylibs were used.
Signed shas: sherpa c8d9e266..., onnxruntime.dylib 5ed47455..., onnxruntime.1.27.0 5b4c4df6...

## Dry run (release)
Daemon sha256 `93f6b324cdcaa6df9ca48be55b34bcf08025af29df5e1e5541bab70fcbba07f6`. Prior root moved to `record-qa-runtime.prev-20261013-043752`. `/usr/bin/python3` (3.9) ran it fine. `curl :3002/status` -> `ok`; authenticated readiness at 26.38 s; exited on its own at the 60 s deadline, daemonExitCode 0; no listener left.
Evidence: `/Volumes/PermagentBuilds/permagent/targets/world-production-20261006/record-qa-runtime/blank-native-daemon-20261013-043753.json`
