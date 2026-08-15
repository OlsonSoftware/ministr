//! ministr-tui — the terminal console for the ministr index engine.
//!
//! Opened with `ministr ui`. This crate owns terminal setup/teardown
//! (raw mode, alternate screen, panic-hook restore), the event loop, the
//! engine plumbing, and every frame. The binding design direction is
//! GUI-BLUEPRINT-v8 (Master-Control Console). The home screen is S1 —
//! The Console ([`console`]): one channel strip per project beside a
//! master section, rendered through the rendering infrastructure of
//! Amendment A: synchronized frames ([`sync`]), the color capability
//! ladder ([`palette`]), adaptive frame pacing ([`pacing`]), lawful
//! transitions ([`motion`]), and the sub-cell meter ([`meter`]).
//!
//! Language rule (blueprint §4): every user-facing string lives in
//! [`strings`] and says project / engine / rebuild / needs update — never
//! the internal vocabulary. `tests/language.rs` enforces this mechanically
//! over all string literals in this crate.

pub mod app;
pub mod console;
pub mod detail;
pub mod ease;
pub mod engine;
pub mod event;
pub mod field;
pub mod lawn;
pub mod meter;
pub mod motion;
pub mod pacing;
pub mod palette;
pub mod patchin;
pub mod strings;
pub mod sync;

use std::sync::Arc;
use std::time::{Duration, Instant};

use ministr_api::client::DaemonClient;
use ratatui::DefaultTerminal;

use crate::app::App;
use crate::pacing::Pacer;

/// How often the frame re-probes the engine for machine state.
const PROBE_INTERVAL: Duration = Duration::from_secs(2);

/// How often the live meters poll the engine's progress counters —
/// only while something is building; at rest this poll never runs.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(500);

/// Errors the console can exit with.
#[derive(Debug, thiserror::Error)]
pub enum UiError {
    /// Drawing or terminal control failed.
    #[error("terminal error: {0}")]
    Io(#[from] std::io::Error),
}

/// Open the console, run until quit, restore the terminal.
///
/// The terminal is restored on the quit path here and on the panic path
/// by the hook `ratatui::init` installs (it puts the terminal back before
/// the panic message prints, so a crash never leaves the shell raw).
///
/// # Errors
///
/// Returns [`UiError::Io`] when terminal control or drawing fails.
pub async fn run() -> Result<(), UiError> {
    // One client, shared by the loop and every background task — the
    // console speaks to exactly one engine.
    let client = Arc::new(DaemonClient::new());
    // The engine handshake (spawn if missing, wait until it answers)
    // runs in the background: the console opens at once and the title
    // reads "starting…" honestly until the engine is up — a cold
    // machine never stares at a blank shell.
    let spawning = tokio::spawn({
        let client = Arc::clone(&client);
        async move {
            engine::ensure_engine(&client).await;
        }
    });

    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &client, spawning).await;
    ratatui::restore();
    result
}

