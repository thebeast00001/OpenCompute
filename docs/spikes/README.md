# Milestone 0 — spike results

*10 November 2026. Run on the development laptop: Ryzen 7 7435HS (8 cores, 16 threads), 24 GB RAM, RTX 4060 Laptop GPU (8 GB), Windows 11, Wi-Fi 6. The code is in [`spikes/`](../../spikes).*

A spike is a small throwaway program that answers one risky question before product code depends on the answer.

## Summary

| Spike | Question | Result |
| --- | --- | --- |
| 4. Inventory | Can we read accurate hardware data, live metrics and "is the owner using the PC" signals without admin rights? | **Yes.** |
| 3. Process limits | Can we cap a task's CPU and memory and reliably kill everything it started? | **Yes on Windows.** The Linux half has not been run. |
| 1. Network | Can two devices find each other and exchange data through iroh with no relay and no internet service? | **Yes between two processes on one machine.** The run between two machines over Wi-Fi is outstanding. |
| 2. Split inference | Is splitting a model across machines faster than spilling it onto the CPU? | **Yes on emulated links, by 2.3× to 3.6×,** provided the remote device runs the first half of the model. Needs confirming on two real machines. |
| Toolchain | Does the workspace build on this laptop? | **Yes, after a change:** the laptop had no MSVC linker. |

## Gate A

| Question | Status |
| --- | --- |
| Does iroh connect directly with mDNS and no outside service? | Provisionally yes. Proven on one machine; needs confirming across two machines on the home Wi-Fi. |
| Is split inference at least 1.5× faster than CPU offload when a model does not fit one GPU? | Provisionally yes. With one GPU standing in for two and the link emulated, split inference was 3.6× faster over gigabit Ethernet, 3.1× over good Wi-Fi and 2.3× over poor Wi-Fi. Needs confirming with two real GPUs and a real network. |

Both answers are provisional for the same reason: only one machine was available. Milestone 1 does not depend on the second answer, so it can start once the two-machine network run passes.

## What the laptop itself revealed

1. **No MSVC linker.** Rust was installed without Visual Studio Build Tools, so nothing could link. The laptop does have MinGW-w64 (GCC 13.2), so the project folder now uses Rust's GNU toolchain. All crates used so far build with it, including iroh. The Tauri desktop app in Milestone 4 needs Microsoft C++ Build Tools; nothing before that does. Release builds come from CI, which has MSVC.
2. **37 GB of free disk** after the Spike 2 downloads. Enough for now. Models and build output will eat it quickly.
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

## Spike 2 — split inference (llama.cpp RPC)

**Setup.** llama.cpp build b12244 with CUDA 12.4, the Qwen2.5-7B-Instruct model in Q4_K_M form (4.36 GiB, 28 layers), and llama.cpp's own `llama-bench` with a 512-token prompt, 128 generated tokens and 3 repetitions. The official Windows build ran on this laptop unmodified, and its RPC server binds to loopback only unless told otherwise.

**How one GPU stood in for two.** The second half of the model ran inside llama.cpp's RPC server on the same laptop. In a layer split the two halves run one after the other, never at once, so sharing one GPU does not make the split look slower than two equal GPUs would. Traffic between the halves went either directly or through [`spike-linkemu`](../../spikes/linkemu), a proxy that adds a fixed delay and a bandwidth cap.

**How "the model does not fit" was emulated.** Half of the layers were placed on the CPU. That is what llama.cpp does with a model too big for the GPU.

**The emulator was checked first.** Against an echo server it produced these round-trip times and throughputs:

| Profile | Target | Measured |
| --- | --- | --- |
| Gigabit Ethernet | 0.3 ms, 940 Mbit/s | 0.39 ms, 889 Mbit/s |
| Good Wi-Fi | 3 ms, 400 Mbit/s | 3.15 ms, 389 Mbit/s |
| Poor Wi-Fi | 8 ms, 80 Mbit/s | 8.33 ms, 78 Mbit/s |

### Generating text

Tokens per second while generating. "Remote last" means the local GPU runs the first half of the layers and the remote device runs the second half and produces the output. "Remote first" is the reverse.

| Configuration | Tokens/s | Compared with one GPU | Compared with half on CPU |
| --- | --- | --- | --- |
| Whole model on one GPU | 53.1 | 100% | 3.9× |
| Half on the GPU, half on the CPU | 13.5 | 26% | 1.0× |
| CPU only | 9.1 | 17% | 0.7× |
| Whole model behind RPC, no delay | 44.8 | 84% | 3.3× |
| Split, remote last, no delay | 45.1 | 85% | 3.3× |
| Split, remote last, gigabit Ethernet | 35.6 | 67% | 2.6× |
| Split, remote last, good Wi-Fi | 26.0 | 49% | 1.9× |
| Split, remote last, poor Wi-Fi | 9.3 | 17% | 0.7× |
| Split, remote first, no delay | 47.2 | 89% | 3.5× |
| **Split, remote first, gigabit Ethernet** | **48.2** | **91%** | **3.6×** |
| **Split, remote first, good Wi-Fi** | **42.5** | **80%** | **3.1×** |
| **Split, remote first, poor Wi-Fi** | **30.7** | **58%** | **2.3×** |

