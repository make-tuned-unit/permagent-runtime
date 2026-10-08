# Dictation tail-loss source fix

Parent direct API evidence:24k mono PCM16 mic_open/far_open lost later negations; ffmpeg16k repeat also lost them. These are uploaded-file transcription results, not native capture evidence. No daemon calls, live configuration changes or Cargo invocation by this lane.

The route `/api/dictation/transcribe` delegates to `dictation/providers.rs::transcribe_local` and `WhisperTranscriber::transcribe`. WAV decode trusts actual decoded rate then resamples to16k, so the route does not simply reinterpret24k as16k. Parent's16k countertest independently rules out rate mismatch as sufficient explanation.

Two concrete source defects in `crates/goose/src/dictation/whisper.rs`:

1. Timestamp tokens were stripped before returning from process_segment; outer transcribe always advanced by the whole audio window. A decoder stop at an unfinished utterance boundary therefore skipped audio after the completed prefix. It now returns committed text plus actual timestamp-covered frame count, so the next iteration revisits remaining audio. Completed window results retain full-window advancement.
2. Timestamp monotonicity always required last_timestamp+1. Adjacent utterances may share an end/start timestamp; now equality is permitted only at that transition, while utterance ends still advance.

The reference decoder makes both distinctions. [OpenAI Whisper window advancement](https://github.com/openai/whisper/blob/main/whisper/transcribe.py), [OpenAI Whisper timestamp constraints](https://github.com/openai/whisper/blob/main/whisper/decoding.py)

New dependency-free production helper `dictation/whisper_segments.rs` carries four regression tests. They run without Whisper/model dependencies:

```sh
rustc --edition 2021 --test reports/record-evidence/whisper-window-harness.rs -o reports/record-evidence/whisper-window-tests
reports/record-evidence/whisper-window-tests --nocapture
```

Result:4 passed,0 failed. Tests exercise a10-second clip partially decoded to4seconds followed by an unfinished utterance; ensure remaining negation/commitment tokens are revisited and committed once; distinguish a genuine terminal timestamp; preserve cursor progress on invalid timestamps; and permit matching adjacent utterance boundaries. rustfmt checks and git diff --check pass. No full crate compile/test yet while parent heavy build owns resources.

The source defects are proven by token-window tests; attributing the exact fixture failure to these defects remains an inference until the rebuilt daemon returns the missing tail. For retest, send both original24k and converted16k fixtures, compare semantic negations and final commitments, then test a>30-second multi-window sample. Existing debug log `all tokens generated in segment`, plus new `advance_frames`, can distinguish a prematurely finalized single-timestamp result from an unfinished boundary. If the model predicts a genuine terminal end before audible speech, this accounting fix does not fabricate missing words; further decoder-quality work remains.
