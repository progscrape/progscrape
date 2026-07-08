//! Heartbeat watchdog: force-restart the process if the tokio runtime wedges.
//!
//! A tokio task refreshes a heartbeat every 30s. A native (non-tokio) thread --
//! which a runtime deadlock can't starve -- checks it, and if it goes stale for
//! 5 minutes the runtime is wedged: we grab best-effort diagnostics and exit so
//! the container restarts.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// How long the on-wedge `Handle::dump()` may run before we give up and exit.
/// (Only used where the taskdump build applies.)
#[allow(dead_code)]
const DUMP_BUDGET: Duration = Duration::from_secs(30);

/// Monotonic base; all heartbeat timestamps are millis since this.
fn base() -> Instant {
    static BASE: OnceLock<Instant> = OnceLock::new();
    *BASE.get_or_init(Instant::now)
}

fn now_millis() -> u64 {
    base().elapsed().as_millis() as u64
}

/// Last heartbeat (millis since `base`). Written by the tokio task, read by the
/// native watchdog thread.
static HEARTBEAT: AtomicU64 = AtomicU64::new(0);
/// Runtime handle captured at startup, so the watchdog thread can request a dump.
static RUNTIME: OnceLock<tokio::runtime::Handle> = OnceLock::new();

/// Start the watchdog. Call once, from within the tokio runtime.
pub fn start() {
    base();
    HEARTBEAT.store(now_millis(), Ordering::Relaxed);
    let _ = RUNTIME.set(tokio::runtime::Handle::current());

    // Let a spawned debugger (lldb/gdb) ptrace us for the crash backtrace even
    // without CAP_SYS_PTRACE bypassing yama.
    #[cfg(target_os = "linux")]
    unsafe {
        libc::prctl(libc::PR_SET_PTRACER, -1i64, 0i64, 0i64, 0i64);
    }

    // Tokio side: proves the runtime is still scheduling tasks.
    tokio::spawn(async {
        loop {
            tokio::time::sleep(HEARTBEAT_INTERVAL).await;
            HEARTBEAT.store(now_millis(), Ordering::Relaxed);
        }
    });

    // Native side: an OS thread, unaffected by any tokio deadlock.
    std::thread::Builder::new()
        .name("heartbeat-watchdog".into())
        .spawn(watchdog_loop)
        .expect("failed to spawn watchdog thread");

    tracing::info!(
        "Heartbeat watchdog started (interval {}s, timeout {}s)",
        HEARTBEAT_INTERVAL.as_secs(),
        HEARTBEAT_TIMEOUT.as_secs()
    );
}

fn watchdog_loop() {
    loop {
        std::thread::sleep(HEARTBEAT_INTERVAL);
        let age = now_millis().saturating_sub(HEARTBEAT.load(Ordering::Relaxed));
        if age >= HEARTBEAT_TIMEOUT.as_millis() as u64 {
            on_wedged(age);
        }
    }
}

fn on_wedged(age_millis: u64) -> ! {
    tracing::error!(
        "WATCHDOG: runtime heartbeat stale for {}s (limit {}s) -- runtime wedged; \
         capturing diagnostics and exiting so the container restarts",
        age_millis / 1000,
        HEARTBEAT_TIMEOUT.as_secs(),
    );

    // Native backtraces first -- they work even when the runtime is deadlocked.
    native_backtraces();
    // Best-effort tokio task dump; likely hangs on a real wedge, hence bounded.
    task_dump();

    tracing::error!("WATCHDOG: diagnostics done -- exiting process now");
    std::process::exit(70);
}

/// Shell out to lldb/gdb to capture native per-thread backtraces. Requires the
/// debugger installed in the image and ptrace permission (CAP_SYS_PTRACE).
/// Output goes to a file (not a pipe) to avoid the tracer stalling on a full
/// pipe while it has our threads stopped.
fn native_backtraces() {
    let pid = std::process::id().to_string();
    let path = format!("/tmp/watchdog-backtrace-{pid}.txt");
    // Prefer the Rust-aware wrappers (rust-lldb/rust-gdb ship with the toolchain);
    // fall back to plain lldb/gdb. lldb and gdb take different flags.
    let lldb = ["-p", &pid, "--batch", "-o", "thread backtrace all", "-o", "detach", "-o", "quit"];
    let gdb = ["-p", &pid, "-batch", "-ex", "thread apply all bt"];
    let ran = run_debugger("rust-lldb", &lldb, &path)
        || run_debugger("rust-gdb", &gdb, &path)
        || run_debugger("lldb", &lldb, &path)
        || run_debugger("gdb", &gdb, &path);
    if !ran {
        tracing::error!("WATCHDOG: no debugger (rust-lldb/rust-gdb/lldb/gdb) available");
        return;
    }
    match std::fs::read_to_string(&path) {
        Ok(bt) if !bt.trim().is_empty() => tracing::error!("WATCHDOG native backtraces:\n{bt}"),
        _ => tracing::error!("WATCHDOG: debugger produced no backtrace output"),
    }
}

fn run_debugger(tool: &str, args: &[&str], out_path: &str) -> bool {
    let Ok(file) = std::fs::File::create(out_path) else {
        return false;
    };
    let Ok(stderr) = file.try_clone() else {
        return false;
    };
    match std::process::Command::new(tool)
        .args(args)
        .stdout(file)
        .stderr(stderr)
        .status()
    {
        Ok(status) => {
            tracing::error!("WATCHDOG {tool} exited: {status}");
            true
        }
        Err(e) => {
            tracing::warn!("WATCHDOG {tool} unavailable: {e}");
            false
        }
    }
}

/// Best-effort `Handle::dump()` on a throwaway thread, bounded by `DUMP_BUDGET`.
/// On a true wedge the dump's own barrier deadlocks, so we abandon it after the
/// budget and let the process exit -- the leaked thread dies with us.
#[cfg(all(
    tokio_unstable,
    target_os = "linux",
    any(target_arch = "aarch64", target_arch = "x86", target_arch = "x86_64"),
))]
fn task_dump() {
    let Some(handle) = RUNTIME.get().cloned() else {
        return;
    };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let dumper = handle.clone();
        let dump = handle.block_on(async move { dumper.dump().await });
        let mut out = String::new();
        for task in dump.tasks().iter() {
            out.push_str(&format!("--- task {} ---\n{}\n\n", task.id(), task.trace()));
        }
        let _ = tx.send(out);
    });
    match rx.recv_timeout(DUMP_BUDGET) {
        Ok(dump) => tracing::error!("WATCHDOG tokio task dump:\n{dump}"),
        Err(_) => tracing::error!(
            "WATCHDOG: tokio task dump did not finish within {}s (runtime wedged)",
            DUMP_BUDGET.as_secs()
        ),
    }
}

#[cfg(not(all(
    tokio_unstable,
    target_os = "linux",
    any(target_arch = "aarch64", target_arch = "x86", target_arch = "x86_64"),
)))]
fn task_dump() {
    tracing::error!("WATCHDOG: tokio task dump not available in this build");
}
