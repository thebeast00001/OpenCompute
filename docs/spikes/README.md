# Milestone 0 — spike results

*10 November 2026. Run on the development laptop: Ryzen 7 7435HS (8 cores, 16 threads), 24 GB RAM, RTX 4060 Laptop GPU (8 GB), Windows 11, Wi-Fi 6. The code is in [`spikes/`](../../spikes).*

A spike is a small throwaway program that answers one risky question before product code depends on the answer.

## Summary

| Spike | Question | Result |
| --- | --- | --- |
| 4. Inventory | Can we read accurate hardware data, live metrics and "is the owner using the PC" signals without admin rights? | **Yes.** |
| 3. Process limits | Can we cap a task's CPU and memory and reliably kill everything it started? | **Yes on Windows.** The Linux half has not been run. |
| 1. Network | Can two devices find each other and exchange data through iroh with no relay and no internet service? | **Yes between two processes on one machine.** The run between two machines over Wi-Fi is outstanding. |
| 2. Split inference | Is splitting a model across machines faster than spilling it onto the CPU? | **Not started.** Needs a model download and a second machine with a GPU. |
| Toolchain | Does the workspace build on this laptop? | **Yes, after a change:** the laptop had no MSVC linker. |

## Gate A

| Question | Status |
| --- | --- |
| Does iroh connect directly with mDNS and no outside service? | Provisionally yes. Proven on one machine; needs confirming across two machines on the home Wi-Fi. |
| Is split inference at least 1.5× faster than CPU offload when a model does not fit one GPU? | Open. Cannot be measured with one GPU. |

Milestone 1 does not depend on the second answer, so it can start once the two-machine network run passes.

## What the laptop itself revealed

1. **No MSVC linker.** Rust was installed without Visual Studio Build Tools, so nothing could link. The laptop does have MinGW-w64 (GCC 13.2), so the project folder now uses Rust's GNU toolchain. All crates used so far build with it, including iroh. The Tauri desktop app in Milestone 4 needs Microsoft C++ Build Tools; nothing before that does. Release builds come from CI, which has MSVC.
2. **44 GB of free disk.** Enough for now. Models and build output will eat it quickly.
3. **No Linux on the laptop.** WSL has no distribution installed and Docker is not running, so Linux testing here needs one of them set up. CI builds and tests on Linux and macOS in the meantime.
4. **There is a gigabit Ethernet port,** currently unplugged. A cable to the second machine or the router would remove Wi-Fi from the picture.
5. **The home Wi-Fi is set to the "Public" profile in Windows.** Firewall rules must cover that profile, and many home networks are set this way.

## Spike 4 — inventory

| Reading | Source | Result |
| --- | --- | --- |
| CPU, cores, RAM, disk | `sysinfo` | Correct: Ryzen 7 7435HS, 8 physical and 16 logical cores, 23.7 GiB RAM, one SSD |
| GPU VRAM | NVML (`nvml-wrapper`) | **8.00 GiB total**, with live usage, load, temperature and power draw |
| GPU VRAM, any vendor | DXGI | 7.77 GiB dedicated. Correct; the difference is memory the driver reserves. |
| GPU VRAM | WMI `AdapterRAM` | 4 GiB. **Wrong** — the field is 32-bit. |
| GPU | `sysinfo`'s new GPU module | Total is right; live usage is always empty. Not usable for monitoring. |
| CPU temperature | `sysinfo` | Zero sensors without admin rights |
| Network link rate | Windows IP Helper | Wi-Fi 1201 Mbps up; Ethernet present, down |
| Seconds since last input | `GetLastInputInfo` | Works |
| Mains or battery | `GetSystemPowerStatus` | Works |
| Full-screen app or game running | `SHQueryUserNotificationState` | Works |
| Cost of one CPU + memory + GPU sample | measured | 0.11 ms, which is 0.011% of one core at one sample per second |

**Changes to the plan:** GPUs are read through NVML and DXGI, never WMI and not `sysinfo`. Temperature rules use GPU temperature on Windows, because CPU temperature is not available to an ordinary process.

## Spike 3 — process limits (Windows Job Objects)

