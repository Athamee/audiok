use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use windows::Win32::System::SystemInformation::GetLocalTime;

static FILE: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();
const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;
const MAX_LINE_CHARS: usize = 2000;

pub fn init() {
    let path = crate::config::log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    rotate_previous(&path, &crate::config::previous_log_path(), MAX_LOG_BYTES);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .ok()
        .map(Mutex::new);
    let _ = FILE.set(file);
    write_file("");
    emit(&format!(
        "===== session {} (pid {}) =====",
        timestamp(),
        std::process::id()
    ));
    log("app", &format!("démarrage (journal: {})", path.display()));
}

fn rotate_previous(current: &Path, previous: &Path, max: u64) {
    let keep = std::fs::metadata(current)
        .map(|meta| meta.len() > 0 && meta.len() <= max)
        .unwrap_or(false);
    if keep {
        let _ = std::fs::copy(current, previous);
    } else {
        let _ = std::fs::remove_file(previous);
    }
}

pub fn log(tag: &str, message: &str) {
    let tag = sanitize(tag);
    let message = sanitize(message);
    emit(&format!("[{tag}] {} {message}", timestamp()));
}

fn sanitize(value: &str) -> String {
    let mut text: String = value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let end = text
        .char_indices()
        .nth(MAX_LINE_CHARS)
        .map_or(text.len(), |(i, _)| i);
    text.truncate(end);
    text
}

fn emit(line: &str) {
    eprintln!("audiok: {line}");
    write_file(line);
}

fn write_file(line: &str) {
    if let Some(Some(file)) = FILE.get() {
        if let Ok(mut file) = file.lock() {
            rotate_if_needed(&mut file, line.len() + 1, MAX_LOG_BYTES);
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
        }
    }
}

fn rotate_if_needed(file: &mut std::fs::File, incoming: usize, max: u64) {
    let too_large = file
        .metadata()
        .map(|meta| meta.len() + incoming as u64 > max)
        .unwrap_or(false);
    if too_large {
        let _ = file.set_len(0);
        let _ = file.seek(SeekFrom::Start(0));
        let _ = file.write_all(b"===== journal recycle =====\n");
    }
}

fn timestamp() -> String {
    let time = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        time.wYear,
        time.wMonth,
        time.wDay,
        time.wHour,
        time.wMinute,
        time.wSecond,
        time.wMilliseconds
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_removes_control_characters_and_caps_length() {
        assert_eq!(sanitize("a\n[hotkey]\r\0"), "a [hotkey]  ");
        assert_eq!(sanitize(&"x".repeat(3000)).len(), MAX_LINE_CHARS);
    }

    #[test]
    fn startup_rotation_keeps_previous_session_and_drops_oversized_logs() {
        let dir = std::env::temp_dir();
        let current = dir.join(format!("audiok-log-current-{}.log", std::process::id()));
        let previous = dir.join(format!("audiok-log-previous-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&current);
        let _ = std::fs::remove_file(&previous);
        std::fs::write(&current, b"session precedente").unwrap();
        rotate_previous(&current, &previous, 100);
        assert_eq!(std::fs::read(&previous).unwrap(), b"session precedente");
        assert_eq!(std::fs::read(&current).unwrap(), b"session precedente");

        std::fs::write(&current, vec![b'x'; 101]).unwrap();
        rotate_previous(&current, &previous, 100);
        assert!(!previous.exists());
        let _ = std::fs::remove_file(&current);
        let _ = std::fs::remove_file(&previous);
    }

    #[test]
    fn rotate_truncates_when_limit_is_exceeded() {
        let path =
            std::env::temp_dir().join(format!("audiok-log-rotate-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .unwrap();
        file.write_all(b"aaaa").unwrap();
        rotate_if_needed(&mut file, 4, 6);
        file.flush().unwrap();
        drop(file);
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "===== journal recycle =====\n");
        let _ = std::fs::remove_file(&path);
    }
}
