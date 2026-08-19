// permagent-applefm — on-device inference over Apple's Foundation Models.
//
// WHY A SEPARATE BINARY. FoundationModels is a Swift-only framework: its
// public surface (`SystemLanguageModel`, `LanguageModelSession`, the
// `ResponseStream` AsyncSequence) has no Objective-C or C entry points, so
// Rust cannot reach it through objc2. Something Swift must exist either way.
// The two shapes available were a C-ABI static library linked into the daemon
// (Apple ships an Apache-2.0 shim for exactly that) and this: a long-lived
// child process speaking line-delimited JSON. The sidecar was chosen because
// generation runs in another address space — a guardrail trip, an assets
// eviction, or a framework crash costs one describe pass, not the daemon —
// and because it needs no Swift runtime linked into the Rust binary and no
// vendored package. It is the same trade the system-audio helper already
// makes (see ui/desktop/src-tauri/audiocap/main.swift).
//
// WHY IT IS LONG-LIVED RATHER THAN SPAWNED PER CALL. Measured on macOS 26.2,
// M-series, with a ~700-token prompt and a 150-token cap: the first call in a
// cold process is ~5.1s, the first call in a fresh process once the OS has the
// assets resident is ~2.0s, and every subsequent call in the same process is
// ~1.2s. Spawning per call would therefore pay roughly an extra 800ms every
// time, on the highest-volume consumer in the product. One warm process pays
// it once.
//
// WHERE THE WEIGHTS COME FROM. Inference is local. Provisioning is not: the OS
// downloads the model assets, and until it has finished, availability reports
// `.unavailable(.modelNotReady)`. Nothing here downloads anything.
//
// PROTOCOL. One JSON object per line on stdin, one or more JSON objects per
// line on stdout, correlated by `id`. stdin EOF terminates the process.
//
//   ->  {"id":1,"op":"probe"}
//   <-  {"id":1,"type":"probe","available":true,"context_size":4096}
//   <-  {"id":1,"type":"probe","available":false,"reason":"model_not_ready"}
//
//   ->  {"id":2,"op":"generate","instructions":"...","prompt":"...",
//        "max_tokens":150,"temperature":0.2,"stream":true}
//   <-  {"id":2,"type":"delta","text":"..."}          (repeated, if streaming)
//   <-  {"id":2,"type":"done","text":"...","context_size":4096,"elapsed_ms":1183}
//   <-  {"id":2,"type":"error","reason":"guardrail_violation","message":"..."}
//
// Availability is re-read on EVERY request rather than cached at startup:
// Apple Intelligence can be switched off, and the assets can be evicted, while
// this process is running. A stale "available" would turn a fallback into a
// failure.
//
// Requests are served strictly one at a time. FoundationModels rejects
// overlapping generations on one session with `.concurrentRequests`, and the
// callers here (a nightly archiving pass, a chat turn) are sequential anyway.

import Foundation

#if canImport(FoundationModels)
    import FoundationModels
#endif

// ── Wire types ───────────────────────────────────────────────────────────────

struct Request: Decodable {
    let id: Int
    let op: String
    var instructions: String?
    var prompt: String?
    var maxTokens: Int?
    var temperature: Double?
    var stream: Bool?
}

/// Write one JSON object as a line on stdout. `JSONSerialization` escapes
/// embedded newlines, so a model response containing them cannot desynchronise
/// the line protocol. `FileHandle` writes are unbuffered — a consumer reading
/// stdout sees each delta as it is produced, not at exit.
func emit(_ object: [String: Any]) {
    guard let data = try? JSONSerialization.data(withJSONObject: object) else { return }
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data([0x0A]))
}

func emitError(_ id: Int, _ reason: String, _ message: String) {
    emit(["id": id, "type": "error", "reason": reason, "message": message])
}

// ── Availability ─────────────────────────────────────────────────────────────

