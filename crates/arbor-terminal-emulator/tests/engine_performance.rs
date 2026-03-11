#![cfg(feature = "ghostty-vt-experimental")]

use {
    arbor_terminal_emulator::{TerminalEmulator, TerminalEngineKind},
    std::time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy)]
struct BenchmarkResult {
    process: Duration,
    snapshot: Duration,
}

#[test]
#[ignore = "benchmark helper; run with -- --ignored --nocapture"]
fn compare_embedded_terminal_engines() {
    let workload = benchmark_workload();
    let iterations = 30;
    let rows = 40;
    let cols = 120;

    let alacritty = benchmark_engine(
        TerminalEngineKind::Alacritty,
        &workload,
        iterations,
        rows,
        cols,
    );
    let ghostty = benchmark_engine(
        TerminalEngineKind::GhosttyVtExperimental,
        &workload,
        iterations,
        rows,
        cols,
    );

    println!("embedded terminal benchmark ({iterations} iterations)");
    println!(
        "{:<26} {:>14} {:>14} {:>14}",
        "engine", "process_ms", "snapshot_ms", "total_ms"
    );
    print_result("alacritty", alacritty);
    print_result("ghostty-vt-experimental", ghostty);
}

fn benchmark_engine(
    engine: TerminalEngineKind,
    workload: &[Vec<u8>],
    iterations: usize,
    rows: u16,
    cols: u16,
) -> BenchmarkResult {
    let mut process = Duration::ZERO;
    let mut snapshot = Duration::ZERO;

    for _ in 0..iterations {
        let mut emulator = TerminalEmulator::with_engine(engine, rows, cols);

        let process_started = Instant::now();
        for chunk in workload {
            emulator.process(chunk);
        }
        process += process_started.elapsed();

        let snapshot_started = Instant::now();
        let terminal_snapshot = emulator.snapshot();
        let rendered = emulator.render_ansi_snapshot(180);
        snapshot += snapshot_started.elapsed();

        assert!(
            terminal_snapshot.output.contains("status: done"),
            "missing benchmark sentinel in {} output",
            engine.as_str(),
        );
        assert!(
            !terminal_snapshot.styled_lines.is_empty(),
            "missing styled lines for {}",
            engine.as_str(),
        );
        assert!(
            rendered.contains("status: done"),
            "missing rendered sentinel in {} output",
            engine.as_str(),
        );
    }

    BenchmarkResult { process, snapshot }
}

fn print_result(name: &str, result: BenchmarkResult) {
    let process_ms = result.process.as_secs_f64() * 1000.0;
    let snapshot_ms = result.snapshot.as_secs_f64() * 1000.0;
    let total_ms = process_ms + snapshot_ms;
    println!(
        "{:<26} {:>14.2} {:>14.2} {:>14.2}",
        name, process_ms, snapshot_ms, total_ms
    );
}

fn benchmark_workload() -> Vec<Vec<u8>> {
    let mut chunks = Vec::new();

    for frame in 0..200 {
        chunks.push(
            format!(
                "\x1b[38;2;90;180;255mframe {frame:03}\x1b[0m \
                 \x1b[48;2;30;30;30mstatus: running\x1b[0m\r\n"
            )
            .into_bytes(),
        );
    }

    chunks.push(b"\x1b[?1049h\x1b[2J\x1b[H".to_vec());
    for step in 0..120 {
        chunks.push(
            format!(
                "\x1b[{line};1H\x1b[38;5;{color}mstep {step:03} unicode: \u{2603}\u{fe0f}\x1b[0m",
                line = (step % 30) + 1,
                color = 16 + (step % 200),
            )
            .into_bytes(),
        );
    }
    chunks.push(b"\x1b[?1049l".to_vec());

    for row in 0..300 {
        chunks.push(
            format!(
                "log-{row:03} :: \x1b[1mhighlight\x1b[0m :: \x1b[4;38;5;214mwarning\x1b[0m\r\n"
            )
            .into_bytes(),
        );
    }

    chunks.push(b"\x1b]1337;CurrentDir=/tmp\x07".to_vec());
    chunks.push(b"\x1b[?25l".to_vec());
    chunks.push(b"\x1b[?25h".to_vec());
    chunks.push(b"\x1b[?1h".to_vec());
    chunks.push(b"\x1b[?1l".to_vec());
    chunks.push(b"\x1b[38;2;120;255;120mstatus: done\x1b[0m\r\n".to_vec());

    chunks
}
