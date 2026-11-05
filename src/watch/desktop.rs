//! What the watch pipeline needs from the desktop: the focused field, the
//! keys it presses, and the lock that keeps one watch action at a time on
//! the keyboard (Swift's `FieldPaster.exclusive`). `Screen` is the seam: the
//! macOS adapter (`watch::screen`) reads Accessibility and posts key events;
//! tests answer from a script.
//!
//! The `watch` thread never calls `Screen`. It hands `/focus` captures and
//! `/enter` and `/undo` presses to `Desktop`'s own `watch-ax` thread, which
//! answers through a callback, so a hung application costs one reply its
//! deadline instead of the protocol its clock.

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use super::field::{Expectation, FieldSnapshot, Focused, KeyGate, key_gate};

/// Reads and keys, all blocking. Accessibility reads are bounded by the
/// messaging timeout on each element.
pub(crate) trait Screen: Send + Sync {
    /// Hex may use Accessibility.
    fn trusted(&self) -> bool;
    /// Secure keyboard entry is on: no app receives synthetic keys.
    fn secure_input(&self) -> bool;
    /// The frontmost application's focused element.
    fn focused(&self) -> Option<Focused>;
    fn press_return(&self) -> Result<(), String>;
    /// ⌘Z.
    fn press_undo(&self) -> Result<(), String>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Key {
    Enter,
    Undo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum KeyOutcome {
    Pressed,
    /// The focus is no longer the recording's field.
    TargetChanged,
    /// A password field or secure keyboard entry.
    Sensitive,
    Failed(String),
}

type Done<T> = Box<dyn FnOnce(T) + Send>;

/// The keyboard lock. Owned, so a read-back thread can keep holding it after
/// the paste that took it returns.
#[derive(Default)]
struct Exclusive {
    held: Mutex<bool>,
    freed: Condvar,
}

impl Exclusive {
    fn acquire(self: &Arc<Self>) -> Held {
        let mut held = self.held.lock().unwrap_or_else(|error| error.into_inner());
        while *held {
            held = self
                .freed
                .wait(held)
                .unwrap_or_else(|error| error.into_inner());
        }
        *held = true;
        Held(Arc::clone(self))
    }
}

/// Proof that no other watch action is on the keyboard; released on drop.
pub(crate) struct Held(Arc<Exclusive>);

impl Drop for Held {
    fn drop(&mut self) {
        let mut held = self
            .0
            .held
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *held = false;
        self.0.freed.notify_one();
    }
}

/// One key request's claim on the keyboard. The service cancels it when it
/// gives up on the request (timeout, a recording that started over), and the
/// `watch-ax` thread checks it inside the lock, so a key that waited for a
/// paste's read-back is not pressed for a request that no longer stands.
#[derive(Clone, Debug)]
pub(crate) struct Ticket {
    /// Which operation this is, so an answer is matched to its own request.
    op: u64,
    live: Arc<std::sync::atomic::AtomicBool>,
}

impl Ticket {
    pub(crate) fn new(op: u64) -> Self {
        Self {
            op,
            live: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        }
    }

    pub(crate) fn op(&self) -> u64 {
        self.op
    }

    pub(crate) fn cancel(&self) {
        self.live.store(false, std::sync::atomic::Ordering::Release);
    }

    pub(crate) fn is_live(&self) -> bool {
        self.live.load(std::sync::atomic::Ordering::Acquire)
    }
}

// A job is built, queued, and run once; boxing the snapshot would only add
// an allocation per key press.
#[allow(clippy::large_enum_variant)]
enum Job {
    Capture(Done<Option<FieldSnapshot>>),
    Press {
        key: Key,
        expected: FieldSnapshot,
        /// The reply was given up on at this moment; a key pressed later
        /// would act on a request the watch was told had failed.
        valid_until: Instant,
        ticket: Ticket,
        done: Done<KeyOutcome>,
    },
}

enum Runner {
    Thread(Sender<Job>),
    /// Runs each job on the caller's thread: deterministic tests.
    #[cfg(test)]
    Inline,
    /// Drops every job unanswered: an application that never responds.
    #[cfg(test)]
    Silent,
    /// Holds jobs until `run_pending`: a key that is slow to reach the app.
    #[cfg(test)]
    Manual(Mutex<Vec<Job>>),
}

/// The handle the service, the output worker, and the jobs share. Cloning
/// shares the screen, the lock, and the `watch-ax` thread.
#[derive(Clone)]
pub(crate) struct Desktop {
    screen: Arc<dyn Screen>,
    exclusive: Arc<Exclusive>,
    runner: Arc<Runner>,
}

impl std::fmt::Debug for Desktop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Desktop")
    }
}

impl PartialEq for Desktop {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.exclusive, &other.exclusive)
    }
}

impl Desktop {
    /// Starts the `watch-ax` thread. It ends when the last clone drops.
    pub(crate) fn start(screen: Arc<dyn Screen>) -> std::io::Result<Self> {
        let (jobs, inbox) = mpsc::channel::<Job>();
        let exclusive = Arc::new(Exclusive::default());
        let worker_screen = Arc::clone(&screen);
        let worker_lock = Arc::clone(&exclusive);
        std::thread::Builder::new()
            .name("watch-ax".to_owned())
            .spawn(move || {
                for job in inbox {
                    run(worker_screen.as_ref(), &worker_lock, job);
                }
            })?;
        Ok(Self {
            screen,
            exclusive,
            runner: Arc::new(Runner::Thread(jobs)),
        })
    }

