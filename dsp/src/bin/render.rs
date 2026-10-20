//! Offline renderer: processes a raw interleaved stereo f32 file, with optional parameter
//! automation.
//!
//! usage: render <sample rate> <block size> <in.f32> <out.f32> [p0 ... p11] [--events <file>]
//!
//! `--events <file>` may appear anywhere; it is removed before the positional arguments are
//! parsed. The file holds one `<frame> <param index> <value>` per line (blank lines and `#`
//! comments are ignored), sorted by frame. Events at frame F are applied right before frame F is
//! processed, splitting the block there; events at frame 0 are applied after `reset`, so they
//! behave like any other mid-stream change. Events at or past the end of the input are ignored.

use dune_crusher_dsp::{DuneCrusher, ParamId};

struct Event {
    frame: usize,
    param: ParamId,
    value: f32,
}

fn fail(msg: String) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

fn load_events(path: &str) -> Vec<Event> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| fail(format!("{path}: {e}")));
    let mut events: Vec<Event> = Vec::new();
    for (lineno, line) in (1..).zip(text.lines()) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bad = || {
            fail(format!(
                "{path}:{lineno}: expected `<frame> <param index> <value>`"
            ))
        };
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [frame, index, value] = fields[..] else {
            bad()
        };
        let frame: usize = frame.parse().unwrap_or_else(|_| bad());
        let index: usize = index.parse().unwrap_or_else(|_| bad());
        let value: f32 = value.parse().unwrap_or_else(|_| bad());
        let Some(&param) = ParamId::ALL.get(index) else {
            fail(format!(
                "{path}:{lineno}: param index {index} out of range 0..{}",
                ParamId::ALL.len() - 1
            ))
        };
        if let Some(prev) = events.last().filter(|prev| frame < prev.frame) {
            fail(format!(
                "{path}:{lineno}: events not sorted by frame ({frame} after {})",
                prev.frame
            ));
        }
        events.push(Event {
            frame,
            param,
            value,
        });
    }
    events
}

fn main() {
    let mut argv = std::env::args();
    let mut args: Vec<String> = argv.next().into_iter().collect();
    let mut events = Vec::new();
    while let Some(arg) = argv.next() {
        if arg == "--events" {
            let path = argv
                .next()
                .unwrap_or_else(|| fail("--events needs a file".into()));
            events = load_events(&path);
        } else {
            args.push(arg);
        }
    }
    if args.len() < 5 {
        eprintln!(
            "usage: {} sr block in.f32 out.f32 [params...] [--events file]",
            args[0]
        );
        std::process::exit(1);
    }
    let sr: f32 = args[1].parse().expect("sample rate");
    let block: usize = args[2].parse().expect("block size");
    let raw = std::fs::read(&args[3]).expect("read input");
    let samples: Vec<f32> = raw
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    let frames = samples.len() / 2;

    let mut fx = DuneCrusher::new();
    fx.set_sample_rate(sr);
    for (i, v) in args[5..].iter().enumerate().take(ParamId::ALL.len()) {
        fx.set_param(ParamId::ALL[i], v.parse().expect("param value"));
    }
    fx.reset();

    let (mut il, mut ir) = (vec![0.0; block], vec![0.0; block]);
    let (mut ol, mut or) = (vec![0.0; block], vec![0.0; block]);
    let mut out = Vec::with_capacity(raw.len());
    let mut pending = events.iter().peekable();
    let mut start = 0;
    while start < frames {
        while let Some(ev) = pending.next_if(|ev| ev.frame == start) {
            fx.set_param(ev.param, ev.value);
        }
        let mut end = frames.min(start + block);
        if let Some(ev) = pending.peek() {
            end = end.min(ev.frame);
        }
        let n = end - start;
        for i in 0..n {
            il[i] = samples[2 * (start + i)];
            ir[i] = samples[2 * (start + i) + 1];
        }
        fx.process(&il[..n], &ir[..n], &mut ol[..n], &mut or[..n]);
        for i in 0..n {
            out.extend_from_slice(&ol[i].to_le_bytes());
            out.extend_from_slice(&or[i].to_le_bytes());
        }
        start = end;
    }
    std::fs::write(&args[4], out).expect("write output");
}
