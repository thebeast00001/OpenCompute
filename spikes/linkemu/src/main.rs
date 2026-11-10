//! Link emulator: a local TCP proxy that adds a fixed one-way delay and a
//! bandwidth cap, so software can be measured as if its two halves were on
//! different machines.
//!
//!   spike-linkemu <listen_port> <target_host:port> <one_way_delay_ms> <mbit_per_s>
//!
//! A bandwidth of 0 means unlimited. It models delay and capacity only: no
//! jitter and no packet loss, so results are a best case for a given link.
//!
//! Timing uses a sleep followed by a short spin, because ordinary sleeps are
//! too coarse to place deliveries a fraction of a millisecond apart. That
//! costs CPU while data is in flight, which is acceptable for a measuring
//! tool.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const CHUNK: usize = 64 * 1024;
const SPIN_WINDOW: Duration = Duration::from_millis(2);

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 {
        eprintln!(
            "usage: spike-linkemu <listen_port> <target_host:port> <one_way_delay_ms> <mbit_per_s>"
        );
        std::process::exit(2);
    }
    let port: u16 = args[1].parse().expect("listen port");
    let target = args[2].clone();
    let delay = Duration::from_secs_f64(args[3].parse::<f64>().expect("delay in ms") / 1000.0);
    let mbit: f64 = args[4].parse().expect("bandwidth in Mbit/s");
    let bytes_per_sec = if mbit > 0.0 {
        Some(mbit * 1e6 / 8.0)
    } else {
        None
    };

    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind listen port");
    println!(
        "linkemu 127.0.0.1:{port} -> {target}: {:.3} ms each way, {}",
        delay.as_secs_f64() * 1000.0,
        if mbit > 0.0 {
            format!("{mbit:.0} Mbit/s")
        } else {
            "unlimited bandwidth".to_string()
        }
    );

    for client in listener.incoming().flatten() {
        let Ok(server) = TcpStream::connect(&target) else {
            eprintln!("could not reach {target}");
            continue;
        };
        let _ = client.set_nodelay(true);
        let _ = server.set_nodelay(true);
        let (Ok(client_w), Ok(server_w)) = (client.try_clone(), server.try_clone()) else {
            continue;
        };
        pump(client, server_w, delay, bytes_per_sec);
        pump(server, client_w, delay, bytes_per_sec);
    }
}

/// Carry bytes one way across the emulated link.
///
/// The reader holds each chunk for as long as the link would take to
/// transmit it, which also slows its reading and so pushes back on the
/// sender. The writer then releases the chunk after the propagation delay.
fn pump(mut from: TcpStream, mut to: TcpStream, delay: Duration, bytes_per_sec: Option<f64>) {
    let (tx, rx) = mpsc::channel::<(Instant, Vec<u8>)>();

    std::thread::spawn(move || {
        let mut buf = vec![0u8; CHUNK];
        let mut link_free_at = Instant::now();
        loop {
            let n = match from.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let start = Instant::now().max(link_free_at);
            link_free_at = match bytes_per_sec {
                Some(rate) => start + Duration::from_secs_f64(n as f64 / rate),
                None => start,
            };
            wait_until(link_free_at);
            if tx.send((link_free_at + delay, buf[..n].to_vec())).is_err() {
                break;
            }
        }
        // Dropping `tx` lets the writer finish what is in flight, then stop.
    });

    std::thread::spawn(move || {
        for (deliver_at, data) in rx {
            wait_until(deliver_at);
            if to.write_all(&data).is_err() {
                return;
            }
        }
        let _ = to.shutdown(Shutdown::Write);
    });
}

fn wait_until(deadline: Instant) {
    loop {
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        let left = deadline - now;
        if left > SPIN_WINDOW {
            std::thread::sleep(left - SPIN_WINDOW);
        } else {
            std::hint::spin_loop();
        }
    }
}
