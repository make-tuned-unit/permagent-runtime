//! Cheap file availability probe. Does not load a model or run inference.
use std::{fs::File, io::Read, path::Path};

pub(super) fn model_file_reason(path: &Path) -> &'static str {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() && metadata.len() > 4 => {}
        Ok(_) => return "invalid_model",
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return "missing_model",
        Err(_) => return "unreadable_model",
    }
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return "missing_model",
        Err(_) => return "unreadable_model",
    };
    let mut magic = [0; 4];
    match file.read_exact(&mut magic) {
        Ok(()) if magic == *b"GGUF" => "model_available",
        Ok(()) => "invalid_model",
        Err(_) => "unreadable_model",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn probe_distinguishes_missing_empty_invalid_and_available_without_mutation() {
        let root = std::env::temp_dir().join(format!(
            "dictation-status-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let file = root.join("model.gguf");
        assert_eq!(model_file_reason(&file), "missing_model");
        for (bytes, reason) in [
            (b"".as_slice(), "invalid_model"),
            (b"not a model", "invalid_model"),
            (b"GGUFfixture", "model_available"),
        ] {
            std::fs::write(&file, bytes).unwrap();
            assert_eq!(model_file_reason(&file), reason);
            assert_eq!(std::fs::read(&file).unwrap(), bytes);
        }
        assert_eq!(model_file_reason(&root), "invalid_model");
        std::fs::remove_dir_all(root).unwrap();
    }
}