/// Draw when something changed, wait otherwise, repeat until quit.
///
/// Pacing is adaptive (GUI-BLUEPRINT-v8 Amendment A): at rest the loop
/// wakes on input and probe answers but draws nothing; while a
/// transition plays it draws at the frame clock, feeding effects real
/// elapsed time. Every draw goes through [`sync::draw_synced`].
///
/// Topology rule: the loop itself never awaits the engine. Every fetch
/// — probe, detail, progress, lawn — and every verb runs on its own
/// task and answers through a channel, so a slow or hung engine can
/// never hold up a frame or a key press.
async fn event_loop(
    terminal: &mut DefaultTerminal,
    client: &Arc<DaemonClient>,
    mut spawning: tokio::task::JoinHandle<()>,
) -> Result<(), UiError> {
    let mut app = App::new();
    let mut events = event::spawn_reader();
    let (mut probe_timer, mut progress_timer) = timers();
    let mut spawn_done = false;
    let mut pacer = Pacer::new();
    let mut last_frame = Instant::now();
    // Verbs run on their own task and answer through this channel — a
    // slow engine answer (a big project registering, a rebuild
    // purging) must never hold up a frame or a key press.
    let (verb_tx, mut verb_rx) = tokio::sync::mpsc::channel::<engine::Outcome>(1);
    let mut verb_running = false;
    // Lawn fetches (per-file freshness + mtimes) ride their own tasks
    // for the same reason — a big project's file list must never hold
    // up a frame. One in flight per project.
    let (lawn_tx, mut lawn_rx) =
        tokio::sync::mpsc::channel::<(String, engine::FreshSig, Option<lawn::Lawn>)>(8);
    let mut lawn_fetching: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Probes ride a background task for the same reason: the timer only
    // spawns the fetch, the answer comes back here. One in flight; a
    // tick landing mid-flight queues exactly one follow-up, so a verb's
    // immediate re-probe is never lost to the guard.
    let (probe_tx, mut probe_rx) =
        tokio::sync::mpsc::channel::<(engine::EngineState, Option<detail::Facts>)>(1);
    let mut probe_inflight = false;
    let mut probe_queued = false;
    // The fast progress poll rides its own task too.
    let (progress_tx, mut progress_rx) =
        tokio::sync::mpsc::channel::<Vec<engine::ProgressTarget>>(1);
    let mut progress_inflight = false;

    loop {
        pacer.set_animating(app.animating(Instant::now()));
        if pacer.take_redraw() {
            let now = Instant::now();
            let delta = now.duration_since(last_frame).min(pacing::MAX_FRAME_DELTA);
            last_frame = now;
            // Stdout is the same stream the default backend writes to,
            // so the sync sequences bracket the frame bytes in order.
            sync::draw_synced(&mut std::io::stdout(), terminal, |frame| {
                app.draw(frame, delta, now);
            })?;
        }

        tokio::select! {
            ev = events.recv() => match ev {
                Some(event::Event::Key(key)) => {
                    if app.on_key(key) {
                        pacer.mark_dirty();
                    }
                }
                Some(event::Event::Resize) => pacer.mark_dirty(),
                Some(event::Event::Tick) => {}
                // The reader thread is gone; without input the console
                // can only be quit from outside, so leave cleanly.
                None => app.should_quit = true,
            },
            _ = probe_timer.tick() => {
                if probe_inflight {
                    probe_queued = true;
                } else {
                    probe_inflight = true;
                    spawn_probe(client, &probe_tx, app.detail_id());
                }
            }
            // A probe answered on its background task.
            Some((state, facts)) = probe_rx.recv() => {
                probe_inflight = false;
                if probe_queued {
                    probe_queued = false;
                    probe_timer.reset_immediately();
                }
                if absorb_probe_answer(&mut app, state, facts, spawn_done) {
                    pacer.mark_dirty();
                }
            }
            // The engine handshake finished: probe at once so the
            // title flips the moment the engine is really up.
            _ = &mut spawning, if !spawn_done => {
                spawn_done = true;
                probe_timer.reset_immediately();
            }
            // The fast progress poll: armed only while something is
            // building, so the rest state never polls it.
            _ = progress_timer.tick(), if app.building() && !progress_inflight => {
                progress_inflight = true;
                spawn_progress_poll(client, &progress_tx);
            }
            // A progress poll answered on its background task.
            Some(targets) = progress_rx.recv() => {
                progress_inflight = false;
                if app.absorb_progress(&targets, Instant::now()) {
                    pacer.mark_dirty();
                }
            }
            // A verb finished on its background task: apply what it
            // left behind, and probe at once when the machine changed
            // shape so the transition plays without waiting out the
            // probe interval.
            Some(outcome) = verb_rx.recv() => {
                verb_running = false;
                if app.absorb_outcome(outcome) {
                    probe_timer.reset_immediately();
                }
                pacer.mark_dirty();
            }
            // A lawn fetch landed: absorb it (the signature is
            // recorded even for a failed fetch, so a miss never spins
            // the fetcher).
            Some((id, sig, fetched)) = lawn_rx.recv() => {
                lawn_fetching.remove(&id);
                if app.absorb_lawn(&id, sig, fetched) {
                    pacer.mark_dirty();
                }
            }
            // The frame clock: armed only while a transition plays or
            // a needle glides, so the rest state stays event-driven.
            () = tokio::time::sleep(pacing::FRAME_INTERVAL), if pacer.is_animating() => {}
        }

        // A strip's freshness signature moved past its cached lawn:
        // fetch the fresh per-file picture in the background.
        spawn_lawn_fetches(&app, &mut lawn_fetching, &lawn_tx, client);

        // A key press queued a verb: hand it to a background task (one
        // at a time — a second queued verb waits its turn) and say
        // what is running on the foot row while it works.
        if !verb_running && let Some(action) = app.pending.take() {
            verb_running = true;
            app.working = action.working_word();
            pacer.mark_dirty();
            spawn_verb(client, &verb_tx, action);
        }

        if app.should_quit {
            return Ok(());
        }
    }
}