### Processing the prompt

Tokens per second while reading a 512-token prompt.

| Configuration | No delay | Gigabit Ethernet | Good Wi-Fi | Poor Wi-Fi |
| --- | --- | --- | --- | --- |
| Whole model on one GPU | 2,861 | | | |
| Half on the GPU, half on the CPU | 1,286 | | | |
| CPU only | 840 | | | |
| Split, remote last | 1,841 | 1,527 | 1,203 | 436 |
| Split, remote first | 2,039 | 1,246 | 849 | 276 |

The CPU rows are higher than a CPU alone would manage because a CUDA build still hands large batches to the GPU. These figures varied more between runs than the generation figures.

### What this means

- **Splitting beats CPU offload clearly when the order is right.** With the remote device first, split inference kept 58–91% of single-GPU speed across the three links and was 2.3× to 3.6× faster than spilling half the model onto the CPU.
- **The order of devices matters more than the link.** The device that runs the last layers produces the output vector, which has one number per word in the model's vocabulary: about 0.6 MB for this model, for every generated token. With the remote device last, that vector crosses the network each time. With the remote device first, only the much smaller intermediate state crosses (about 14 KB each way). On poor Wi-Fi that is the difference between 9.3 and 30.7 tokens per second.
- **The trade-off is prompt processing.** Remote-first sends the prompt's state across the link and back, about twice the data of remote-last, so it reads prompts more slowly on slow links. For a typical request generation dominates.
- **The RPC protocol itself costs about 15%** with no network delay at all.
- **A poorly ordered split on a slow link is worse than no split.** Remote-last on poor Wi-Fi was slower than CPU offload.

A worked example from these rates, for a 1,000-token prompt and a 300-token answer:

| Configuration | Prompt | Answer | Total |
| --- | --- | --- | --- |
| Whole model on one GPU (if it fits) | 0.3 s | 5.7 s | 6.0 s |
| Half on the CPU | 0.8 s | 22.2 s | 22.9 s |
| Split, remote first, gigabit Ethernet | 0.8 s | 6.2 s | 7.0 s |
| Split, remote first, good Wi-Fi | 1.2 s | 7.1 s | 8.2 s |
| Split, remote first, poor Wi-Fi | 3.6 s | 9.8 s | 13.4 s |
| Split, remote last, poor Wi-Fi | 2.3 s | 32.4 s | 34.7 s |

### Loading a split model the first time

On first load the client uploads the remote half's weights to the RPC server, about 2.2 GiB for this model. Measured on one machine with no link delay:

| RPC server's disk cache | Time to load |
| --- | --- |
| Off | 4.6 s |
| On, empty | 9.6 s |
| On, already filled | 10.2 s |

On one machine the upload takes seconds, and the cache saves nothing because checking it costs as much as sending. Across a real link the upload takes as long as the link needs: about 20 seconds on gigabit Ethernet and about 4 minutes at 80 Mbit/s. Those two figures are calculated from the size, not measured. The cache exists to avoid repeating that.

One run in the first batch took nearly 7 minutes to load the whole model behind RPC. A repeat took 32 seconds. The cause was not identified.

The unedited benchmark output is in [raw/spike2-llama-bench.md](raw/spike2-llama-bench.md).

### Limits of this measurement

- One physical GPU played both devices. Two real GPUs of different speeds will behave differently.
- The emulated links have no jitter and no packet loss. Real Wi-Fi has both, so real Wi-Fi results will be worse than these.
- One model was tested. A model with a larger vocabulary or wider layers moves more data per token.
- The laptop was in normal use during the runs, with other programs holding about two thirds of its RAM.

**Changes to the plan:**

- The split planner places remote devices first and keeps the final layers on the device that receives the request.
- The planner estimates speed from measured link delay and bandwidth, and refuses a split it predicts will be slower than CPU offload.
- Split mode keeps its "experimental" label until the result is repeated on two real machines, but it is no longer expected to be the weak part of the product on wired or good wireless links.
- The first load of a split model over a slow link takes minutes. The dashboard shows that as progress, and the RPC server's cache is kept on so it happens once per model.

**Outstanding:** the same benchmark with two real machines. Machine B runs `ggml-rpc-server -H <its LAN address> -p 50052 -c`; machine A runs `llama-bench` with `-rpc <B's address>:50052 -dev RPC0/CUDA0 -ts 1/1 -ngl 99`. The RPC server has no authentication, so do this only on a home network you trust and stop the server afterwards.

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

Spike 2 needs about 5.4 GB of downloads in a git-ignored `.spike-cache` folder at the repository root:

| Folder | Contents | Source |
| --- | --- | --- |
| `.spike-cache/llama-bin/` | `llama-b12244-bin-win-cuda-12.4-x64.zip`, extracted | llama.cpp GitHub releases |
| `.spike-cache/cudart/` | `cudart-llama-bin-win-cuda-12.4-x64.zip`, extracted | llama.cpp GitHub releases |
| `.spike-cache/models/` | both parts of `qwen2.5-7b-instruct-q4_k_m` | `Qwen/Qwen2.5-7B-Instruct-GGUF` on Hugging Face |

Then, from the repository root in Git Bash:

```bash
bash spikes/split-inference/run.sh .spike-cache 3
```
