//! Capture lifecycle used by the native IPC adapter and standalone tests.
//! Standard library only; no Tauri runtime, devices, capture, or permissions.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::Mutex;
use std::thread::JoinHandle;

pub fn stop_and_drain(
    child: &mut Child,
    readers: &mut Vec<JoinHandle<()>>,
    request_stop: impl FnOnce(u32),
) -> Result<(), String> {
    // A helper may already have failed before Stop. Preserve its status;
    // never signal an already-reaped PID (which may have been reused).
    let mut status = child.try_wait().map_err(|e| e.to_string())?;
    if status.is_none() {
        request_stop(child.id());
        for _ in 0..30 {
            status = child.try_wait().map_err(|e| e.to_string())?;
            if status.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    if status.is_none() {
        child.kill().map_err(|e| e.to_string())?;
        child.wait().map_err(|e| e.to_string())?;
    }
    // EOF follows process exit. Return only after the final stdout path was registered/emitted.
    let mut reader_failed = false;
    for reader in readers.drain(..) {
        // A failed stderr reader must not prevent joining final stdout.
        reader_failed |= reader.join().is_err();
    }
    if reader_failed {
        return Err("capture output reader panicked; final audio may be incomplete".into());
    }
    match status {
        Some(status) if status.success() => Ok(()),
        Some(status) => Err(format!(
            "capture helper exited unsuccessfully ({status}); final audio may be incomplete"
        )),
        None => Err("capture helper timed out; final audio may be incomplete".into()),
    }
}

#[derive(Default)]
pub struct StopReceipt {
    pub unread_paths: Vec<String>,
    pub error: Option<String>,
}

pub fn receipt_after_stop(
    stopped: Result<(), String>,
    directory: &Path,
    chunks: &Mutex<BTreeSet<PathBuf>>,
) -> Result<StopReceipt, String> {
    let error = stopped.err();
    let chunks = chunks.lock().map_err(|e| e.to_string())?;
    let unread_paths = chunks
        .iter()
        .filter(|p| p.parent() == Some(directory))
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    Ok(StopReceipt {
        unread_paths,
        error,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::process::{Command, Stdio};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    fn exited(code: u8) -> Child {
        let mut child = Command::new("/bin/sh")
            .args(["-c", &format!("exit {code}")])
            .spawn()
            .unwrap();
        assert_eq!(child.wait().unwrap().code(), Some(code as i32));
        child
    }

    #[test]
    fn successful_exit_is_not_signaled_again() {
        let mut child = exited(0);
        stop_and_drain(&mut child, &mut Vec::new(), |_| {
            panic!("already exited PID signaled")
        })
        .unwrap();
    }

    #[test]
    fn failed_exit_preserves_registered_tail_and_rejects_other_capture_paths() {
        let directory = Path::new("/owned/capture");
        let chunks = Arc::new(Mutex::new(BTreeSet::from([PathBuf::from(
            "/other/capture/sysaudio_0000.wav",
        )])));
        let mut child = Command::new("/bin/sh")
            .args(["-c", "printf '/owned/capture/sysaudio_0002.wav\\n/owned/capture/sysaudio_0001.wav\\n'; exit 7"])
            .stdout(Stdio::piped()).spawn().unwrap();
        let output = child.stdout.take().unwrap();
        assert_eq!(child.wait().unwrap().code(), Some(7));
        let registry = chunks.clone();
        let mut readers = vec![std::thread::spawn(move || {
            for path in BufReader::new(output).lines() {
                registry
                    .lock()
                    .unwrap()
                    .insert(PathBuf::from(path.unwrap()));
            }
        })];
        let stopped = stop_and_drain(&mut child, &mut readers, |_| panic!("failed PID signaled"));
        let receipt = receipt_after_stop(stopped, directory, &chunks).unwrap();
        assert!(readers.is_empty());
        assert!(receipt.error.unwrap().contains("exited unsuccessfully"));
        assert_eq!(
            receipt.unread_paths,
            vec![
                "/owned/capture/sysaudio_0001.wav",
                "/owned/capture/sysaudio_0002.wav"
            ]
        );
    }

    #[test]
    fn graceful_stop_waits_for_real_child_tail_and_reader_completion() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "read stop; printf final-tail; exit 0"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        let mut output = child.stdout.take().unwrap();
        let drained = Arc::new(AtomicBool::new(false));
        let observed = drained.clone();
        let mut readers = vec![std::thread::spawn(move || {
            let mut tail = String::new();
            output.read_to_string(&mut tail).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
            observed.store(tail == "final-tail", Ordering::SeqCst);
        })];
        stop_and_drain(&mut child, &mut readers, |_| {
            input.write_all(b"stop\n").unwrap();
        })
        .unwrap();
        assert!(drained.load(Ordering::SeqCst));
        assert!(readers.is_empty());
    }

    #[test]
    fn reader_panic_still_joins_remaining_stdout_reader() {
        let mut child = exited(0);
        let drained = Arc::new(AtomicBool::new(false));
        let observed = drained.clone();
        let mut readers = vec![
            std::thread::spawn(|| panic!("synthetic reader error")),
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(20));
                observed.store(true, Ordering::SeqCst);
            }),
        ];
        let error = stop_and_drain(&mut child, &mut readers, |_| unreachable!()).unwrap_err();
        assert!(error.contains("reader panicked"));
        assert!(drained.load(Ordering::SeqCst));
        assert!(readers.is_empty());
    }

    #[test]
    fn timeout_kills_only_owned_child_and_reports_partial_audio() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "read stop"])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let _keep_input_open = child.stdin.take().unwrap();
        let error = stop_and_drain(&mut child, &mut Vec::new(), |_| {}).unwrap_err();
        assert!(error.contains("timed out"));
        assert!(child.try_wait().unwrap().is_some());
    }
}
