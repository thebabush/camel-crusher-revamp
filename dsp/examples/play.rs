//! Plays an audio file through the Dune Crusher DSP to the default output device, with
//! live parameter control from the terminal.
//!
//! usage: cargo run --release -p dune-crusher-dsp --example play -- <file> [options]
//!   --preset <name>       start from a factory preset (default: "Annihilate")
//!   --set <param>=<v>     set a parameter (0..1), repeatable
//!   --loop                loop the file
//!
//! While playing, type commands followed by Enter:
//!   <param> <value>       e.g. `mech 0.8`, `cutoff 0.4`, `phat 0`
//!   preset <name|number>  load a factory preset
//!   show                  print the current settings
//!   presets               list factory presets
//!   q                     quit

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use dune_crusher_dsp::params::{compress_amount_db, cutoff_hz, volume_gain};
use dune_crusher_dsp::{DuneCrusher, ParamId, FACTORY_PRESETS, NUM_PARAMS};
use std::error::Error;
use std::io::BufRead;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Command-line names, in `ParamId::ALL` order.
const NAMES: [&str; NUM_PARAMS] = [
    "dist_on",
    "mech",
    "tube",
    "filter_on",
    "cutoff",
    "res",
    "comp_on",
    "comp",
    "phat",
    "master_on",
    "mix",
    "volume",
];

/// Largest number of frames handed to the DSP per call from the audio callback.
const SCRATCH: usize = 4096;

fn param_by_name(name: &str) -> Option<ParamId> {
    NAMES
        .iter()
        .position(|n| *n == name)
        .map(|i| ParamId::ALL[i])
}

fn display(id: ParamId, v: f32) -> String {
    let on = if v >= 0.5 { "on" } else { "off" };
    match id {
        ParamId::DistOn
        | ParamId::MmFilterOn
        | ParamId::CompressOn
        | ParamId::CompressMode
        | ParamId::MasterOn => on.into(),
        ParamId::MmFilterCutoff => format!("{:.0} Hz", cutoff_hz(v)),
        ParamId::CompressAmount => format!("{:.1} dB", compress_amount_db(v)),
        ParamId::MasterVolume if volume_gain(v) < 1e-6 => "-inf dB".into(),
        ParamId::MasterVolume => format!("{:.1} dB", 20.0 * volume_gain(v).log10()),
        _ => format!("{:.0} %", v * 100.0),
    }
}

fn find_preset(key: &str) -> Option<usize> {
    if let Ok(n) = key.parse::<usize>() {
        return (1..=FACTORY_PRESETS.len()).contains(&n).then(|| n - 1);
    }
    let key = key.to_lowercase();
    FACTORY_PRESETS
        .iter()
        .position(|p| p.name.to_lowercase() == key)
        .or_else(|| {
            FACTORY_PRESETS
                .iter()
                .position(|p| p.name.to_lowercase().starts_with(&key))
        })
}

/// Decodes the whole file into interleaved stereo f32 (mono is duplicated, extra channels
/// dropped). Returns the samples and the file's sample rate.
fn decode(path: &Path) -> Result<(Vec<f32>, u32), Box<dyn Error>> {
    let mss = MediaSourceStream::new(Box::new(std::fs::File::open(path)?), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;
    let mut format = probed.format;
    let track = format.default_track().ok_or("no audio track")?;
    let track_id = track.id;
    let mut rate = track.codec_params.sample_rate.unwrap_or(44100);
    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;

    let mut out = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymError::DecodeError(_)) => continue,
            Err(e) => return Err(e.into()),
        };
        let spec = *decoded.spec();
        rate = spec.rate;
        let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buf.copy_interleaved_ref(decoded);
        let ch = spec.channels.count();
        for frame in buf.samples().chunks_exact(ch) {
            out.push(frame[0]);
            out.push(if ch > 1 { frame[1] } else { frame[0] });
        }
    }
    Ok((out, rate))
}

/// Linear-interpolation resampler for interleaved stereo. Good enough for a demo player.
fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    let frames = input.len() / 2;
    let out_frames = (frames as u64 * to as u64 / from as u64) as usize;
    let step = from as f64 / to as f64;
    let mut out = Vec::with_capacity(out_frames * 2);
    for i in 0..out_frames {
        let pos = i as f64 * step;
        let j = pos as usize;
        let t = (pos - j as f64) as f32;
        let k = (j + 1).min(frames - 1);
        for c in 0..2 {
            out.push(input[2 * j + c] * (1.0 - t) + input[2 * k + c] * t);
        }
    }
    out
}

