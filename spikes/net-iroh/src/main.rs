//! Spike 1: can two devices find each other and exchange data through iroh
//! with no relay and no internet service, and what does that cost compared
//! with plain TCP?
//!
//! Usage:
//!   spike-net selftest [size_mib]         two processes on this machine
//!   spike-net listen                      run on machine A
//!   spike-net connect <id> [size_mib] [tcp_port] [ip:port]
//!                                         run on machine B with A's printed id
//!
//! The endpoint uses the `Minimal` preset: no relay servers and no public
//! address-lookup service. The only way peers find each other is mDNS on the
//! local network (or an address given by hand).

use std::net::SocketAddr;
use std::process::Stdio;
use std::str::FromStr;
use std::time::{Duration, Instant};

use bytes::Bytes;
use iroh::address_lookup::AddrFilter;
use iroh::endpoint::{Connection, RecvStream, SendStream, presets};
use iroh::{Endpoint, EndpointAddr, EndpointId, TransportAddr};
use iroh_mdns_address_lookup::{DiscoveryEvent, MdnsAddressLookup};
use n0_error::{Result, StdResultExt, anyerr};
use n0_future::StreamExt;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

const ALPN: &[u8] = b"oc/spike-net/1";
/// Our own mDNS service name, so other iroh apps on the network are ignored.
const MDNS_SERVICE: &str = "ocspike";
static FILLER: [u8; 1 << 20] = [0xAB; 1 << 20];

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("listen") => listen().await,
        Some("selftest") => {
            let size_mib = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1024);
            selftest(size_mib).await
        }
        Some("connect") => {
            let id = args.get(2).ok_or_else(|| anyerr!("missing endpoint id"))?;
            let id = EndpointId::from_str(id).anyerr()?;
            let size_mib = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1024);
            let tcp_port = args.get(4).and_then(|s| s.parse().ok());
            let hint = args.get(5).and_then(|s| s.parse().ok());
            let (ep, mdns) = bind().await?;
            measure(&ep, &mdns, id, hint, tcp_port, size_mib).await?;
            ep.close().await;
            Ok(())
        }
        _ => {
            eprintln!(
                "usage: spike-net selftest [size_mib] | listen | connect <id> [size_mib] [tcp_port] [ip:port]"
            );
            Ok(())
        }
    }
}

/// An endpoint that knows about nothing outside the local network.
async fn bind() -> Result<(Endpoint, MdnsAddressLookup)> {
    let ep = Endpoint::builder(presets::Minimal)
        .addr_filter(AddrFilter::ip_only())
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await?;
    let mdns = MdnsAddressLookup::builder()
        .service_name(MDNS_SERVICE)
        .build(ep.id())?;
    ep.address_lookup()?.add(mdns.clone());
    Ok((ep, mdns))
}

// ---------------------------------------------------------------- listener

async fn listen() -> Result<()> {
    let (ep, _mdns) = bind().await?;
    // Listen on IPv6 and IPv4 with one port. On Windows `[::]` is IPv6 only,
    // so IPv4 needs its own socket; on Linux the second bind fails harmlessly.
    let tcp6 = TcpListener::bind("[::]:0").await.anyerr()?;
    let tcp_port = tcp6.local_addr().anyerr()?.port();
    let tcp4 = TcpListener::bind(("0.0.0.0", tcp_port)).await.ok();
    tokio::time::sleep(Duration::from_millis(300)).await; // let local addresses settle
    println!("ID {}", ep.id());
    println!("TCP {tcp_port}");
    for addr in ep.addr().ip_addrs() {
        println!("ADDR {addr}");
    }
    println!("READY");

    tokio::spawn(tcp_server(tcp6));
    if let Some(tcp4) = tcp4 {
        tokio::spawn(tcp_server(tcp4));
    }
    while let Some(incoming) = ep.accept().await {
        tokio::spawn(async move {
            let Ok(conn) = incoming.await else {
                return;
            };
            while let Ok((send, recv)) = conn.accept_bi().await {
                tokio::spawn(async move {
                    let _ = serve_stream(send, recv).await;
                });
            }
        });
    }
    Ok(())
}

/// One request per stream: `D <u64 size>` streams that many bytes back;
/// `P` echoes single bytes until the client finishes.
async fn serve_stream(mut send: SendStream, mut recv: RecvStream) -> Result<()> {
    match recv.read_u8().await.anyerr()? {
        b'D' => {
            let mut left = recv.read_u64().await.anyerr()? as usize;
            while left > 0 {
                let n = left.min(FILLER.len());
                send.write_all(&FILLER[..n]).await.anyerr()?;
                left -= n;
            }
        }
        b'P' => {
            let mut byte = [0u8; 1];
            while recv.read_exact(&mut byte).await.is_ok() {
                send.write_all(&byte).await.anyerr()?;
            }
        }
        _ => {}
    }
    send.finish().anyerr()?;
    Ok(())
}

