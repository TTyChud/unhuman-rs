use std::fs;
use std::path::PathBuf;

const MAGIC: &str = "nocap-capture-v1";
const DEFAULT_FRAMES: u64 = 16;
const SEED: u64 = 0x9E37_79B9_7F4A_7C15;

fn gen_values(frames: u64) -> Vec<u64> {
    let mut state = SEED;
    let mut out = Vec::with_capacity(frames as usize);
    for _ in 0..frames {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        out.push(state);
    }
    out
}

fn fnv64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
    }
    hash
}

fn write_capture(path: &PathBuf, frames: u64) -> Result<u64, String> {
    let values = gen_values(frames);
    let mut body = String::new();
    body.push_str(MAGIC);
    body.push('\n');
    body.push_str(&format!("frames={frames}\n"));
    body.push_str(&format!("seed={SEED}\n"));
    for (index, value) in values.iter().enumerate() {
        body.push_str(&format!("row {index} {value}\n"));
    }
    let checksum = fnv64(body.as_bytes());
    body.push_str(&format!("checksum={checksum:016x}\n"));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(path, body.as_bytes()).map_err(|e| e.to_string())?;
    Ok(checksum)
}

fn read_capture(path: &PathBuf) -> Result<(u64, u64, Vec<u64>, u64), String> {
    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut frames = None;
    let mut seed = None;
    let mut rows = Vec::new();
    let mut stored_checksum = None;
    let mut body = String::new();
    let mut in_checksum = false;
    for line in raw.split_inclusive('\n') {
        let trimmed = line.trim_end_matches('\n');
        if trimmed.starts_with("checksum=") {
            stored_checksum = Some(
                u64::from_str_radix(trimmed.trim_start_matches("checksum="), 16)
                    .map_err(|_| "bad checksum".to_string())?,
            );
            in_checksum = true;
            continue;
        }
        if !in_checksum {
            body.push_str(line);
        }
        if trimmed == MAGIC {
        } else if let Some(rest) = trimmed.strip_prefix("frames=") {
            frames = Some(rest.parse::<u64>().map_err(|_| "bad frames".to_string())?);
        } else if let Some(rest) = trimmed.strip_prefix("seed=") {
            seed = Some(rest.parse::<u64>().map_err(|_| "bad seed".to_string())?);
        } else if let Some(rest) = trimmed.strip_prefix("row ") {
            let mut parts = rest.split_whitespace();
            let index: usize = parts
                .next()
                .ok_or_else(|| "bad row".to_string())?
                .parse()
                .map_err(|_| "bad row index".to_string())?;
            let value: u64 = parts
                .next()
                .ok_or_else(|| "bad row".to_string())?
                .parse()
                .map_err(|_| "bad row value".to_string())?;
            // index is checked by count below.
            let _ = index;
            rows.push(value);
        }
    }

    let frames = frames.ok_or_else(|| "missing frames".to_string())?;
    let seed = seed.ok_or_else(|| "missing seed".to_string())?;
    let stored = stored_checksum.ok_or_else(|| "missing checksum".to_string())?;
    let computed = fnv64(body.as_bytes());
    if computed != stored {
        return Err(format!(
            "checksum mismatch: calculated {computed:016x}, stored {stored:016x}"
        ));
    }

    // Validate deterministic content.
    let expected_seed = SEED;
    if seed != expected_seed {
        return Err(format!("unexpected seed: {seed} (expected {expected_seed})"));
    }
    if rows.len() as u64 != frames {
        return Err(format!(
            "row count mismatch: {} rows for {} frames",
            rows.len(),
            frames
        ));
    }
    let expected = gen_values(frames);
    for (index, (actual, wanted)) in rows.iter().zip(expected.iter()).enumerate() {
        if actual != wanted {
            return Err(format!("value mismatch at frame {index}"));
        }
    }
    Ok((frames, seed, rows, stored))
}

fn default_path() -> PathBuf {
    std::env::temp_dir().join("unhuman-nocap").join("capture.txt")
}

fn parse_frames(args: &[String]) -> u64 {
    let mut frames = DEFAULT_FRAMES;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--frames" {
            if let Some(value) = args.get(i + 1) {
                frames = value.parse().unwrap_or(DEFAULT_FRAMES).max(1);
            }
        }
        i += 1;
    }
    frames
}

fn usage() {
    println!("nocap: deterministic capture/replay stub");
    println!("usage: nocap [run]");
    println!("       nocap capture <path> [--frames N]");
    println!("       nocap replay <path>");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let frames = parse_frames(&args);

    // Filter out --frames and following value to get positional arguments.
    let mut pos = Vec::new();
    let mut skip = false;
    for arg in &args {
        if skip {
            skip = false;
            continue;
        }
        if arg == "--frames" {
            skip = true;
        } else {
            pos.push(arg.clone());
        }
    }

    let mode = pos.first().cloned().unwrap_or_else(|| "run".to_string());

    match mode.as_str() {
        "run" => {
            let path = default_path();
            let checksum = match write_capture(&path, frames) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("nocap: capture failed: {e}");
                    std::process::exit(1);
                }
            };
            println!("nocap: captured {frames} frames -> {}", path.display());
            match read_capture(&path) {
                Ok((n, _, _, stored)) => {
                    println!("nocap: replay ok (frames={n}, checksum={stored:016x})");
                    let _ = (checksum, n);
                }
                Err(e) => {
                    eprintln!("nocap: replay failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        "capture" => {
            if let Some(path) = pos.get(1) {
                let path = PathBuf::from(path);
                let checksum = match write_capture(&path, frames) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("nocap: capture failed: {e}");
                        std::process::exit(1);
                    }
                };
                println!("nocap: captured {frames} frames -> {} (checksum={checksum:016x})", path.display());
            } else {
                usage();
                std::process::exit(2);
            }
        }
        "replay" | "verify" => {
            if let Some(path) = pos.get(1) {
                let path = PathBuf::from(path);
                match read_capture(&path) {
                    Ok((n, _, _, stored)) => {
                        println!("nocap: replay ok (frames={n}, checksum={stored:016x})");
                    }
                    Err(e) => {
                        eprintln!("nocap: replay failed: {e}");
                        std::process::exit(1);
                    }
                }
            } else {
                usage();
                std::process::exit(2);
            }
        }
        _ => {
            usage();
            std::process::exit(2);
        }
    }
}
