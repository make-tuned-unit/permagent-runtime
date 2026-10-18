# Build header alignment — October 18, 2026

Removed the entire Build subtitle as requested, so the Projects picker sits beside the one-line Build title. The shared header, project launch callbacks and active-run controls remain unchanged. The existing rail tests now check the title-only block in idle and active states.

Verification: Build and ViewHeader suites passed 10 files / 53 tests; TypeScript and diff checks passed. Logs: `build-header-tests-20261018.log`, `build-header-typecheck-20261018.log`. Packaged installation and native visual check are pending.
