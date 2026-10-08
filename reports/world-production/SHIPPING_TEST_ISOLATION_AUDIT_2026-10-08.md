# Shipping frontend failure isolation — October8

The original full-run receipt remains unchanged: `SHIPPING_FRONTEND_SUITE_2026-10-08.json`,486 test files,3923 tests,3534 passed and389 failed. The1419 suite count includes nested describes and is not a file count. This investigation ran only focused subsets.

## Confirmed product-style defect fixed

App's new transcription banner used bare12px text/gap and8px16px padding. Replaced with textSize.caption and space.xl/md/xxl. Isolated spacing contract4/4 passed; App.workspaces2/2 passed. Typography initially3/4: remaining World-owned HeroAgentPortrait.tsx:78 bare11px was reported to root for its owning worker. No MeetingRecorder style violation found in that run.

## Confirmed timer pollution, fixed

Exact focused command (from command-center):

```text
npm test -- src/components/world/forum/ForumPreferences.test.tsx src/components/projects/currentProject.shared.test.tsx src/App.workspaces.test.tsx --pool=forks --poolOptions.forks.singleFork
```

Before fix: ForumPreferences5 passed; currentProject4 timeouts and1 pass; App2 blank-render failures. Total6 failed/6 passed,21.35seconds. Without ForumPreferences, currentProject+App passed7/7.

ForumPreferences spied on already-faked global setTimeout/clearTimeout. Local teardown switched to real timers before restoring spies, reinstalling fake originals. Merely reversing that order still failed because Vitest restores spies again at file cleanup. Removed global timer spies; checked the HUD's single pending rest timer is removed on unmount using timer-count difference, and retained listener-cleanup coverage. Restored mocks before uninstalling the fake clock and added original timer-identity assertions.

After fix, the identical three-file singleFork command passed12/12 in3.09seconds. Unresolved async act calls after timeouts explain the reproduced downstream empty renders; this does not prove every failure in the original full run has the same cause.

## Separate storage pollution, isolated rather than broadly refactored

Exact pair:

```text
npm test -- src/components/projects/currentProject.shared.test.tsx src/hooks/useMeetingDictation.drafts.test.ts --pool=forks --poolOptions.forks.singleFork
```

Reproduced6 failed/12 passed. The project test leaves a localStorage stub with get/set/remove/clear but no length/key. The following draft tests cannot enumerate stored entries. These are the same six draft failures as the full receipt; not evidence that production draft persistence lost data.

With identical source and file list, replacing singleFork with `--pool=forks --maxWorkers=1 --minWorkers=1` passed18/18 in2.77seconds. This keeps concurrency low while preserving per-file environment isolation. Installed Vitest1.6.1 batches same-environment files together in singleFork mode; ordinary isolated forks schedule them separately. Recommend serial isolated forks for the coordinator's one full rerun. No global mock cleanup refactor was attempted.

Final source changes for this batch: App.tsx and ForumPreferences.test.tsx. Typecheck exit0; git diff check exit0. Source frozen; no full-suite repeat by this worker, no native builds or runtime actions.
