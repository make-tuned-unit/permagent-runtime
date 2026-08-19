# Audio decode fixtures

Real encoded audio, not hand-built headers: a hand-built fixture can only assert
the metadata someone typed into it, and the bug these guard against was
*missing* metadata. Every file here is the same ~1.4 s of synthesised speech
("Testing audio decode."), produced on macOS 2026-08-19 by AVFoundation --
the same encoder path the phone meeting recorder uses:

    say -v Samantha -o short.aiff "Testing audio decode."
    afconvert -f WAVE -d LEI16@16000 -c 1 short.aiff speech_mono_16k.wav
    afconvert -f m4af -d aac@16000   -c 1 -s 3 short.aiff speech_mono_16k_aac.m4a
    afconvert -f m4af -d aac@16000   -c 2 -s 3 short.aiff speech_stereo_16k_aac.m4a
    afconvert -f m4af -d alac@16000  -c 1 short.aiff speech_mono_16k_alac.m4a

The two AAC files and the ALAC file all declare no channel count at the
container level; the WAV declares one. That difference is the whole point.
