//! Guard against the stray Enter key event some terminals (Windows Terminal /
//! conhost) deliver immediately after a bracketed paste that ends with a
//! newline. Without this, pasting multi-line text submitted the chat (#544).
//!
//! Paste events and key events are both handled on the TUI event-loop thread,
//! so a thread-local timestamp is sufficient and keeps `App` untouched.

use std::cell::Cell;
use std::time::{Duration, Instant};

const PASTE_ENTER_SUPPRESS_WINDOW: Duration = Duration::from_millis(150);

thread_local! {
    static LAST_PASTE: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Record that a bracketed-paste event was just handled.
pub(super) fn note_paste() {
    LAST_PASTE.with(|cell| cell.set(Some(Instant::now())));
}

/// Returns true (and consumes the marker) when a bare Enter arrives within the
/// suppression window after a paste, meaning it belongs to the paste rather
/// than being a user submit.
pub(super) fn consume_paste_trailing_enter() -> bool {
    LAST_PASTE.with(|cell| {
        cell.take()
            .is_some_and(|at| at.elapsed() < PASTE_ENTER_SUPPRESS_WINDOW)
    })
}

// 브래킷 붙여넣기 없이 키 입력 연속으로 들어오는 붙여넣기 감지 (#928).
// 사람이 칠 수 없는 속도(BURST_WINDOW 안에 BURST_CHARS자 초과)로 문자가 들어오면
// 잠시 동안 Enter를 전송이 아니라 줄바꿈으로 본다.
const BURST_WINDOW: Duration = Duration::from_millis(100);
const BURST_CHARS: usize = 8;
const BURST_ENTER_GRACE: Duration = Duration::from_millis(50);
const ENTER_AFTER_CHAR_GAP: Duration = Duration::from_millis(8);

thread_local! {
    static RECENT_CHARS: std::cell::RefCell<std::collections::VecDeque<Instant>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
    static BURST_UNTIL: Cell<Option<Instant>> = const { Cell::new(None) };
    // 직전 키 입력이 ASCII였는지. 한글 IME는 확정 글자와 Enter를 거의 동시에 보내므로
    // ENTER_AFTER_CHAR_GAP 규칙은 ASCII 직후에만 쓴다.
    static LAST_CHAR_ASCII: Cell<bool> = const { Cell::new(false) };
}

/// 한 글자짜리 키 입력 이벤트 수를 기록하고, 붙여넣기 속도면 감지 창을 연다.
pub(super) fn note_typed_chars(text: &str) {
    // 테스트는 키를 순식간에 주입하므로, 켜 두면 "입력 후 Enter 전송" 테스트가
    // 붙여넣기로 오인된다. 판정 로직은 아래 burst_tests에서 시각을 넣어 검증한다.
    if cfg!(test) {
        return;
    }
    note_typed_chars_at(text, Instant::now());
}

fn note_typed_chars_at(text: &str, now: Instant) {
    let count = text.chars().count();
    if let Some(last) = text.chars().last() {
        LAST_CHAR_ASCII.with(|cell| cell.set(last.is_ascii()));
    }
    RECENT_CHARS.with(|recent| {
        let mut recent = recent.borrow_mut();
        while recent
            .front()
            .is_some_and(|at| now.duration_since(*at) > BURST_WINDOW)
        {
            recent.pop_front();
        }
        recent.extend(std::iter::repeat_n(now, count));
        if recent.len() > BURST_CHARS {
            BURST_UNTIL.with(|until| until.set(Some(now + BURST_ENTER_GRACE)));
            // 메모리가 계속 늘지 않게 판정에 필요한 만큼만 남긴다.
            while recent.len() > BURST_CHARS + 1 {
                recent.pop_front();
            }
        }
    });
}

/// 붙여넣기 연속 입력 중의 Enter면 true. 빈 줄이 이어져도 유지되도록 창을 늘린다.
pub(super) fn enter_in_key_stream_paste() -> bool {
    enter_in_key_stream_paste_at(Instant::now())
}

fn enter_in_key_stream_paste_at(now: Instant) -> bool {
    // 첫 줄이 짧아(BURST_CHARS 이하) 감지 창이 아직 안 열렸어도, 직전 글자 직후
    // 사람이 누를 수 없는 간격으로 들어온 Enter는 붙여넣기의 줄바꿈으로 본다.
    let right_after_char = LAST_CHAR_ASCII.with(Cell::get)
        && RECENT_CHARS.with(|recent| {
            recent
                .borrow()
                .back()
                .is_some_and(|at| now.duration_since(*at) <= ENTER_AFTER_CHAR_GAP)
        });
    BURST_UNTIL.with(|until| {
        let active = right_after_char || until.get().is_some_and(|at| now <= at);
        if active {
            until.set(Some(now + BURST_ENTER_GRACE));
        } else {
            until.set(None);
        }
        active
    })
}

/// Test hook: age the recorded paste so a subsequent Enter submits normally.
#[cfg(test)]
pub(in crate::tui::app) fn expire_for_test() {
    LAST_PASTE.with(|cell| cell.set(None));
    RECENT_CHARS.with(|recent| recent.borrow_mut().clear());
    BURST_UNTIL.with(|until| until.set(None));
    LAST_CHAR_ASCII.with(|cell| cell.set(false));
}

#[cfg(test)]
mod burst_tests {
    use super::*;

    #[test]
    fn fast_key_stream_turns_enter_into_newline() {
        expire_for_test();
        let start = Instant::now();
        for i in 0..12 {
            note_typed_chars_at("a", start + Duration::from_millis(i));
        }
        assert!(enter_in_key_stream_paste_at(
            start + Duration::from_millis(20)
        ));
        // 빈 줄(연속 Enter)도 줄바꿈으로 유지된다.
        assert!(enter_in_key_stream_paste_at(
            start + Duration::from_millis(25)
        ));
        expire_for_test();
    }

    #[test]
    fn human_typing_still_submits() {
        expire_for_test();
        let start = Instant::now();
        for i in 0..20 {
            note_typed_chars_at("a", start + Duration::from_millis(i * 80));
        }
        assert!(!enter_in_key_stream_paste_at(
            start + Duration::from_millis(1700)
        ));
        expire_for_test();
    }

    #[test]
    fn short_first_line_paste_does_not_submit() {
        // Codex 검토 지적: `abc\ndef`는 Enter 시점에 3자뿐이라 감지 창이 안 열린다.
        expire_for_test();
        let start = Instant::now();
        for i in 0..3 {
            note_typed_chars_at("a", start + Duration::from_millis(i));
        }
        assert!(enter_in_key_stream_paste_at(
            start + Duration::from_millis(3)
        ));
        expire_for_test();
    }

    #[test]
    fn korean_ime_commit_then_enter_submits() {
        // 한글 IME는 확정 글자와 Enter가 거의 동시에 온다. 짧은 한글 입력의 Enter는 전송.
        expire_for_test();
        let start = Instant::now();
        note_typed_chars_at("한", start);
        assert!(!enter_in_key_stream_paste_at(
            start + Duration::from_millis(1)
        ));
        expire_for_test();
    }

    #[test]
    fn human_enter_after_short_word_submits() {
        expire_for_test();
        let start = Instant::now();
        for i in 0..3 {
            note_typed_chars_at("a", start + Duration::from_millis(i * 90));
        }
        // 마지막 글자(180ms) 후 사람 속도로 누른 Enter
        assert!(!enter_in_key_stream_paste_at(
            start + Duration::from_millis(300)
        ));
        expire_for_test();
    }

    #[test]
    fn enter_after_burst_grace_submits() {
        expire_for_test();
        let start = Instant::now();
        for i in 0..12 {
            note_typed_chars_at("a", start + Duration::from_millis(i));
        }
        assert!(!enter_in_key_stream_paste_at(
            start + Duration::from_millis(500)
        ));
        expire_for_test();
    }
}

/// Test hook: bytes of a real 1x1 PNG, for drop tests that need content
/// `load_dropped_image` accepts.
#[cfg(test)]
pub(crate) fn tiny_png_bytes_for_test() -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(1, 1, image::Rgb([1, 2, 3])))
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("encode png");
    out.into_inner()
}

/// True for file extensions drag-and-drop paste treats as images.
fn has_image_extension(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tif" | "tiff"
            )
        })
}

/// Load a dropped image file as a provider-safe `(media_type, bytes)` pair.
///
/// The extension only decides whether to try. The media type comes from the
/// bytes, and BMP/ICO/TIFF are converted to PNG, because an unsupported image
/// block in history makes every later request fail (#1712). Returns `None`
/// when the file is not an image the model can view, so the caller falls back
/// to inserting the path as text.
pub(super) fn load_dropped_image(path: &std::path::Path) -> Option<(String, Vec<u8>)> {
    if !has_image_extension(path) {
        return None;
    }
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(error) => {
            crate::logging::info(&format!(
                "Dropped image {} could not be read, inserting the path instead: {error}",
                path.display()
            ));
            return None;
        }
    };
    match crate::image_normalize::normalize_image_bytes(data) {
        Ok(image) => Some((image.media_type.to_string(), image.data)),
        Err(reason) => {
            crate::logging::info(&format!(
                "dropped file {} not attached as image: {reason}",
                path.display()
            ));
            None
        }
    }
}
