# Prime integration live smoke

Lib tests (no live Claude Code):

```
cargo test -p permagent --lib trigger_roadmap_dispatch
cargo test -p permagent --lib rlm
cargo test -p permagent --lib subagent
cargo test -p permagent --lib executable_skills
cargo test -p permagent --lib goal_refinement
cargo test -p permagent --lib review_fanout
cargo test -p permagent --lib goal_a2a
```

Against a rebuilt `permagentd`:

1. Confirm startup logs `Goal DAG driver loop started`.
2. Feed `docs/planning/prime_integration_roadmap_goals.json` to `create_roadmap`.
3. Root goal 0 (no `depends_on`) should dispatch first; dependents promote after Complete.
