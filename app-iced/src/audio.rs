//! In-app voice-note playback.
//!
//! iced runs the whole application (update + view) on a single thread, and
//! rodio's `OutputStream`/`Sink` are not `Send`. Keeping them in a
//! `thread_local!` means the UI thread *owns* the audio device, so play /
//! pause / stop are trivial and progress can be queried per frame. The
//! timer-driven tick polls `finished()` so completion clears the UI state.

use rodio::{Decoder, OutputStream, Sink};
use std::cell::RefCell;
use std::fs::File;
use std::io::BufReader;
use std::time::Instant;

thread_local! {
    /// The live audio device + sink, held by the UI thread for as long as one
    /// voice note is playing. `None` while idle (and on machines with no audio
    /// output, so the app still runs).
    static PLAYER: RefCell<Option<Player>> = const { RefCell::new(None) };
}

struct Player {
    sink: Sink,
    started: Instant,
    _stream: OutputStream,
}

fn with_player<R>(f: impl FnOnce(&mut Option<Player>) -> R) -> R {
    PLAYER.with(|p| f(&mut p.borrow_mut()))
}

/// True if a voice note is currently loaded (playing or paused).
pub fn is_active() -> bool {
    with_player(|p| p.is_some())
}

/// Start playing `path`, replacing whatever was playing. Returns `false` if
/// there is no usable audio output or the file can't be decoded (the UI treats
/// it as a silent no-op, never a crash).
pub fn play(path: &str) -> bool {
    let started = Instant::now();
    let Ok((stream, handle)) = OutputStream::try_default() else {
        return false;
    };
    let Ok(sink) = Sink::try_new(&handle) else {
        return false;
    };
    let Ok(file) = File::open(path) else {
        return false;
    };
    let Ok(src) = Decoder::new(BufReader::new(file)) else {
        return false;
    };
    sink.append(src);
    sink.play();
    with_player(|p| {
        if let Some(old) = p.take() {
            old.sink.stop();
        }
        *p = Some(Player {
            sink,
            started,
            _stream: stream,
        });
    });
    true
}

/// True when the current note has played to its end (the UI stops and clears).
/// Idle (`is_active() == false`) is not "finished".
pub fn finished() -> bool {
    with_player(|p| p.as_ref().is_some_and(|pl| pl.sink.empty()))
}

/// Seconds elapsed on the current note (0.0 when idle).
pub fn elapsed_secs() -> f32 {
    with_player(|p| match p {
        Some(pl) => pl.started.elapsed().as_secs_f32(),
        None => 0.0,
    })
}

/// Pause the current note.
pub fn pause() {
    with_player(|p| {
        if let Some(pl) = p {
            pl.sink.pause();
        }
    });
}

/// Resume the paused note.
pub fn resume() {
    with_player(|p| {
        if let Some(pl) = p {
            pl.sink.play();
        }
    });
}

/// Stop the current note and release the device.
pub fn stop() {
    with_player(|p| {
        if let Some(pl) = p.take() {
            pl.sink.stop();
        }
    });
}