/// The probe and progress clocks. `interval` fires immediately, so the
/// first frame after the loop opens already carries a real probe result
/// instead of a stuck "starting". While nothing builds the progress
/// timer is not polled and its ticks go missing; skipping them keeps a
/// build's first poll from replaying the whole quiet spell as a burst.
fn timers() -> (tokio::time::Interval, tokio::time::Interval) {
    let probe_timer = tokio::time::interval(PROBE_INTERVAL);
    let mut progress_timer = tokio::time::interval(PROGRESS_INTERVAL);
    progress_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    (probe_timer, progress_timer)
}

/// Spawn one queued verb on its background task — a slow engine answer
/// (a big project registering, a rebuild purging) must never hold up a
/// frame or a key press.
fn spawn_verb(
    client: &Arc<DaemonClient>,
    tx: &tokio::sync::mpsc::Sender<engine::Outcome>,
    action: engine::Action,
) {
    let tx = tx.clone();
    let client = Arc::clone(client);
    tokio::spawn(async move {
        let _ = tx.send(engine::run(&client, action).await).await;
    });
}

/// Spawn one background probe: machine state, plus the opened
/// project's slower facts when S2 is up (they ride the same cadence,
/// so the panel's counts and path set stay current). The task answers
/// on `tx` whatever happens, so the in-flight guard always clears.
fn spawn_probe(
    client: &Arc<DaemonClient>,
    tx: &tokio::sync::mpsc::Sender<(engine::EngineState, Option<detail::Facts>)>,
    open: Option<String>,
) {
    let tx = tx.clone();
    let client = Arc::clone(client);
    tokio::spawn(async move {
        let state = engine::probe(&client).await;
        let facts = match open {
            Some(id) => engine::detail(&client, &id).await,
            None => None,
        };
        let _ = tx.send((state, facts)).await;
    });
}

/// Apply one probe answer to the app. Returns whether the frame
/// changed. While the background handshake is still bringing the
/// engine up, an unanswered probe is expected — the title keeps
/// "starting…" instead of raising an alarm the machine is about to
/// answer.
fn absorb_probe_answer(
    app: &mut App,
    state: engine::EngineState,
    facts: Option<detail::Facts>,
    spawn_done: bool,
) -> bool {
    let starting_grace = !spawn_done
        && matches!(state, engine::EngineState::Unreachable)
        && matches!(app.engine, engine::EngineState::Starting);
    let mut changed = !starting_grace && app.absorb(state);
    if let Some(facts) = facts {
        changed |= app.absorb_detail(facts);
    }
    changed
}

/// Spawn one background progress poll. An unanswered poll sends no
/// targets: the meters simply hold until the next answer.
fn spawn_progress_poll(
    client: &Arc<DaemonClient>,
    tx: &tokio::sync::mpsc::Sender<Vec<engine::ProgressTarget>>,
) {
    let tx = tx.clone();
    let client = Arc::clone(client);
    tokio::spawn(async move {
        let targets = engine::progress(&client).await.unwrap_or_default();
        let _ = tx.send(targets).await;
    });
}

/// Spawn a background lawn fetch for every strip whose freshness
/// signature moved past its cached lawn — at most one in flight per
/// project. The task answers on `tx` whatever happens, so the
/// in-flight set always drains.
fn spawn_lawn_fetches(
    app: &App,
    fetching: &mut std::collections::HashSet<String>,
    tx: &tokio::sync::mpsc::Sender<(String, engine::FreshSig, Option<lawn::Lawn>)>,
    client: &Arc<DaemonClient>,
) {
    for (id, sig) in app.lawn_wants() {
        if fetching.insert(id.clone()) {
            let tx = tx.clone();
            let client = Arc::clone(client);
            tokio::spawn(async move {
                let fetched = engine::lawn(&client, &id).await;
                let _ = tx.send((id, sig, fetched)).await;
            });
        }
    }
}