enum Cmd {
    Set(ParamId, f32),
    Preset(usize),
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let mut file = None;
    let mut preset = 0;
    let mut sets = Vec::new();
    let mut looping = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--preset" => {
                let name = args.next().ok_or("--preset needs a name")?;
                preset = find_preset(&name).ok_or_else(|| format!("unknown preset '{name}'"))?;
            }
            "--set" => {
                let kv = args.next().ok_or("--set needs param=value")?;
                let (k, v) = kv.split_once('=').ok_or("--set needs param=value")?;
                let id = param_by_name(k).ok_or_else(|| {
                    format!("unknown parameter '{k}'; one of {}", NAMES.join(", "))
                })?;
                sets.push((id, v.parse::<f32>()?.clamp(0.0, 1.0)));
            }
            "--loop" => looping = true,
            "-h" | "--help" => {
                eprintln!("usage: play <file> [--preset NAME] [--set param=value]... [--loop]");
                return Ok(());
            }
            _ => file = Some(a),
        }
    }
    let file = file.ok_or("usage: play <file> [--preset NAME] [--set param=value]... [--loop]")?;

    let (audio, file_rate) = decode(Path::new(&file))?;
    if audio.is_empty() {
        return Err("file decoded to no audio".into());
    }

    // Use the device's current format rather than switching its sample rate (on macOS that
    // would change it for every app), and resample the file instead.
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("no default output device")?;
    let supported = device.default_output_config()?;
    if supported.sample_format() != cpal::SampleFormat::F32 {
        return Err(format!(
            "output device uses {:?} samples; this example only handles f32",
            supported.sample_format()
        )
        .into());
    }
    let config: cpal::StreamConfig = supported.into();
    let rate = config.sample_rate.0;
    let channels = config.channels as usize;
    let audio = if rate == file_rate {
        audio
    } else {
        resample(&audio, file_rate, rate)
    };
    let total_frames = audio.len() / 2;

    let mut values = FACTORY_PRESETS[preset].values;
    for &(id, v) in &sets {
        values[id as usize] = v;
    }
    let mut fx = DuneCrusher::new();
    fx.set_sample_rate(rate as f32);
    for (i, &v) in values.iter().enumerate() {
        fx.set_param(ParamId::ALL[i], v);
    }
    fx.reset();

    println!(
        "Playing {} ({:.1} s, {} Hz{}) on '{}' through preset '{}'",
        file,
        total_frames as f32 / rate as f32,
        file_rate,
        if rate != file_rate {
            format!(" -> {rate} Hz")
        } else {
            String::new()
        },
        device.name().unwrap_or_default(),
        FACTORY_PRESETS[preset].name,
    );
    print_settings(&values);
    println!("Commands: <param> <value> | preset <name|n> | presets | show | q");

    let (tx, rx) = mpsc::channel::<Cmd>();
    let finished = Arc::new(AtomicBool::new(false));
    let finished_cb = finished.clone();
    let mut pos = 0usize;
    let (mut il, mut ir, mut ol, mut or) = (
        vec![0.0; SCRATCH],
        vec![0.0; SCRATCH],
        vec![0.0; SCRATCH],
        vec![0.0; SCRATCH],
    );

    let stream = device.build_output_stream(
        &config,
        move |data: &mut [f32], _| {
            while let Ok(cmd) = rx.try_recv() {
                match cmd {
                    Cmd::Set(id, v) => fx.set_param(id, v),
                    Cmd::Preset(i) => fx.load_preset(&FACTORY_PRESETS[i]),
                }
            }
            for out in data.chunks_mut(SCRATCH * channels) {
                let n = out.len() / channels;
                for i in 0..n {
                    if pos >= total_frames && looping {
                        pos = 0;
                    }
                    let (l, r) = if pos < total_frames {
                        (audio[2 * pos], audio[2 * pos + 1])
                    } else {
                        (0.0, 0.0)
                    };
                    il[i] = l;
                    ir[i] = r;
                    pos += 1;
                }
                fx.process(&il[..n], &ir[..n], &mut ol[..n], &mut or[..n]);
                for (i, frame) in out.chunks_exact_mut(channels).enumerate() {
                    if channels == 1 {
                        frame[0] = 0.5 * (ol[i] + or[i]);
                    } else {
                        frame[0] = ol[i];
                        frame[1] = or[i];
                        frame[2..].fill(0.0);
                    }
                }
            }
            if !looping && pos >= total_frames {
                finished_cb.store(true, Ordering::Relaxed);
            }
        },
        |err| eprintln!("stream error: {err}"),
        None,
    )?;
    stream.play()?;

    // Terminal commands come in on a separate thread so the main thread can stop when
    // playback ends.
    let quit = Arc::new(AtomicBool::new(false));
    let quit_in = quit.clone();
    std::thread::spawn(move || {
        let mut values = values;
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            let words: Vec<&str> = line.split_whitespace().collect();
            match words.as_slice() {
                [] => {}
                ["q" | "quit" | "exit"] => {
                    quit_in.store(true, Ordering::Relaxed);
                    break;
                }
                ["show"] => print_settings(&values),
                ["presets"] => {
                    for (i, p) in FACTORY_PRESETS.iter().enumerate() {
                        println!("  {:2}  {}", i + 1, p.name);
                    }
                }
                ["preset", rest @ ..] if !rest.is_empty() => match find_preset(&rest.join(" ")) {
                    Some(i) => {
                        values = FACTORY_PRESETS[i].values;
                        let _ = tx.send(Cmd::Preset(i));
                        println!("preset '{}'", FACTORY_PRESETS[i].name);
                        print_settings(&values);
                    }
                    None => println!("unknown preset; type `presets` for the list"),
                },
                [name, value] => match (param_by_name(name), value.parse::<f32>()) {
                    (Some(id), Ok(v)) => {
                        let v = v.clamp(0.0, 1.0);
                        values[id as usize] = v;
                        let _ = tx.send(Cmd::Set(id, v));
                        println!("{name} = {v} ({})", display(id, v));
                    }
                    _ => println!("expected `<param> <0..1>`; params: {}", NAMES.join(", ")),
                },
                _ => println!("commands: <param> <value> | preset <name|n> | presets | show | q"),
            }
        }
    });

    while !quit.load(Ordering::Relaxed) && !finished.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(50));
    }
    // Let the device drain its last buffer before tearing the stream down.
    std::thread::sleep(Duration::from_millis(200));
    drop(stream);
    Ok(())
}

fn print_settings(values: &[f32; NUM_PARAMS]) {
    for (i, &v) in values.iter().enumerate() {
        println!(
            "  {:10} {:>5.3}  {}",
            NAMES[i],
            v,
            display(ParamId::ALL[i], v)
        );
    }
}
