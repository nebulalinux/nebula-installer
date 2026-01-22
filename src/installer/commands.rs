use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::model::InstallerEvent;

use super::{send_event, TMP_INSTALLER_LOG};

// Appends a line to the temporary installer log
pub(crate) fn append_temp_installer_log(line: &str) {
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(TMP_INSTALLER_LOG)
    {
        let _ = writeln!(file, "{}", line);
    }
}

// Helper to run a command inside the arch-chroot environment
pub(crate) fn run_chroot(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
    args: &[&str],
    input: Option<&str>,
) -> Result<()> {
    let mut cmd = vec!["/mnt".to_string()];
    cmd.extend(args.iter().map(|s| s.to_string()));
    let args_ref: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();
    run_command(tx, "arch-chroot", &args_ref, input)
}

// Helper to run a streaming command inside the arch-chroot environment
pub(crate) fn run_chroot_stream(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
    args: &[&str],
    input: Option<&str>,
    heartbeat: Option<&str>,
    envs: Option<&[(&str, &str)]>,
) -> Result<()> {
    let mut cmd = vec!["/mnt".to_string()];
    cmd.extend(args.iter().map(|s| s.to_string()));
    let args_ref: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();
    run_command_stream(tx, "arch-chroot", &args_ref, input, heartbeat, envs)
}

// A generic helper to run an external command and stream its output
pub(crate) fn run_command(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
    command: &str,
    args: &[&str],
    input: Option<&str>,
) -> Result<()> {
    let cmdline = if args.is_empty() {
        command.to_string()
    } else {
        format!("{} {}", command, args.join(" "))
    };
    send_event(tx, InstallerEvent::Log(format!("$ {}", cmdline)));

    let mut child = Command::new(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn {}", command))?;

    if let Some(data) = input {
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(data.as_bytes()).context("write stdin")?;
        }
    }

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let tx_out = tx.clone();
    let tx_err = tx.clone();

    let out_handle = stdout.map(|out| {
        thread::spawn(move || {
            let reader = BufReader::new(out);
            for line in reader.lines().flatten() {
                send_event(&tx_out, InstallerEvent::Log(line));
            }
        })
    });

    let err_handle = stderr.map(|err| {
        thread::spawn(move || {
            let reader = BufReader::new(err);
            for line in reader.lines().flatten() {
                send_event(&tx_err, InstallerEvent::Log(line));
            }
        })
    });

    let status = child.wait().context("wait")?;
    if let Some(handle) = out_handle {
        let _ = handle.join();
    }
    if let Some(handle) = err_handle {
        let _ = handle.join();
    }

    if !status.success() {
        anyhow::bail!("Command failed: {}", cmdline);
    }
    Ok(())
}

// A more advanced command runner that streams output line-by-line and provides a heartbeat
pub(crate) fn run_command_stream(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
    command: &str,
    args: &[&str],
    input: Option<&str>,
    heartbeat: Option<&str>,
    envs: Option<&[(&str, &str)]>,
) -> Result<()> {
    let cmdline = if args.is_empty() {
        command.to_string()
    } else {
        format!("{} {}", command, args.join(" "))
    };
    send_event(tx, InstallerEvent::Log(format!("$ {}", cmdline)));

    let mut cmd = Command::new(command);
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(envs) = envs {
        for (key, value) in envs {
            cmd.env(key, value);
        }
    }
    let mut child = cmd.spawn().with_context(|| format!("spawn {}", command))?;

    if let Some(data) = input {
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(data.as_bytes()).context("write stdin")?;
        }
    }

    let running = Arc::new(AtomicBool::new(true));
    if let Some(message) = heartbeat {
        let running = Arc::clone(&running);
        let tx = tx.clone();
        let message = message.to_string();
        thread::spawn(move || {
            send_event(&tx, InstallerEvent::Log(message.clone()));
            while running.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_secs(10));
                if running.load(Ordering::Relaxed) {
                    send_event(&tx, InstallerEvent::Log(message.clone()));
                }
            }
        });
    }

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let tx_out = tx.clone();
    let tx_err = tx.clone();

    let out_handle = stdout.map(|out| thread::spawn(move || stream_command_output(out, &tx_out)));

    let err_handle = stderr.map(|err| thread::spawn(move || stream_command_output(err, &tx_err)));

    let status = child.wait().context("wait")?;
    running.store(false, Ordering::Relaxed);
    if let Some(handle) = out_handle {
        let _ = handle.join();
    }
    if let Some(handle) = err_handle {
        let _ = handle.join();
    }

    if !status.success() {
        anyhow::bail!("Command failed: {}", cmdline);
    }
    Ok(())
}

// Runs a command and captures its stdout
pub(crate) fn run_command_capture(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
    command: &str,
    args: &[&str],
) -> Result<String> {
    let cmdline = if args.is_empty() {
        command.to_string()
    } else {
        format!("{} {}", command, args.join(" "))
    };
    send_event(tx, InstallerEvent::Log(format!("$ {}", cmdline)));

    let output = Command::new(command)
        .args(args)
        .output()
        .with_context(|| format!("run {}", command))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Command failed: {}", stderr.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

// Streams the output of a command, sending each line as a log event
fn stream_command_output<R: std::io::Read>(
    reader: R,
    tx: &crossbeam_channel::Sender<InstallerEvent>,
) {
    let mut buffer = [0u8; 4096];
    let mut line = String::new();
    let mut pending_cr = false;
    let mut reader = reader;
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(_) => break,
        };
        let chunk = String::from_utf8_lossy(&buffer[..count]);
        for ch in chunk.chars() {
            if pending_cr {
                if ch == '\n' {
                    let trimmed = sanitize_log_line(&line);
                    if !trimmed.is_empty() {
                        send_event(tx, InstallerEvent::Log(trimmed));
                    }
                    line.clear();
                    pending_cr = false;
                    continue;
                }
                line.clear();
                pending_cr = false;
            }
            if ch == '\r' {
                pending_cr = true;
                continue;
            }
            if ch == '\n' {
                let trimmed = sanitize_log_line(&line);
                if !trimmed.is_empty() {
                    send_event(tx, InstallerEvent::Log(trimmed));
                }
                line.clear();
            } else {
                line.push(ch);
            }
        }
    }
    if pending_cr {
        let trimmed = sanitize_log_line(&line);
        if !trimmed.is_empty() {
            send_event(tx, InstallerEvent::Log(trimmed));
        }
        return;
    }
    let trimmed = sanitize_log_line(&line);
    if !trimmed.is_empty() {
        send_event(tx, InstallerEvent::Log(trimmed));
    }
}

// Removes ANSI escape codes and other control characters from log lines
fn sanitize_log_line(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            0x1b => {
                i += 1;
                if i >= bytes.len() {
                    break;
                }
                match bytes[i] {
                    b'[' => {
                        i += 1;
                        while i < bytes.len() {
                            let b = bytes[i];
                            if (0x40..=0x7e).contains(&b) {
                                i += 1;
                                break;
                            }
                            i += 1;
                        }
                    }
                    b']' => {
                        i += 1;
                        while i < bytes.len() {
                            if bytes[i] == 0x07 {
                                i += 1;
                                break;
                            }
                            if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                                i += 2;
                                break;
                            }
                            i += 1;
                        }
                    }
                    _ => {
                        i += 1;
                    }
                }
            }
            b if b.is_ascii_control() => {
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    let cleaned = String::from_utf8_lossy(&out);
    cleaned.trim().to_string()
}