#if canImport(FoundationModels)

    /// Stable snake_case names for the framework's unavailability reasons. The
    /// Rust side matches on these strings and logs them, so they are part of the
    /// contract and must not be reworded to suit a display surface.
    @available(macOS 26.0, *)
    func reasonName(_ reason: SystemLanguageModel.Availability.UnavailableReason) -> String {
        switch reason {
        case .deviceNotEligible: return "device_not_eligible"
        case .appleIntelligenceNotEnabled: return "apple_intelligence_not_enabled"
        case .modelNotReady: return "model_not_ready"
        @unknown default: return "unknown"
        }
    }

    /// Stable names for generation failures. `context_window_exceeded` and
    /// `guardrail_violation` are the two the caller treats as "this prompt will
    /// never work here" rather than "the backend is down".
    @available(macOS 26.0, *)
    func errorName(_ error: Error) -> String {
        guard let generation = error as? LanguageModelSession.GenerationError else {
            return "generation_failed"
        }
        switch generation {
        case .exceededContextWindowSize: return "context_window_exceeded"
        case .assetsUnavailable: return "assets_unavailable"
        case .guardrailViolation: return "guardrail_violation"
        case .unsupportedGuide: return "unsupported_guide"
        case .unsupportedLanguageOrLocale: return "unsupported_language"
        case .decodingFailure: return "decoding_failure"
        case .rateLimited: return "rate_limited"
        case .concurrentRequests: return "concurrent_requests"
        case .refusal: return "refusal"
        @unknown default: return "generation_failed"
        }
    }

    @available(macOS 26.0, *)
    func handleProbe(_ request: Request) {
        let model = SystemLanguageModel.default
        switch model.availability {
        case .available:
            // Read the window rather than assume it. It is 4096 on macOS 26.2 —
            // `contextSize` is back-deployed before 26.4 and its fallback body
            // returns that literal — and on 26.4+ the same call returns whatever
            // the installed model actually has. Reading it means the number
            // tracks the OS with no code change here.
            emit([
                "id": request.id, "type": "probe", "available": true,
                "context_size": model.contextSize,
            ])
        case .unavailable(let reason):
            emit([
                "id": request.id, "type": "probe", "available": false,
                "reason": reasonName(reason),
            ])
        @unknown default:
            emit([
                "id": request.id, "type": "probe", "available": false, "reason": "unknown",
            ])
        }
    }

    @available(macOS 26.0, *)
    func handleGenerate(_ request: Request) async {
        let model = SystemLanguageModel.default

        // Re-probed per request: Apple Intelligence can be turned off, and the
        // assets can be evicted, between one call and the next.
        if case .unavailable(let reason) = model.availability {
            emitError(request.id, reasonName(reason), "on-device model is not available")
            return
        }

        guard let prompt = request.prompt, !prompt.isEmpty else {
            emitError(request.id, "bad_request", "generate requires a non-empty prompt")
            return
        }

        let options = GenerationOptions(
            temperature: request.temperature,
            maximumResponseTokens: request.maxTokens
        )
        let session = LanguageModelSession(model: model, instructions: request.instructions)
        let started = Date()

        do {
            var text = ""
            if request.stream ?? true {
                // Snapshots are cumulative, not incremental: each carries the
                // whole response so far. The caller wants deltas, so diff against
                // what was already sent. If a snapshot ever fails to extend the
                // previous one, resynchronise on the new full text rather than
                // emit a bogus delta.
                for try await snapshot in session.streamResponse(to: prompt, options: options) {
                    let whole = snapshot.content
                    let delta: String
                    if whole.hasPrefix(text) {
                        delta = String(whole.dropFirst(text.count))
                    } else {
                        delta = whole
                    }
                    text = whole
                    if !delta.isEmpty {
                        emit(["id": request.id, "type": "delta", "text": delta])
                    }
                }
            } else {
                text = try await session.respond(to: prompt, options: options).content
            }
            emit([
                "id": request.id, "type": "done", "text": text,
                "context_size": model.contextSize,
                "elapsed_ms": Int(Date().timeIntervalSince(started) * 1000),
            ])
        } catch {
            emitError(request.id, errorName(error), String(describing: error))
        }
    }

    /// Pay the per-process warm-up before the first real request arrives, so it
    /// overlaps the caller's probe round-trip instead of landing on a user's
    /// first description. Best-effort: prewarming is a hint, and a failure here
    /// must not stop the process serving requests.
    @available(macOS 26.0, *)
    func prewarm() {
        guard case .available = SystemLanguageModel.default.availability else { return }
        LanguageModelSession(model: SystemLanguageModel.default).prewarm()
    }

#endif

// ── Main loop ────────────────────────────────────────────────────────────────

@main
struct AppleFoundationModelsSidecar {
    static func main() async {
        #if canImport(FoundationModels)
            guard #available(macOS 26.0, *) else {
                // Built against a 26 SDK but launched on an older system. The
                // framework is weak-linked, so the process starts; every request
                // is answered honestly rather than crashing on first use.
                await serveUnsupported(reason: "os_too_old")
                return
            }
            prewarm()
            await serve()
        #else
            // Not macOS, or a toolchain without the framework. The build script
            // does not produce this binary off Darwin at all, so this branch
            // exists to keep the file compilable rather than to ship.
            await serveUnsupported(reason: "unsupported_platform")
        #endif
    }

    /// Answer every request with the same unavailability reason. Deliberately
    /// not an exit: a caller that started this process gets a reasoned refusal
    /// it can log and fall back from, not a broken pipe it has to guess about.
    static func serveUnsupported(reason: String) async {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        do {
            for try await line in FileHandle.standardInput.bytes.lines {
                guard let data = line.data(using: .utf8),
                    let request = try? decoder.decode(Request.self, from: data)
                else { continue }
                if request.op == "probe" {
                    emit([
                        "id": request.id, "type": "probe", "available": false, "reason": reason,
                    ])
                } else {
                    emitError(request.id, reason, "on-device model is not available")
                }
            }
        } catch {}
    }

    #if canImport(FoundationModels)
        @available(macOS 26.0, *)
        static func serve() async {
            let decoder = JSONDecoder()
            decoder.keyDecodingStrategy = .convertFromSnakeCase
            do {
                for try await line in FileHandle.standardInput.bytes.lines {
                    let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
                    if trimmed.isEmpty { continue }
                    guard let data = trimmed.data(using: .utf8) else { continue }
                    let request: Request
                    do {
                        request = try decoder.decode(Request.self, from: data)
                    } catch {
                        // No id to correlate against, so this cannot be answered
                        // on the wire. The caller's read times out and it falls
                        // back; stderr is where a human finds out why.
                        FileHandle.standardError.write(
                            "permagent-applefm: undecodable request: \(error)\n".data(using: .utf8)!
                        )
                        continue
                    }
                    switch request.op {
                    case "probe": handleProbe(request)
                    case "generate": await handleGenerate(request)
                    default:
                        emitError(request.id, "bad_request", "unknown op '\(request.op)'")
                    }
                }
            } catch {
                FileHandle.standardError.write(
                    "permagent-applefm: stdin closed: \(error)\n".data(using: .utf8)!
                )
            }
        }
    #endif
}