A worker process ran 16 busy threads inside a job while the parent changed the job's CPU cap.

| Setting | Measured share of the whole machine |
| --- | --- |
| No cap | 94.6% |
| Cap at 25% | 25.04% |
| Cap at the minimum (0.01%) | 0.55% |
| Cap lifted | 94.2% |

Other results:

- **Memory cap of 256 MiB:** the task's allocation was refused at 224 MiB and the job's peak was 257 MiB. The task was not killed; it saw an allocation failure.
- **Kill on close:** a task and a child it had started were both gone within one 5 ms poll of the job handle closing.
- **Overhead:** creating the job, starting the process and assigning it took under 10 ms.

**Changes to the plan:**

- Yielding to the owner is done by lowering the CPU cap on the running job. The minimum cap is a near-pause, not a full stop.
- Rate control cannot be switched off on a job that never had it switched on; Windows rejects the call. The runtime tracks whether a cap is set.
- A memory cap surfaces as a failed allocation, which most programs turn into a crash. The runtime listens for the job's memory-limit notification so it can report "memory limit exceeded" instead of a mystery exit code.
- This spike assigned the process to the job just after starting it. The product creates the process inside the job from the start, so nothing can escape in between.

**Not done:** the Linux equivalent (cgroups v2).

## Spike 1 — network (iroh)

Two separate processes on the laptop, each with its own iroh endpoint. The endpoints used iroh's `Minimal` preset: no relay servers and no public address-lookup service. mDNS was the only way to find each other.

| Measurement | Run 1 | Run 2 |
| --- | --- | --- |
| Found by mDNS | 357 ms | 391 ms |
| Connected by endpoint ID alone, TLS 1.3 handshake included | 2.3 ms | 4.2 ms |
| Round trip of a 1-byte message, median | 0.10 ms | 0.12 ms |
| Round trip of a 1-byte message, 99th percentile | 0.23 ms | 0.31 ms |
| 1 GiB over one iroh stream | 151 MB/s | 123 MB/s |
| 1 GiB over plain TCP, same route | not measured | 1,835 MB/s |
| 1 GiB over 4 streams on one connection | not measured | 91 MB/s |
| 1 GiB over 4 separate connections | not measured | 239 MB/s |

What this means:

- **Discovery and connection work with nothing outside the machine.** A device was dialled by its ID alone and iroh chose a direct path.
- **Control messages are cheap:** about a tenth of a millisecond per round trip.
- **One iroh connection carries about 1 Gbit/s on this CPU.** That is the cost of encrypting in user space; plain TCP on the same route was about 13 times faster. Gigabit Ethernet carries about 118 MB/s and this laptop's Wi-Fi less, so one connection is enough for the links this laptop has.
- **More streams on one connection do not help; more connections do.** Four connections reached 239 MB/s.

These numbers measure software cost only. Both ends shared one CPU and no real network was involved.

**Changes to the plan:**

- The default endpoint is LAN-only (`Minimal` preset). iroh's `N0` preset publishes a device's addresses to a public discovery service, so it is used only when the user turns on remote access.
- Large transfers open several connections when the link is faster than one connection can fill.
- Windows created inbound "allow" rules for the spike on the Public profile after its firewall prompt was accepted. The product explains the prompt during onboarding and `ocx doctor` checks the rule against the active profile.

**Outstanding:** the same test between two machines on the home Wi-Fi. On machine A run `spike-net listen`; on machine B run `spike-net connect <id> 1024 <tcp port>` using the ID and TCP port A prints.

## Spike 2 — split inference

Not started. It needs:

- a llama.cpp release with CUDA support and one model file (several gigabytes of downloads), and
- a second machine with a GPU for the measurement Gate A asks for.

On this laptop alone it can still answer two smaller questions: whether llama.cpp's RPC mode runs on Windows with CUDA at all, and how much the RPC protocol costs with no network in the way.

## Running the spikes again

```bash
cd spikes
cargo build --release
```

```bash
./target/release/spike-inventory
```

```bash
./target/release/spike-joblimits
```

```bash
./target/release/spike-net selftest 1024
```
