//! UI bus: single event queue driving the interactive main loop.
//!
//! All interactive event sources — crossterm input, the session event bus,
//! and the animation/housekeeping tick — are forwarded into one channel so
//! the main loop can block on `recv()` instead of polling at 30 fps, then
//! drain a batch and render at most once per batch.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use crossterm::event;

use crate::core::event::SessionEvent;

/// Events on the UI bus.
pub(super) enum UiEvent {
    /// Raw crossterm input (key / mouse / paste / resize).
    Input(event::Event),
    /// An event from the session event bus (worker, sub-agent, background).
    Session(Box<SessionEvent>),
    /// Animation + housekeeping tick (spinner, toast expiry, sidebar poll).
    Tick,
}

pub(super) struct UiBus {
    pub(super) rx: mpsc::Receiver<UiEvent>,
    /// Mirror of `ai_running`; the tick thread speeds up while set.
    animate: Arc<AtomicBool>,
}

impl UiBus {
    /// Spawn the producer threads: input reader, session-bus forwarder, and
    /// the tick generator. `session_rx` is the TUI-owned end of the session
    /// event bus; `shutdown` stops the forwarder and tick threads.
    pub(super) fn spawn(
        session_rx: mpsc::Receiver<SessionEvent>,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<UiEvent>();
        let animate = Arc::new(AtomicBool::new(false));

        // Crossterm input reader. Blocks on read(); the thread dies with the
        // process when the TUI exits.
        {
            let tx = tx.clone();
            std::thread::spawn(move || {
                loop {
                    match event::read() {
                        Ok(ev) => {
                            if tx.send(UiEvent::Input(ev)).is_err() {
                                return;
                            }
                        }
                        Err(_) => return,
                    }
                }
            });
        }

        // Session bus forwarder.
        {
            let tx = tx.clone();
            let shutdown = Arc::clone(&shutdown);
            std::thread::spawn(move || {
                while !shutdown.load(Ordering::SeqCst) {
                    match session_rx.recv_timeout(Duration::from_millis(100)) {
                        Ok(ev) => {
                            if tx.send(UiEvent::Session(Box::new(ev))).is_err() {
                                return;
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
            });
        }

        // Tick generator: 16 ms (~60 Hz) while animating (tool spinner,
        // streaming), 500 ms when idle (toast expiry, sidebar poll).
        {
            let animate = Arc::clone(&animate);
            std::thread::spawn(move || {
                loop {
                    let interval = if animate.load(Ordering::SeqCst) {
                        Duration::from_millis(16)
                    } else {
                        Duration::from_millis(500)
                    };
                    std::thread::sleep(interval);
                    if tx.send(UiEvent::Tick).is_err() {
                        return;
                    }
                }
            });
        }

        Self { rx, animate }
    }

    /// Mirror the animation state into the tick thread.
    pub(super) fn set_animate(&self, animating: bool) {
        self.animate.store(animating, Ordering::SeqCst);
    }

    /// Block until the next event arrives.
    pub(super) fn recv(&self) -> Option<UiEvent> {
        self.rx.recv().ok()
    }

    /// Drain everything currently queued (batch coalescing).
    pub(super) fn drain(&self) -> Vec<UiEvent> {
        let mut batch = Vec::new();
        while let Ok(ev) = self.rx.try_recv() {
            batch.push(ev);
        }
        batch
    }
}
