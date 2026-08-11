# QR pairing code — scanner round-trip evidence

`qrMatrix.ts` is a hand-rolled QR encoder with no third-party dependency. Its
failure mode is nasty: a wrong code still *looks* like a QR code on screen and
only reveals itself when a phone silently refuses to scan it. Unit tests that
re-read the matrix (see `qrMatrix.test.ts`) catch bit-packing and placement
bugs, but they share my own understanding of the spec — if I misread the spec,
encoder and test are wrong in the same direction and both pass.

So the encoder was additionally checked against an **independent decoder**:
Apple's CoreImage `CIDetector(ofType: CIDetectorTypeQRCode)`, which is the same
detection stack behind the iOS camera's QR scanning.

## Run — 2026-08-02

The matrix for a representative pairing URL was rendered to a 410×410 8-bit
grayscale PNG (10px per module, 4-module quiet zone) and fed to CoreImage:

```
$ swift decode.swift pairing-qr.png
DECODED: https://jesses-mac-mini-2.tail4429f1.ts.net:8443/pair?claim=8f3c1d2e9a7b4c60
```

Input payload and decoded payload are byte-identical, so the encoder produces a
genuinely scannable code and not merely a self-consistent one.

## Still not covered

This proves the **Mac half** emits a valid code. It does **not** prove the iOS
capture path: `AVCaptureSession` with `metadataObjectTypes = [.qr]` cannot be
exercised in the simulator, and no camera is available to the test harness. The
first real scan from a physical iPhone remains the outstanding manual check for
the pairing flow end to end.
