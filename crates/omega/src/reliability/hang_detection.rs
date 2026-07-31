use std::sync::Arc;
use std::thread;
use std::time::Duration;

use client::Client;
use gpui::{AppContext, TasksIncluded, profiler};
use parking_lot::Mutex;
use ui::App;

mod logging;
mod task_traces;
mod telemetry;

gpui::actions!(
    dev,
    [
        /// Causes a performance hang to test performance monitoring
        HangAction,
        /// Causes a performance hang to test performance monitoring
        HangBackground,
        /// Causes a performance hang to test performance monitoring
        HangForeground,
    ]
);

pub(crate) fn start(client: Arc<Client>, cx: &mut App) {
    let hang_time = if cfg!(debug_assertions) {
        if cfg!(windows) {
            // yes windows debug builds are horribly slow
            Duration::from_secs(30)
        } else {
            Duration::from_secs(5)
        }
    } else {
        // will be lowered over time or turned into a setting
        Duration::from_millis(100)
    };

    if cfg!(debug_assertions) {
        log::warn!("debug build, only reporting hangs longer then {hang_time:?}");
    }

    start_hang_detection(hang_time, client, cx);

    cx.on_action(move |_: &HangAction, _| {
        log::warn!(
            "Hanging the foreground for {hang_time:?} by blocking in an action. \
            Omega will be unresponsive for that time. This should trigger a report in the log",
        );
        thread::sleep(hang_time + Duration::from_micros(1));
        log::warn!("Hang ended");
    });
    cx.on_action(move |_: &HangBackground, cx| {
        cx.background_spawn(async move {
            log::warn!(
                "Hanging one background executor for {hang_time:?}. \
                This should trigger a report in the log",
            );
            thread::sleep(hang_time + Duration::from_micros(1));
            log::warn!("Hang ended");
        })
        .detach();
    });
    cx.on_action(move |_: &HangForeground, cx| {
        cx.spawn(async move |_| {
            log::warn!(
                "Hanging the foreground executor for {hang_time:?} seconds to test \
                performance monitoring! Omega will be unresponsive for that time. \
                This should trigger a report in the log"
            );
            thread::sleep(hang_time + Duration::from_micros(1));
            log::warn!("Hang ended");
        })
        .detach();
    });
}

/// How many completed task timings each thread keeps for a hang trace.
///
/// `OMEGA-DELTA-0210`. A `TaskTiming` is four words, so this is a few hundred
/// KiB per thread and a few MiB across a running Omega — paid for the life of
/// the process, on every thread, so that a hang trace has something in it. The
/// alternative gpui offers is 16 MiB per thread, which is sized for a live
/// trace viewer somebody is reading, not for a buffer that exists to be dumped
/// after the fact.
///
/// A power of two: `VecDeque` grows by doubling, and an off-by-one capacity
/// wastes half a buffer on every thread.
const HANG_TRACE_TIMINGS_PER_THREAD: usize = 8192;

fn start_hang_detection(report_longer_then: Duration, client: Arc<Client>, cx: &App) {
    // `OMEGA-DELTA-0210`. Turn task-timing tracing on, because otherwise the
    // trace this detector writes is empty.
    //
    // gpui gathers cheap statistics unconditionally — which is what the log
    // report below is built from — but only pushes a timing into the per-thread
    // ring buffer when tracing is enabled, and it ships disabled. Upstream, the
    // one caller that enabled it was the miniprofiler UI. `OMEGA-DELTA-0186`
    // deleted that crate with the rest of the legacy editor surface, which left
    // `set_trace_enabled` with no callers at all, and from then on every
    // `hang-*.miniprof.json` Omega wrote contained `"timings": []` for every
    // thread. The file was still written, still cleaned up, still named after
    // the hang — and could never explain one.
    //
    // So the hang detector owns the trace now. That is the honest ownership:
    // it is the only thing left in Omega that reads the buffer, and a buffer
    // whose only reader does not enable it is a buffer that is never read.
    profiler::set_trace_enabled_with_capacity(true, HANG_TRACE_TIMINGS_PER_THREAD);

    let foreground_thread = thread::current().id();
    let monitor_interval = Duration::from_secs(1);
    let background_report_longer_then = report_longer_then.max(Duration::from_secs(1));
    let telemetry = Arc::new(Mutex::new(telemetry::Reporter::new(foreground_thread)));
    let mut log = logging::Reporter::new(
        monitor_interval,
        report_longer_then,
        background_report_longer_then,
        foreground_thread,
    );

    let telemetry2 = Arc::clone(&telemetry);
    cx.on_app_quit({
        move |_| {
            telemetry2.lock().send();
            client.telemetry().flush_events()
        }
    })
    .detach();

    // an OS thread to insulate detection and reporting from hangs on the fore
    // or background.
    thread::Builder::new()
        .name("HangDetection".to_string())
        .spawn(move || {
            // allow "bad" tasks during startup. Not because we should but since here
            // they are not observed by the user and to lower on clutter from the reporter
            thread::sleep(Duration::from_millis(200));
            loop {
                thread::sleep(monitor_interval);
                // TODO(yara) the telemetry should not include still running tasks while the
                // reports being logged should.
                let task_stats = profiler::take_all_stats(TasksIncluded::CompletedAndRunning);
                let action_stats = profiler::take_action_stats();

                {
                    let mut telemetry = telemetry.lock();
                    telemetry.update(&task_stats, &action_stats);
                    telemetry.send_periodically();
                }

                let should_write_trace = log.check_and_report(&task_stats, &action_stats);
                if should_write_trace {
                    if let Some(path) = task_traces::save_any(foreground_thread) {
                        log::info!("Task trace has been saved to: {}", path.display());
                    }
                }
            }
        })
        .expect("App can always spawn threads");
}