/// Plain TCP with the same download request, as the baseline.
async fn tcp_server(listener: TcpListener) {
    while let Ok((mut stream, _)) = listener.accept().await {
        tokio::spawn(async move {
            let _ = stream.set_nodelay(true);
            let Ok(size) = stream.read_u64().await else {
                return;
            };
            let mut left = size as usize;
            while left > 0 {
                let n = left.min(FILLER.len());
                if stream.write_all(&FILLER[..n]).await.is_err() {
                    return;
                }
                left -= n;
            }
            let _ = stream.shutdown().await;
        });
    }
}

// ------------------------------------------------------------------ client

/// Start a second process as the listener, then measure against it.
async fn selftest(size_mib: u64) -> Result<()> {
    // Bind and subscribe first so the mDNS timing covers the whole discovery.
    let (ep, mdns) = bind().await?;
    println!("This endpoint:  {}", ep.id().fmt_short());

    let exe = std::env::current_exe().anyerr()?;
    let mut child = tokio::process::Command::new(exe)
        .arg("listen")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .anyerr()?;
    let mut lines = BufReader::new(child.stdout.take().expect("piped stdout")).lines();

    let mut id = None;
    let mut tcp_port = None;
    let mut addrs: Vec<SocketAddr> = Vec::new();
    while let Some(line) = lines.next_line().await.anyerr()? {
        if let Some(v) = line.strip_prefix("ID ") {
            id = Some(EndpointId::from_str(v.trim()).anyerr()?);
        } else if let Some(v) = line.strip_prefix("TCP ") {
            tcp_port = v.trim().parse().ok();
        } else if let Some(v) = line.strip_prefix("ADDR ") {
            addrs.extend(v.trim().parse::<SocketAddr>());
        } else if line == "READY" {
            break;
        }
    }
    let id = id.ok_or_else(|| anyerr!("listener did not print its id"))?;
    println!("Other endpoint: {} (separate process)", id.fmt_short());
    println!(
        "It advertises {} addresses ({} IPv4, {} IPv6)\n",
        addrs.len(),
        addrs.iter().filter(|a| a.is_ipv4()).count(),
        addrs.iter().filter(|a| a.is_ipv6()).count()
    );

    let result = measure(&ep, &mdns, id, None, tcp_port, size_mib).await;
    let _ = child.kill().await;
    ep.close().await;
    result
}

