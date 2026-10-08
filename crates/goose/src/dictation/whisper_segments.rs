//! Timestamp-window accounting for local Whisper dictation.
//!
//! A decoder result need not cover the whole input window. Consecutive
//! timestamps close one utterance and open another; without a final single
//! timestamp, only the completed prefix is committed and the remaining audio
//! must be decoded again. See OpenAI Whisper's transcribe.py timestamp slicing.

pub(super) struct DecodedWindow {
    pub text_tokens: Vec<u32>,
    pub advance_frames: usize,
}

pub(super) fn decoded_window(
    tokens: &[u32],
    segment_frames: usize,
    timestamp_begin: u32,
    eot: u32,
    frames_per_timestamp: usize,
) -> DecodedWindow {
    // EOT is an instruction to stop decoding, not part of timestamp ending
    // classification. The reference decoder also removes it from its result.
    let tokens = &tokens[..tokens
        .iter()
        .position(|&t| t == eot)
        .unwrap_or(tokens.len())];
    let single_timestamp_ending = tokens.len() >= 2
        && tokens[tokens.len() - 2] < timestamp_begin
        && tokens[tokens.len() - 1] >= timestamp_begin;
    let last_boundary = tokens
        .windows(2)
        .rposition(|pair| pair[0] >= timestamp_begin && pair[1] >= timestamp_begin)
        .map(|index| index + 1);

    let (committed, advance_frames) = if let Some(boundary) = last_boundary {
        let covered = (tokens[boundary - 1] - timestamp_begin) as usize * frames_per_timestamp;
        if !single_timestamp_ending && covered > 0 && covered < segment_frames {
            (&tokens[..boundary], covered)
        } else {
            // A terminal single timestamp declares the window finished. Zero
            // or out-of-window timestamps must never stall/reverse the cursor.
            (tokens, segment_frames)
        }
    } else {
        (tokens, segment_frames)
    };

    DecodedWindow {
        text_tokens: committed.iter().copied().filter(|&t| t < eot).collect(),
        advance_frames,
    }
}

/// Adjacent utterances may share an end/start timestamp. Within an utterance
/// its ending timestamp must advance, preventing zero-duration loops.
pub(super) fn timestamp_floor(tokens: &[u32], timestamp_begin: u32) -> Option<u32> {
    let last_timestamp = tokens.iter().copied().rfind(|&t| t >= timestamp_begin)?;
    let last_is_timestamp = tokens.last().is_some_and(|&t| t >= timestamp_begin);
    let previous_is_timestamp = tokens.len() < 2 || tokens[tokens.len() - 2] >= timestamp_begin;
    Some(if last_is_timestamp && !previous_is_timestamp {
        last_timestamp
    } else {
        last_timestamp.saturating_add(1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const TS: u32 = 50364;
    const EOT: u32 = 50257;

    #[test]
    fn incomplete_window_revisits_tail_instead_of_losing_later_commitment() {
        // Decoder ends after one completed utterance and an unfinished next
        // one. Old code committed both partial text and skipped all 10 seconds.
        let first = decoded_window(&[TS, 10, 11, TS + 200, TS + 200, 12, EOT], 1000, TS, EOT, 2);
        assert_eq!(first.advance_frames, 400);
        assert_eq!(first.text_tokens, [10, 11]);
        // Remainder contains the negation/action withdrawal and must be visited.
        let tail = decoded_window(&[TS, 12, 13, 14, TS + 300, EOT], 600, TS, EOT, 2);
        assert_eq!(tail.advance_frames, 600);
        assert_eq!(
            [first.text_tokens, tail.text_tokens].concat(),
            [10, 11, 12, 13, 14]
        );
    }

    #[test]
    fn terminal_timestamp_finishes_window_without_duplicate_replay() {
        let result = decoded_window(
            &[TS, 10, TS + 100, TS + 100, 20, TS + 400, EOT],
            1000,
            TS,
            EOT,
            2,
        );
        assert_eq!(result.advance_frames, 1000);
        assert_eq!(result.text_tokens, [10, 20]);
    }

    #[test]
    fn invalid_or_missing_timestamps_cannot_stall_or_exceed_window() {
        for tokens in [
            vec![],
            vec![10, EOT],
            vec![TS, TS, EOT],
            vec![TS, 10, TS + 2000, TS + 2000],
        ] {
            assert_eq!(decoded_window(&tokens, 500, TS, EOT, 2).advance_frames, 500);
        }
    }

    #[test]
    fn adjacent_utterances_can_start_at_previous_end_but_must_end_later() {
        assert_eq!(timestamp_floor(&[TS, 10, TS + 100], TS), Some(TS + 100));
        assert_eq!(
            timestamp_floor(&[TS, 10, TS + 100, TS + 100], TS),
            Some(TS + 101)
        );
        assert_eq!(timestamp_floor(&[TS, 10], TS), Some(TS + 1));
        assert_eq!(timestamp_floor(&[10], TS), None);
    }
}