    #[cfg(test)]
    pub(crate) fn inline(screen: Arc<dyn Screen>) -> Self {
        Self {
            screen,
            exclusive: Arc::new(Exclusive::default()),
            runner: Arc::new(Runner::Inline),
        }
    }

    #[cfg(test)]
    pub(crate) fn manual(screen: Arc<dyn Screen>) -> Self {
        Self {
            screen,
            exclusive: Arc::new(Exclusive::default()),
            runner: Arc::new(Runner::Manual(Mutex::new(Vec::new()))),
        }
    }

    /// Runs the jobs a manual desktop held, in order.
    #[cfg(test)]
    pub(crate) fn run_pending(&self) {
        let Runner::Manual(held) = self.runner.as_ref() else {
            return;
        };
        let jobs = std::mem::take(&mut *held.lock().unwrap());
        for job in jobs {
            run(self.screen.as_ref(), &self.exclusive, job);
        }
    }

    #[cfg(test)]
    pub(crate) fn silent(screen: Arc<dyn Screen>) -> Self {
        Self {
            screen,
            exclusive: Arc::new(Exclusive::default()),
            runner: Arc::new(Runner::Silent),
        }
    }

    /// A desktop that sees nothing, for tests that never reach the screen.
    #[cfg(test)]
    pub(crate) fn null() -> Self {
        Self::inline(Arc::new(fake::FakeScreen::default()))
    }

    pub(crate) fn screen(&self) -> &dyn Screen {
        self.screen.as_ref()
    }

    pub(crate) fn trusted(&self) -> bool {
        self.screen.trusted()
    }

    /// Held while a paste is sent and read back, and while a key is
    /// pressed: one action at a time on the keyboard.
    pub(crate) fn lock(&self) -> Held {
        self.exclusive.acquire()
    }

    /// Reads the focused field for `/focus`; `None` when nothing is focused
    /// or Accessibility is not granted.
    pub(crate) fn capture(&self, done: impl FnOnce(Option<FieldSnapshot>) + Send + 'static) {
        self.submit(Job::Capture(Box::new(done)));
    }

    /// Presses `key` in the field `expected` names, if it still has focus.
    pub(crate) fn press(
        &self,
        key: Key,
        expected: FieldSnapshot,
        valid_until: Instant,
        ticket: Ticket,
        done: impl FnOnce(KeyOutcome) + Send + 'static,
    ) {
        self.submit(Job::Press {
            key,
            expected,
            valid_until,
            ticket,
            done: Box::new(done),
        });
    }

    fn submit(&self, job: Job) {
        match self.runner.as_ref() {
            Runner::Thread(jobs) => {
                if let Err(mpsc::SendError(job)) = jobs.send(job) {
                    // The thread ended: answer as a screen that sees nothing.
                    run(self.screen.as_ref(), &self.exclusive, job);
                }
            }
            #[cfg(test)]
            Runner::Inline => run(self.screen.as_ref(), &self.exclusive, job),
            #[cfg(test)]
            Runner::Silent => drop(job),
            #[cfg(test)]
            Runner::Manual(held) => held.lock().unwrap().push(job),
        }
    }
}

fn run(screen: &dyn Screen, exclusive: &Arc<Exclusive>, job: Job) {
    match job {
        Job::Capture(done) => {
            let snapshot = if screen.trusted() {
                screen.focused().map(|focused| focused.field)
            } else {
                None
            };
            done(snapshot);
        }
        Job::Press {
            key,
            expected,
            valid_until,
            ticket,
            done,
        } => done(press(
            screen,
            exclusive,
            key,
            &expected,
            valid_until,
            &ticket,
        )),
    }
}

/// Swift's `pressEnter` and `undo`: refuse under secure input, take the
/// lock, and only then compare the focus with the recording's field.
fn press(
    screen: &dyn Screen,
    exclusive: &Arc<Exclusive>,
    key: Key,
    expected: &FieldSnapshot,
    valid_until: Instant,
    ticket: &Ticket,
) -> KeyOutcome {
    if screen.secure_input() {
        return KeyOutcome::Sensitive;
    }
    if !screen.trusted() {
        return KeyOutcome::Failed("Accessibility permission is not granted".to_owned());
    }
    let _exclusive = exclusive.acquire();
    // The wait for the lock may have outlasted the request.
    if Instant::now() >= valid_until || !ticket.is_live() {
        return KeyOutcome::Failed("the request ended before the key was pressed".to_owned());
    }
    // Read twice: reading the focus can be slow, and the user may have
    // switched apps meanwhile. The second read, the deadline, and the ticket
    // are all checked right before the key, as close as the API allows (a
    // switch inside those few microseconds is still possible).
    let live = || Instant::now() < valid_until && ticket.is_live();
    for _ in 0..2 {
        let focused = screen.focused();
        let gate = key_gate(
            &Expectation::Field(expected.clone()),
            focused.as_ref().map(|focused| &focused.field),
            screen.secure_input(),
        );
        match gate {
            KeyGate::TargetChanged => return KeyOutcome::TargetChanged,
            KeyGate::Sensitive => return KeyOutcome::Sensitive,
            KeyGate::Press => {}
        }
        if !live() {
            return KeyOutcome::Failed("the request ended before the key was pressed".to_owned());
        }
    }
    let sent = match key {
        Key::Enter => screen.press_return(),
        Key::Undo => screen.press_undo(),
    };
    match sent {
        Ok(()) => KeyOutcome::Pressed,
        Err(error) => KeyOutcome::Failed(error),
    }
}

#[cfg(test)]
pub(crate) mod fake;
#[cfg(test)]
mod tests;
