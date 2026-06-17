//! Local, on-device OCR via Apple's Vision framework (`VNRecognizeTextRequest`).
//!
//! No network, no cloud API — text recognition runs on the Neural Engine /
//! GPU. We call Vision through the ObjC runtime (`msg_send!`) rather than the
//! typed `objc2-vision` crate: it keeps dependency weight minimal (the Vision
//! framework is linked directly) and avoids coupling to a typed-API version.
//! The same `objc2::exception::catch` + `catch_unwind` belt-and-suspenders
//! bridge pattern as the WKWebView mic-capture bridge guards against ObjC
//! exceptions and Rust panics — a failure degrades to an error, never a crash.
//!
//! On non-macOS targets `recognize` returns an error (images are not OCR'd);
//! Permagent is a macOS-first product and this keeps the crate compiling on
//! Linux CI.

/// One recognized line of text with Vision's per-observation confidence (0–1).
#[derive(Debug, Clone)]
pub struct OcrLine {
    pub text: String,
    pub confidence: f32,
}

/// Run Vision text recognition over raw image bytes (PNG/JPEG/HEIC/…).
///
/// Returns the recognized lines, each with a confidence score. The caller
/// weighs total volume AND confidence to decide whether the image is "textual"
/// (worth ingesting as a digest) or "visual" (pass to the agent to see).
#[cfg(target_os = "macos")]
pub fn recognize(image_bytes: &[u8]) -> Result<Vec<OcrLine>, String> {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        objc2::exception::catch(std::panic::AssertUnwindSafe(|| unsafe {
            macos::recognize_inner(image_bytes)
        }))
    }));
    match result {
        Ok(Ok(lines)) => Ok(lines),
        Ok(Err(exc)) => Err(format!("Vision ObjC exception: {exc:?}")),
        Err(_) => Err("Vision OCR panicked".to_string()),
    }
}

/// Non-macOS stub: images are not OCR'd off-platform.
#[cfg(not(target_os = "macos"))]
pub fn recognize(_image_bytes: &[u8]) -> Result<Vec<OcrLine>, String> {
    Err("Vision OCR is only available on macOS".to_string())
}

#[cfg(target_os = "macos")]
mod macos {
    use super::OcrLine;
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use objc2_foundation::NSString;
    use std::ffi::c_void;

    // Link the Vision framework directly — we resolve its classes at runtime
    // via the ObjC runtime, so no typed crate is required.
    #[link(name = "Vision", kind = "framework")]
    extern "C" {}

    // VNRequestTextRecognitionLevel: Accurate = 0, Fast = 1.
    const RECOGNITION_LEVEL_ACCURATE: i64 = 0;

    /// # Safety
    /// Must run inside the `objc2::exception::catch` + `catch_unwind` guard in
    /// the public `recognize`. Issues raw `msg_send!` calls to Vision/Foundation.
    pub(super) unsafe fn recognize_inner(image_bytes: &[u8]) -> Vec<OcrLine> {
        objc2::rc::autoreleasepool(|_pool| {
            // NSData wrapping the image bytes (copied into the NSData).
            let nsdata: *mut AnyObject = msg_send![
                class!(NSData),
                dataWithBytes: image_bytes.as_ptr() as *const c_void,
                length: image_bytes.len(),
            ];
            if nsdata.is_null() {
                return Vec::new();
            }

            // handler = [[VNImageRequestHandler alloc] initWithData:nsdata options:@{}]
            let empty_opts: *mut AnyObject = msg_send![class!(NSDictionary), dictionary];
            let handler: *mut AnyObject = msg_send![class!(VNImageRequestHandler), alloc];
            let handler: *mut AnyObject =
                msg_send![handler, initWithData: nsdata, options: empty_opts];
            if handler.is_null() {
                return Vec::new();
            }

            // request = [[VNRecognizeTextRequest alloc] init]
            let request: *mut AnyObject = msg_send![class!(VNRecognizeTextRequest), alloc];
            let request: *mut AnyObject = msg_send![request, init];
            if request.is_null() {
                let _: () = msg_send![handler, release];
                return Vec::new();
            }
            let _: () = msg_send![request, setRecognitionLevel: RECOGNITION_LEVEL_ACCURATE];
            let _: () = msg_send![request, setUsesLanguageCorrection: true];

            // [handler performRequests:@[request] error:&err]
            let requests: *mut AnyObject = msg_send![class!(NSArray), arrayWithObject: request];
            let mut err: *mut AnyObject = std::ptr::null_mut();
            let _ok: bool = msg_send![handler, performRequests: requests, error: &mut err];

            // results: NSArray<VNRecognizedTextObservation>
            let mut lines = Vec::new();
            let results: *mut AnyObject = msg_send![request, results];
            if !results.is_null() {
                let count: usize = msg_send![results, count];
                for i in 0..count {
                    let obs: *mut AnyObject = msg_send![results, objectAtIndex: i];
                    if obs.is_null() {
                        continue;
                    }
                    // topCandidates: returns NSArray<VNRecognizedText>
                    let candidates: *mut AnyObject = msg_send![obs, topCandidates: 1_usize];
                    if candidates.is_null() {
                        continue;
                    }
                    let ccount: usize = msg_send![candidates, count];
                    if ccount == 0 {
                        continue;
                    }
                    let top: *mut AnyObject = msg_send![candidates, objectAtIndex: 0_usize];
                    let string: *mut NSString = msg_send![top, string];
                    let confidence: f32 = msg_send![top, confidence];
                    if string.is_null() {
                        continue;
                    }
                    let text = (*string).to_string();
                    if !text.trim().is_empty() {
                        lines.push(OcrLine { text, confidence });
                    }
                }
            }

            // Release the +1 objects we alloc'd (results/observations/candidates
            // are autoreleased and stay valid until the pool drains).
            let _: () = msg_send![request, release];
            let _: () = msg_send![handler, release];
            lines
        })
    }
}