async fn measure(
    ep: &Endpoint,
    mdns: &MdnsAddressLookup,
    id: EndpointId,
    addr_hint: Option<SocketAddr>,
    tcp_port: Option<u16>,
    size_mib: u64,
) -> Result<()> {
    // 1. Discovery by mDNS alone.
    println!("== 1. Discovery (mDNS only, no relay, no internet service) ==");
    let start = Instant::now();
    let mut events = mdns.subscribe().await;
    let found = tokio::time::timeout(Duration::from_secs(15), async {
        while let Some(event) = events.next().await {
            if let DiscoveryEvent::Discovered { endpoint_info, .. } = event
                && endpoint_info.endpoint_id == id
            {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    if found {
        println!("Found by mDNS in {:.0} ms", ms(start.elapsed()));
    } else {
        println!("NOT found by mDNS within 15 s");
    }

    // 2. Connect. With no hint, the id alone must be enough.
    println!("\n== 2. Connect ==");
    let target = match addr_hint {
        Some(addr) => {
            println!("Dialing by id + given address {addr}");
            EndpointAddr::from_parts(id, [TransportAddr::Ip(addr)])
        }
        None => {
            println!("Dialing by id only");
            EndpointAddr::from(id)
        }
    };
    let start = Instant::now();
    let conn = tokio::time::timeout(Duration::from_secs(20), ep.connect(target, ALPN))
        .await
        .map_err(|_| anyerr!("connect timed out after 20 s"))??;
    println!(
        "Connected in {:.1} ms (TLS 1.3 handshake included)",
        ms(start.elapsed())
    );

    let selected = conn
        .paths()
        .iter()
        .find(|p| p.is_selected())
        .map(|p| (p.remote_addr().clone(), p.rtt()));
    let mut remote_ip = None;
    match &selected {
        Some((addr, rtt)) => {
            // Describe the path without printing the address itself.
            let kind = match addr {
                TransportAddr::Ip(sock) if sock.is_ipv4() => "direct, IPv4",
                TransportAddr::Ip(_) => "direct, IPv6",
                _ => "relayed or other",
            };
            println!("Path: {kind}, transport RTT estimate {:.3} ms", ms(*rtt));
            if let TransportAddr::Ip(sock) = addr {
                remote_ip = Some(sock.ip());
            }
        }
        None => println!("Path: none selected yet"),
    }

    // 3. Small-message latency: what a control message costs.
    println!("\n== 3. Round trip of a 1-byte message on an open stream ==");
    let mut samples = quic_ping(&conn, 500).await?;
    samples.sort();
    println!(
        "median {:.3} ms, 99th percentile {:.3} ms ({} samples)",
        ms(samples[samples.len() / 2]),
        ms(samples[samples.len() * 99 / 100]),
        samples.len()
    );

    // 4. Bulk throughput, iroh vs plain TCP on the same route.
    println!("\n== 4. Bulk transfer of {size_mib} MiB ==");
    let size = size_mib << 20;
    let (bytes, took) = quic_download(&conn, size).await?;
    report("iroh (QUIC, encrypted)", bytes, took);
    let quic_rate = bytes as f64 / took.as_secs_f64();

    if let (Some(ip), Some(port)) = (remote_ip, tcp_port) {
        match tcp_download(SocketAddr::new(ip, port), size).await {
            Ok((bytes, took)) => {
                report("plain TCP (unencrypted)", bytes, took);
                let tcp_rate = bytes as f64 / took.as_secs_f64();
                println!(
                    "iroh reaches {:.0}% of plain TCP",
                    quic_rate / tcp_rate * 100.0
                );
            }
            Err(e) => println!("plain TCP baseline failed: {e}"),
        }
    } else {
        println!("plain TCP baseline skipped (no TCP port or no IP path)");
    }

    // 5. Does iroh scale past one stream? This decides how blob transfer
    //    should parallelise on links faster than a single stream can fill.
    println!(
        "\n== 5. Four parallel transfers of {} MiB each ==",
        size_mib / 4
    );
    let quarter = size / 4;
    let start = Instant::now();
    let (a, b, c, d) = tokio::try_join!(
        quic_download(&conn, quarter),
        quic_download(&conn, quarter),
        quic_download(&conn, quarter),
        quic_download(&conn, quarter),
    )?;
    report(
        "4 streams, 1 connection",
        a.0 + b.0 + c.0 + d.0,
        start.elapsed(),
    );

    let dial = || async {
        let target = match addr_hint {
            Some(addr) => EndpointAddr::from_parts(id, [TransportAddr::Ip(addr)]),
            None => EndpointAddr::from(id),
        };
        ep.connect(target, ALPN).await
    };
    let conns = tokio::try_join!(dial(), dial(), dial(), dial())?;
    let start = Instant::now();
    let (a, b, c, d) = tokio::try_join!(
        quic_download(&conns.0, quarter),
        quic_download(&conns.1, quarter),
        quic_download(&conns.2, quarter),
        quic_download(&conns.3, quarter),
    )?;
    report(
        "4 separate connections",
        a.0 + b.0 + c.0 + d.0,
        start.elapsed(),
    );
    for c in [&conns.0, &conns.1, &conns.2, &conns.3] {
        c.close(0u32.into(), b"done");
    }

    conn.close(0u32.into(), b"done");
    Ok(())
}

async fn quic_ping(conn: &Connection, rounds: usize) -> Result<Vec<Duration>> {
    let (mut send, mut recv) = conn.open_bi().await.anyerr()?;
    send.write_u8(b'P').await.anyerr()?;
    let mut samples = Vec::with_capacity(rounds);
    let mut byte = [0u8; 1];
    for _ in 0..rounds {
        let start = Instant::now();
        send.write_all(&[1]).await.anyerr()?;
        recv.read_exact(&mut byte).await.anyerr()?;
        samples.push(start.elapsed());
    }
    send.finish().anyerr()?;
    Ok(samples)
}

async fn quic_download(conn: &Connection, size: u64) -> Result<(u64, Duration)> {
    let (mut send, mut recv) = conn.open_bi().await.anyerr()?;
    let start = Instant::now();
    send.write_u8(b'D').await.anyerr()?;
    send.write_u64(size).await.anyerr()?;
    send.finish().anyerr()?;

    let mut bufs: [Bytes; 32] = std::array::from_fn(|_| Bytes::new());
    let mut total = 0u64;
    while let Some(n) = recv.read_many_chunks(&mut bufs[..]).await.anyerr()? {
        total += bufs.iter().take(n).map(|b| b.len() as u64).sum::<u64>();
    }
    Ok((total, start.elapsed()))
}

async fn tcp_download(addr: SocketAddr, size: u64) -> Result<(u64, Duration)> {
    let mut stream = TcpStream::connect(addr).await.anyerr()?;
    stream.set_nodelay(true).anyerr()?;
    let start = Instant::now();
    stream.write_u64(size).await.anyerr()?;
    let mut buf = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        let n = stream.read(&mut buf).await.anyerr()?;
        if n == 0 {
            break;
        }
        total += n as u64;
    }
    Ok((total, start.elapsed()))
}

fn report(label: &str, bytes: u64, took: Duration) {
    let per_sec = bytes as f64 / took.as_secs_f64();
    println!(
        "{label:<26} {:>6.0} MiB in {:>6.2} s = {:>7.1} MB/s ({:.2} Gbit/s)",
        bytes as f64 / (1u64 << 20) as f64,
        took.as_secs_f64(),
        per_sec / 1e6,
        per_sec * 8.0 / 1e9
    );
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}
