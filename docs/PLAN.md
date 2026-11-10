# OpenCompute — build plan

*10 November 2026. Read [FEASIBILITY.md](FEASIBILITY.md) first; this plan builds only what that review found practical.*

This document is the specification to build from. Milestone 0 is under way: the repository, workspace skeleton and CI exist, and all four spikes have run on the development laptop. Two of them still need repeating across two real machines. Their results are in [spikes/README.md](spikes/README.md) and are folded into the sections below.

---

## 1. What we are building

**One sentence:** an open-source app that turns the computers you own into a mesh, so any job runs on the machine best suited to it, or on all of them at once.

**Three promises, in order of importance:**

1. **Route** — run a command, a Python function or an AI model on another of your machines as easily as on this one.
2. **Fan out** — split a batch job across every machine and get the results back in one place.
3. **Split** — run an AI model too big for any single machine across several (experimental).

**First-class platform:** Windows with NVIDIA GPUs. Linux second, macOS third. Mixed clusters must work.

**Explicit non-goals:** pooled RAM, a unified GPU memory space, running games or unmodified apps across machines, phones as compute nodes, a marketplace for strangers' hardware, Kubernetes, any cloud dependency.

**Trust model for version 1:** every device in a cluster belongs to one owner. Devices authenticate each other strongly; tasks from a paired device are trusted the way an SSH session is. Running code from strangers is out of scope.

## 2. Principles

1. **Local-first.** Everything works on a LAN with no internet and no account.
2. **Measure, then schedule.** Placement decisions use measured link speeds and machine speeds, never assumptions.
3. **The owner comes first.** A machine in use by a person yields to that person automatically.
4. **No master.** Any device can sleep or vanish without stopping the rest.
5. **Mixed versions must interoperate.** Devices on different releases of the same major version work together.
6. **Honest numbers.** The dashboard shows only measured values. Benchmarks publish losses as well as wins.
7. **Borrow before building.** Use iroh for networking, llama.cpp for inference and uv for Python environments. Write only what does not exist.

## 3. Architecture

Every device runs the same agent. There is no separate server.

```
 ┌───────────── one agent per device (ocxd) ─────────────┐
 │  Local API  (HTTP + WebSocket on 127.0.0.1)           │◄── dashboard, desktop app,
 │                                                       │    CLI, Python SDK
 │  Identity    Inventory    Policy    Scheduler   AI    │
 │  & roster    & metrics    (sharing) (per job)   mesh  │
 │                                                       │
 │  Runtime:  process · Python worker · service · wasm*  │
 │  Object store:  content-addressed blobs on disk       │
 │                                                       │
 │  Network:  iroh  (QUIC, keys as addresses, relays)    │
 └───────────────────────────────────────────────────────┘
              ▲  encrypted streams, one protocol each  ▼
                 the same agent on every other device        * later
```

**Control and data stay separate.** Small control messages (heartbeats, leases, task state) travel on one long-lived stream per peer. Bulk data (file chunks, model weights, tunnelled traffic) travels on its own streams so it can never delay control.

**Each job has an owner.** The owner is the device that submitted the job, or a device the job was handed to with `--detach`. The owner holds the task list, chooses placements, and handles retries. Other devices only grant or refuse leases on their own resources. This replaces the "cluster coordinator" in the original summary: there is nothing to elect and nothing that must stay awake.

**Membership is a signed list.** Every device holds the roster of paired devices. Changes are signed entries that spread between devices automatically.

## 4. Component design

### 4.1 Identity and pairing

- **Device identity:** an Ed25519 key pair created on first run. The public key is the device's permanent address (iroh's `EndpointId`). The secret key lives in the OS keychain (Windows Credential Manager, macOS Keychain, libsecret on Linux) through the `keyring` crate.
- **Cluster identity:** a cluster root key pair created by the first device. Devices with the `admin` role hold a copy, delivered over the encrypted channel during pairing.
- **Roster:** an append-only log of signed operations: `add(device, name, role)`, `set_role`, `revoke(device)`. Each is signed by the root key. Merge rule: union of all valid entries, and a revoke always wins. Devices compare roster versions in every heartbeat and fetch what they lack.
- **Roles:** `admin` (manage devices, submit, accept work), `member` (submit and accept work), `worker` (accept work only), `client` (submit only; for phones and thin laptops).
- **Pairing on the same network:** the new device appears in "Nearby devices". The user clicks it. Both screens show the same 6-digit code and the user confirms on both. The code is derived as `truncate(BLAKE3(id_A ‖ id_B ‖ nonce_A ‖ nonce_B))`, where A first sends a commitment to its nonce, B replies with its nonce, then A reveals. The commitment stops an attacker from searching for a key that produces a matching code.
- **Pairing across the internet or by phone:** an admin device creates an invite (link or QR code) holding its address and a one-time token valid for 10 minutes. The new device connects and presents the token; the admin device shows an approval prompt.
- **Revocation:** a revoke entry spreads to all devices, which drop connections to the revoked device at once. Revoking an admin device also rotates the root key.

### 4.2 Networking

- **Library:** iroh 1.x. It provides QUIC transport, TLS 1.3 keyed to device identities, NAT hole punching and relay fallback.
- **LAN-only by default:** endpoints use iroh's `Minimal` preset, with no relay and no public address-lookup service. iroh's `N0` preset publishes a device's addresses to a public discovery service, so it is used only when the user turns on remote access. Spike 1 confirmed that two endpoints find and dial each other this way.
- **Discovery on the LAN:** mDNS through iroh's `iroh-mdns-address-lookup` crate, under our own service name so other iroh apps on the network are ignored.
- **Parallel connections for bulk data:** Spike 1 measured about 1 Gbit/s on one iroh connection on the development laptop. Extra streams on the same connection added nothing; separate connections scaled. Large transfers open several connections when the link is faster than one can fill.
- **When discovery is blocked** (college and guest Wi-Fi often isolate clients): add a device by pasting its address ticket, or let iroh connect through a relay.
- **Relays:** n0's public relays are documented as for development and testing only. Use them during development. Before public release, run our own `iroh-relay` on a small server and make the relay address configurable so users can host theirs.
- **Ports:** a fixed default UDP port for iroh (7422) so firewall rules are predictable; UDP 5353 for mDNS. The first launch on Windows triggers a firewall prompt; the onboarding screen explains it and `ocx doctor` verifies that the rule covers the network profile in use. Many home networks, including the development laptop's, are set to "Public".
- **Topology:** full mesh. Every device keeps one connection to every other. Designed for 2–16 devices.
- **Liveness:** heartbeat every second on the control stream. A peer is *suspect* after 5 seconds of silence and *down* after 15. A device about to sleep or shut down sends a goodbye so peers react within a second (Windows: `RegisterSuspendResumeNotification`).
- **Link measurement:** at pairing, and every 10 minutes while idle, each pair measures round-trip time and throughput with a 16 MB burst, and records whether the path is direct or relayed. The scheduler reads these numbers. Data-heavy placements over relayed paths are refused.
- **Abstraction:** all networking sits behind a `Transport` trait with two implementations: iroh, and an in-memory simulated network with configurable delay, loss, bandwidth and partitions. The simulator is what makes the scheduler testable on one machine.

### 4.3 Wire protocols

Each protocol is its own QUIC application protocol (ALPN). Messages are Protocol Buffers with a 4-byte length prefix. Schemas are compiled with `protox` so developers and CI do not need `protoc` installed.

| ALPN | Purpose |
| --- | --- |
| `oc/hello/1` | Version, capabilities and roster version exchange on connect |
| `oc/pair/1` | Pairing handshake |
| `oc/ctl/1` | Heartbeats, inventory, roster sync, leases, task control |
| `oc/blob/1` | Chunk requests and responses |
| `oc/io/1` | Live stdout, stderr and log streaming |
| `oc/tunnel/1` | TCP port forwarding for services |

The number in the ALPN is the major version. Minor capabilities are negotiated in `hello`. Unknown fields are ignored, so a newer device can talk to an older one.

### 4.4 Inventory and metrics

- **Static inventory**, collected at start and on hardware change: OS and version, CPU model, cores and threads, RAM, disks (size, free, SSD or HDD), GPUs (vendor, model, VRAM, driver, compute backends available), network adapters and link rates, battery presence.
- **Sources:** the `sysinfo` crate for CPU, RAM and disk; the Windows IP Helper API for adapter link rates. GPUs: NVML through `nvml-wrapper` for NVIDIA; DXGI on Windows for every vendor; sysfs on Linux for AMD and Intel; Metal on macOS. Never use Windows' `Win32_VideoController.AdapterRAM`: on the development laptop it reports 4 GB for an 8 GB card. `sysinfo`'s own GPU module reports total VRAM but no live usage, so it is not used for GPUs.
- **Temperature:** GPU temperature comes from NVML. CPU temperature is not readable by an ordinary process on Windows, so thermal rules there use GPU temperature.
- **Live metrics**, sampled once per second: CPU use per core, RAM used, GPU use, VRAM used, GPU temperature and power draw, disk and network throughput, battery level and whether on mains power.
- **Storage:** the last hour at 1-second resolution in memory; 1-minute roll-ups for 30 days in SQLite.
- **Calibration:** a 10-second benchmark on first run produces a CPU score (single and multi-core) and a GPU score. The scheduler starts from these and refines them from real task durations.
- **Sharing:** each device sends its inventory on connect and a compact metric summary with every heartbeat.

### 4.5 Object store

The practical form of "distributed memory". Data is stored once, addressed by its hash, and referenced from anywhere.

- **Blob:** immutable bytes identified by their BLAKE3 hash.
- **Layout:** one file per blob under `objects/ab/cdef…` in the data directory. Readers memory-map the file, so the OS page cache keeps hot data in RAM without a custom memory manager, and Python gets zero-copy access.
- **Chunking for transfer:** fixed 1 MiB chunks for job objects. Files use content-defined chunking (FastCDC, about 1 MiB average) so an edited file re-sends only the changed parts. A manifest lists chunk hashes; the manifest's hash is the blob's identity.
- **Transfer:** the receiver states which chunks it already has and requests the rest, over several parallel streams. Every chunk is verified on arrival. An interrupted transfer resumes with no verified chunk sent twice.
- **Small objects** (under 64 KiB) travel inline inside task messages.
- **Budget and eviction:** each device has a configurable store limit (default: 20% of free disk). Unpinned blobs are evicted least-recently-used. Blobs referenced by a live job are pinned.
- **Locations:** a job's owner tracks where that job's objects are. Any device answers "do you have this hash?". No global directory.
- **Interface:** a `BlobStore` trait, so the implementation can be replaced (for example by iroh-blobs once it is production quality).

### 4.6 Jobs, tasks and the scheduler

**Vocabulary:** a *task* is one unit of work on one machine. A *job* is a set of tasks, possibly depending on each other's outputs.

**Task specification (protobuf sketch):**

```proto
message TaskSpec {
  bytes task_id = 1;                 // ULID
  bytes job_id  = 2;
  oneof body {
    ProcessTask process = 10;        // argv, env, working dir
    PythonTask  python  = 11;        // environment ref, pickled function ref, argument refs
    ServiceTask service = 12;        // long-running, with ports to expose
    WasmTask    wasm    = 13;        // later
  }
  repeated InputBinding  inputs  = 20;   // blob -> file path or argument
  repeated OutputBinding outputs = 21;   // file path or return value -> blob
  Resources resources = 30;          // cpu_millis, memory_bytes, gpus, vram_bytes, disk_bytes
  map<string, string> selectors = 31;    // os=windows, arch=x86_64, gpu.vendor=nvidia
  RetryPolicy retry = 40;            // max attempts, backoff, which failures to retry
  uint64 timeout_ms = 41;
  int32  priority   = 42;
  bool   idempotent = 43;
}
```

**Task states:**

```
PENDING -> LEASED -> STAGING -> RUNNING -> SUCCEEDED
                                        -> FAILED     (retry if policy allows)
                                        -> CANCELLED
                                        -> LOST       (device vanished; retry)
```

`STAGING` is fetching inputs and preparing the environment.

**Leases.** The owner asks a device for a lease on specific resources. The device grants or refuses based on its real current load and its sharing policy. A lease expires unless renewed by heartbeat. This is how two owners can use the same device without a central arbiter: the device itself is the arbiter.

**Placement.** For each ready task the owner computes, for every device that passes the filters (selectors, free resources, policy):

```
estimated finish = time until the device is free
                 + sum over missing inputs of (size / measured link throughput)
                 + estimated run time / device speed for this kind of task
```

It picks the smallest. Ties go to the device that needs the fewest bytes moved, then the least loaded. Run-time estimates start from a user hint or the median, then learn per function name from history. Device speed starts from the calibration score. This is the rule from section 4 of the feasibility review, applied automatically: work moves only when moving it is faster.

**Guarantees.** Execution is at-least-once. Each attempt has an ID and results are accepted only from the current attempt, so a device that reappears after being declared lost cannot overwrite newer results. Tasks should be idempotent; outputs are content-addressed, so a repeated task produces the same blob.

**Failure handling:**

| Failure | Response |
| --- | --- |
| Worker goes down mid-task | Owner marks the task `LOST` and reschedules it if the retry policy allows. |
| An object's only copy was on a dead device | Owner re-runs the task that produced it (it keeps every task spec, so it knows how). |
| Owner goes down | Workers finish running tasks, keep results pinned for 24 hours, and cancel queued tasks. The owner reconciles when it returns. |
| Network partition | Leases expire on both sides; attempt IDs prevent double-accepting results. |
| Task exceeds its memory or time limit | Killed by the runtime; reported as `FAILED` with the reason. |

**Detached jobs.** `ocx run --detach` hands ownership to a device that will stay up (by default the one doing most of the work), so a laptop can submit a job and close its lid.

**Checkpoints (after the first release of the runtime).** A long task can save a checkpoint blob through the SDK; a retry receives the latest one.

### 4.7 Runtime

| Executor | What it runs | When |
| --- | --- | --- |
| **Process** | Any command line. Inputs are placed as files, outputs collected by path, stdout and stderr streamed live. | Milestone 1 |
| **Python worker** | Functions sent from the SDK, run in a pool of warm worker processes. | Milestone 2 |
| **Service** | A long-running process with ports exposed to other devices through tunnels. AI model servers are services. | Milestone 3 |
| **WebAssembly** | Sandboxed, portable modules through `wasmtime`. | Later |

- **Python environments:** a task names its environment as a Python version plus a lockfile hash. The agent builds it once with `uv` (which also installs the right Python) and caches it by hash. This removes the usual "same packages on every machine" problem. Pin workers to a widely supported Python (3.12 or 3.13); the development laptop's 3.14 is still marked experimental by libraries such as Ray.
- **Resource limits:** Windows Job Objects (CPU rate, memory cap, kill all children when the job closes); Linux cgroups v2 through a systemd user scope; macOS resource limits and background QoS. Spike 3 confirmed all three behaviours on Windows.
- **Windows details from Spike 3:** the task process is created inside its job from the start, so no child can escape. A memory cap makes allocations fail instead of killing the task, so the runtime listens for the job's memory-limit notification and reports "memory limit exceeded". CPU rate control cannot be switched off on a job that never had it on, so the runtime tracks whether a cap is set.
- **Priority:** task processes run at low OS priority by default.
- **GPUs:** a GPU is granted whole to one task by default. Inference services may share a GPU within a VRAM budget.
- **Keep-awake:** while tasks run and policy allows, the agent asks the OS not to sleep (Windows: `SetThreadExecutionState`).

### 4.8 Sharing policy

Per device, set in the dashboard:

- **When to accept work:** always, only when idle, only on mains power, or on a schedule.
- **How much:** maximum CPU share, maximum RAM, whether the GPU is shared.
- **Automatic yielding:**
  - User becomes active (Windows `GetLastInputInfo`; equivalents elsewhere): throttle to the configured "in use" share.
  - A full-screen app or game starts (Windows `SHQueryUserNotificationState`): release the GPU.
  - On battery: accept no work by default.
  - Temperature above the threshold: pause until it drops.
- **How yielding happens,** in order: lower the CPU cap on the running task, pause it (the minimum cap, which Spike 3 measured at about 0.5% of the machine), then cancel and let the owner reschedule elsewhere. The cap can be changed while the task runs.
- **Who may submit:** which roles or specific devices this device accepts work from.

### 4.9 AI mesh

Built on llama.cpp; no custom tensor runtime.

- **Model registry:** GGUF model files stored as blobs. Download once from Hugging Face on any device; other devices fetch over the LAN.
- **Runtime:** the agent downloads the matching prebuilt `llama-server` for each device (CUDA, Vulkan, Metal or CPU) and runs it as a service task.
- **Gateway:** every device exposes an OpenAI-compatible API on `127.0.0.1:7430` (`/v1/chat/completions`, `/v1/embeddings`, `/v1/models`). Requests are routed across the mesh. Any tool that speaks the OpenAI API works unchanged.
- **Three deployment modes:**
  1. **Best fit:** the model runs on the single device that fits it best. Default.
  2. **Replicas:** the same model on several devices, with the gateway balancing requests. Throughput scales with devices.
  3. **Split:** `ggml-rpc-server` on the helper devices and `llama-server --rpc` on the lead device. Experimental.
- **Securing split mode:** llama.cpp's RPC server has no authentication, by its own documentation. Ours binds only to `127.0.0.1`, and traffic between devices goes through `oc/tunnel/1`, which is encrypted and restricted to roster members.
- **Split planner:** reads layer sizes from the GGUF file, estimates context memory, and combines free VRAM, free RAM and measured link speeds to choose which devices take part and the `--tensor-split` values. It estimates tokens per second, drops devices that would slow the result, and refuses a split it predicts will be slower than CPU offload.
- **Device order (from Spike 2):** remote devices run the first layers; the device that receives the request runs the last layers and produces the output. The output vector is as large as the model's vocabulary (about 0.6 MB per token for a 7B Qwen model), so it must not cross the network. On an emulated poor Wi-Fi link this order gave 30.7 tokens per second against 9.3 for the reverse.
- **Speed estimate (from Spike 2):** time per generated token is compute time, plus the link's round-trip time for each hand-over, plus the bytes that cross divided by measured bandwidth. Prompt processing moves more data in the remote-first order, so the planner weighs prompt length against expected answer length on slow links.
- **First load of a split model:** the lead device uploads the remote half's weights through the RPC connection, which takes as long as the link needs (minutes on slow Wi-Fi). The RPC server's disk cache stays on so this happens once per model, and the dashboard shows it as progress. Filling that cache from the device's own copy of the model file is a later optimisation.
- **Batch helpers:** fan-out jobs for embeddings and for transcription with whisper.cpp. These are ordinary tasks, and they are where several GPUs give a real speedup.

### 4.10 Files and storage

- **Drop (Milestone 1):** send a file or folder to a device. It lands in that device's `OpenCompute/Inbox`. Resumable and verified.
- **Mirror (Milestone 5):** one-way copy of a folder to another device, kept up to date. Covers backup.
- **Sync (Milestone 5, last):** two-way folder sync. Design: a per-folder manifest of path, size, modification time and chunk list, with a Merkle root for fast comparison; a version vector per file to detect conflicts; conflicting edits keep both copies; deletions recorded as tombstones; the last N versions kept in a hidden folder; ignore patterns; change detection with the `notify` crate plus periodic full rescans. Known hard parts: case-insensitive file systems, Windows reserved names and path limits, partial writes, clock differences. If this slips, the fallback is documenting how to run Syncthing alongside.

### 4.11 Local API, CLI and SDK

**Local API** — `axum`, bound to `127.0.0.1:7420`, bearer token in a user-only file, and rejection of requests with an unexpected `Host` or `Origin` header (this blocks web pages from reaching it). An OpenAPI description is generated with `utoipa` and the TypeScript client is generated from it.

| Area | Endpoints |
| --- | --- |
| Cluster | `GET /v1/cluster`, `GET /v1/devices`, `GET /v1/devices/{id}/metrics`, `WS /v1/events` |
| Pairing | `POST /v1/pair/start`, `POST /v1/pair/confirm`, `POST /v1/invites`, `POST /v1/devices/{id}/revoke` |
| Jobs | `POST /v1/jobs`, `GET /v1/jobs`, `GET /v1/jobs/{id}`, `POST /v1/jobs/{id}/cancel`, `GET /v1/tasks/{id}/logs` |
| Data | `PUT /v1/blobs`, `GET /v1/blobs/{hash}`, `POST /v1/transfers`, `GET /v1/transfers` |
| Policy | `GET /v1/policy`, `PUT /v1/policy` |
| AI | `GET /v1/ai/models`, `POST /v1/ai/models/pull`, `POST /v1/ai/deployments`, `DELETE /v1/ai/deployments/{id}` |

**CLI** — `ocx` (working name; `oc` belongs to OpenShift):

```
ocx up | status | doctor
ocx devices | pair | invite | revoke <device>
ocx send <path> <device>
ocx run [--on <device>] [--gpu] [--cpus N] [--mem 8G] [--detach] -- <command...>
ocx map --input "<glob>" -- <command with {} placeholder>
ocx jobs | logs <id> | cancel <id>
ocx policy show | set ...
ocx models pull <repo> | run <model> [--split] | list
ocx dev cluster --nodes 3 [--latency 5ms] [--bandwidth 100mbit]
```

**Python SDK** — pure Python, talks to the local agent, no compiled extension:

```python
import opencompute as oc

@oc.task(cpus=2, memory="4GiB")
def embed(batch):
    ...

refs = [embed.submit(b) for b in batches]   # returns immediately
results = oc.get(refs)                      # waits and fetches

big = oc.put(dataframe)                     # stored once, passed by reference
out = oc.map(process, partitions)           # each partition runs where its data is
```

### 4.12 Dashboard and desktop app

- **Dashboard:** a React single-page app built with Vite, using Tailwind, shadcn/ui, TanStack Query, Zustand, and Recharts. It is served by the agent at `http://127.0.0.1:7420` and embedded unchanged in the desktop app.
- **Screens:** Overview; Devices (hardware, live charts, policy); Jobs (list, task timeline, logs); Files (Drop, transfers, later folders); AI (models, deployments, chat playground, tokens per second); Settings (pairing, roles, audit log).
- **Design direction:** as in the summary — large type, generous spacing, rounded cards, dark theme, One UI influence. The overview shows per-device capacity and "largest job that fits right now"; any sum is labelled as a total across devices.
- **One-device state:** hardware monitor and local AI server work alone; the empty cluster view invites pairing.
- **Desktop app:** Tauri 2. Tray icon, start at login, runs the agent as a separate process so work continues when the window closes, signed auto-updates.
- **Mobile (Milestone 6):** an Expo app as a remote control: pair by QR, monitor, send files, chat with models.

### 4.13 Observability

- Structured logs with the `tracing` crate; a trace ID travels with every task across devices.
- Per-task event timeline (leased, staged, started, finished, with timings and bytes moved) stored by the owner.
- `ocx doctor` checks firewall, mDNS visibility, NAT type, clock skew, GPU drivers, disk space and peer reachability.
- Optional OpenTelemetry export, off by default. The product sends no telemetry anywhere unless the user turns it on.

## 5. Security model

| Threat | Defence |
| --- | --- |
| An unknown device on the network tries to join | Nothing is accepted from devices outside the signed roster. Joining requires the pairing code or a one-time invite. |
| Someone intercepts traffic on the LAN | All traffic is TLS 1.3 bound to device keys. The pairing code detects an interceptor at first contact. |
| A paired laptop is lost or stolen | Revoke it from any admin device. Keys are in the OS keychain. Revoking an admin rotates the root key. |
| A web page or local program calls the local API | Loopback only, bearer token, `Host` and `Origin` checks. |
| A paired device is compromised | It can run tasks on devices that accept work from its role — the same exposure as SSH access. Roles and per-device accept lists limit the reach. The audit log records every submission. |
| A task misbehaves | CPU, memory and time limits; killed with all its children. |
| llama.cpp's open RPC port | Bound to loopback; reachable only through the authenticated tunnel. |
| A relay operator observes traffic | Relays forward encrypted packets and cannot read them. |
| A tampered release | Signed installers and signed updates; dependency audit (`cargo-deny`) in CI. |

Out of scope for version 1: defending against malicious code a user chooses to run, and any multi-owner cluster.

## 6. Repository and tooling

### 6.1 Git setup (done)

The project folder has its own git repository, with `origin` set to `https://github.com/thebeast00001/OpenCompute.git` and `main` as the default branch. It was created with `git init` inside the project folder so that commands run there affect only this project.

### 6.2 Layout

```
OpenCompute/
├─ Cargo.toml                 workspace
├─ rust-toolchain.toml        pinned stable toolchain
├─ crates/
│  ├─ oc-core/                IDs, errors, config, resource types, clock trait
│  ├─ oc-proto/               .proto files and generated types (protox)
│  ├─ oc-identity/            device keys, keychain, roster, pairing
│  ├─ oc-net/                 Transport trait; iroh and simulated implementations; link probes
│  ├─ oc-inventory/           hardware detection and live metrics
│  ├─ oc-store/               blob store, chunking, transfer protocol
│  ├─ oc-sched/               placement, leases, job state machine (pure logic)
│  ├─ oc-runtime/             process, Python, service executors; resource limits
│  ├─ oc-policy/              sharing rules; idle, battery, thermal, full-screen detection
│  ├─ oc-ai/                  model registry, llama.cpp management, split planner, gateway
│  ├─ oc-sync/                folder mirror and sync (Milestone 5)
│  ├─ oc-api/                 local HTTP and WebSocket API
│  ├─ oc-agent/               binary: ocxd
│  └─ oc-cli/                 binary: ocx
├─ apps/
│  ├─ dashboard/              React + Vite
│  ├─ desktop/                Tauri 2 shell
│  └─ mobile/                 Expo (Milestone 6)
├─ packages/
│  ├─ api-client/             generated TypeScript client
│  └─ ui/                     design tokens and shared components
├─ sdk/python/                the opencompute Python package
├─ bench/                     reproducible benchmarks and published results
├─ tests/                     simulation scenarios, multi-agent integration, chaos
├─ docs/                      this plan, the feasibility review, spike results
└─ .github/workflows/         CI
```

Mapping from the summary's layout: `core/*` becomes `crates/oc-*`; `coordinator` and `scheduler` merge into `oc-sched`; `protocols/` becomes `oc-proto`; `apps/web` becomes `apps/dashboard`.

### 6.3 Key dependencies

| Need | Choice |
| --- | --- |
| Async runtime | tokio |
| Networking | iroh 1.x, `iroh-mdns-address-lookup` |
| Messages | prost, protox |
| Local API | axum, utoipa |
| Database | rusqlite (bundled SQLite, WAL mode) |
| Hashing and chunking | blake3, fastcdc |
| Hardware | sysinfo, nvml-wrapper, `windows` crate (DXGI, Job Objects, power events) |
| Secrets | keyring |
| CLI | clap |
| Logging and tracing | tracing, optional opentelemetry |
| File watching | notify |
| Sandbox (later) | wasmtime |
| Python environments | uv (external binary managed by the agent) |
| AI runtime | llama.cpp release binaries; whisper.cpp for transcription |

### 6.4 Data stored per device (SQLite)

`devices`, `roster_ops`, `jobs`, `tasks`, `task_events`, `objects`, `object_locations`, `transfers`, `metrics_rollup`, `policy`, `models`, `deployments`, `audit`. Schema migrations are numbered files applied at start.

### 6.5 Conventions

- Licence: Apache-2.0.
- Rust: `cargo fmt`, `clippy` with warnings as errors, no `unsafe` outside the OS-integration modules.
- Windows toolchains: release builds come from CI with MSVC. Local development also works with MinGW-w64 and Rust's GNU toolchain, which is what the development laptop uses. The Tauri desktop app (Milestone 4) needs Microsoft C++ Build Tools installed locally.
- CI on every push: format, lint, unit tests, and a 3-agent integration test, on Windows, Ubuntu and macOS runners.
- Commits small and descriptive; `main` always builds.

## 7. Milestones

Durations are rough estimates for one developer working on this most days with AI assistance. Each milestone ends with tests that must pass before the next begins.

### Milestone 0 — Foundations and spikes (about 1 week)

Purpose: answer the four riskiest questions before writing product code.

- Fix the git setup (6.1). Create the workspace skeleton, licence, README and CI.
- **Spike 1, network:** two minimal iroh programs find each other by mDNS on the home Wi-Fi and stream 1 GB. Compare with `iperf3`. Record the Windows Firewall behaviour. Repeat between Windows and WSL 2.
- **Spike 2, split inference:** run `ggml-rpc-server` and `llama-server --rpc` with CUDA. Measure tokens per second for a model that fits on the GPU; a model too big for it using CPU offload; and the same model split across two machines.
- **Spike 3, execution:** start a child process in a Windows Job Object with CPU and memory caps and confirm that closing the job kills it. Repeat on Linux.
- **Spike 4, inventory:** read CPU, RAM, disk and GPU on this laptop and confirm VRAM reads as 8 GB.

**Done when:** results are written up in `docs/spikes/` and Gate A (section 9) is decided.

### Milestone 1 — Alpha: two computers, one cluster (4–6 weeks)

1. Agent skeleton: config, data directory, SQLite, logging, device identity.
2. Discovery by mDNS, and adding a device by address.
3. Pairing with the 6-digit code; roster; revoke.
4. Inventory and live metrics, exchanged between peers.
5. Heartbeats, sleep goodbyes, link measurement.
6. Process tasks: `ocx run --on <device> -- <command>` with live output, exit code, timeout, cancel and resource limits.
7. Blob store and `ocx send`.
8. Local API and dashboard: Overview, Devices, Tasks, Transfers, pairing flow.
9. `ocx doctor` and `ocx dev cluster`.

**Done when:**
- Two real machines pair in under 60 seconds from a fresh install.
- Inventory matches Task Manager and `nvidia-smi` on both.
- A 10 GB file transfers at 90% or more of the measured link speed, survives a disconnect halfway, and verifies.
- A real command (an ffmpeg transcode) runs on the other machine and its output comes back.
- Killing the remote agent mid-task is reported within 5 seconds.
- An idle agent uses under 1% CPU and under 60 MB of RAM.

### Milestone 2 — Runtime: jobs, objects, scheduling (6–8 weeks)

1. Object store features for jobs: pinning, eviction, inline objects, location tracking.
2. Jobs with task dependencies and `map`.
3. Scheduler: leases, placement, retries, re-running lost work.
4. Python SDK and worker pool with uv-managed environments.
5. Sharing policy with idle, battery, temperature and full-screen detection.
6. Dashboard: job timeline and per-task logs.
7. Simulation tests for scheduling and failures.
8. Benchmarks (section 8.3).

**Done when:**
- `oc.map` uses both machines with no `--on`.
- Unplugging a worker mid-job still produces a complete, correct result.
- A warm Python task round-trips in under 10 ms on a LAN.
- Starting a game on a worker frees its GPU within 5 seconds.
- The benchmark report is published in `bench/results/`.

### Milestone 3 — AI mesh (4–6 weeks)

1. Model registry, download, LAN replication.
2. Managed `llama-server` with health checks and VRAM accounting.
3. OpenAI-compatible gateway with routing and replicas.
4. Tunnels; split deployments; split planner.
5. Batch embedding and transcription jobs.
6. Dashboard AI screen with a chat playground.

**Done when:**
- A request to `localhost:7430` on a machine with no model loaded is answered by another machine's GPU.
- A model too large for either GPU runs split across both.
- A published table compares single machine, CPU offload and split, on wired and Wi-Fi links.

### Milestone 4 — Desktop app and public alpha (3–4 weeks)

Tauri shell, tray, start at login, installers (Windows first), signed auto-update, onboarding, single-device features, documentation site, demo video, first GitHub release. Budget for a Windows code-signing certificate; unsigned installers trigger SmartScreen warnings.

**Done when:** three people who have never seen the project install it and pair two machines unaided in under 5 minutes.

### Milestone 5 — Storage (6–10 weeks)

Drop polish, then one-way mirror, then two-way sync with versions and conflict handling.

**Done when:** a 50,000-file folder stays consistent between a Windows and a Linux machine through a week of random edits, renames, deletions and disconnects in an automated test.

### Milestone 6 — Reach (4–8 weeks)

Self-hosted relay, internet invites, mobile remote-control app, macOS polish.

**Done when:** a laptop on a different network submits a job to a home machine and receives the result, and a phone pairs by QR and chats with a model on the PC.

### Milestone 7 — Research (open-ended, only with real users)

WebAssembly tasks; automatic ownership hand-off; smarter split planning for mixed hardware (prima.cpp's ideas); a lab and fleet edition with central policy; far-memory experiments limited to 10 Gb Ethernet or Thunderbolt links.

**To a public alpha (Milestones 0–4): about 4–6 months.**

## 8. Testing and benchmarks

### 8.1 Test layers

| Layer | What it covers |
| --- | --- |
| Unit and property tests | Chunking, manifests, roster merging, placement cost, state machines |
| Deterministic simulation | The scheduler and failure handling on the simulated network with a seeded random source: thousands of scenarios of delay, loss, partition and device death, each reproducible from its seed |
| Multi-agent on one machine | `ocx dev cluster` starts several real agents with separate data directories, and can cap latency and bandwidth to imitate Wi-Fi or gigabit links |
| Cross-OS on one machine | A Windows agent and a WSL 2 agent on the same laptop (mDNS does not cross WSL's default network, so add by address) |
| Real hardware | Two or more physical machines, wired and Wi-Fi, before every release |
| Chaos | Kill agents, suspend network adapters, sleep machines and skew clocks during jobs |

### 8.2 Developing with one laptop

Most development needs only this laptop: the simulator and the multi-agent mode. Linux testing on the laptop needs a WSL distribution or Docker set up first; neither is ready today, and CI covers Linux and macOS in the meantime. A second physical machine is required to finish Milestone 1 (real pairing, real links, real firewall behaviour) and is needed for every exit test after that.

### 8.3 Benchmarks

All scripts live in `bench/`, write JSON, and are run before each release.

1. **Transfer:** 10 GB file and 10,000 small files; throughput against `iperf3`; resume correctness.
2. **Overhead:** latency of a no-op task, median and 99th percentile.
3. **Fan-out scaling:** 1,000 short tasks on 1, 2 and 3 devices.
4. **The dataset benchmark,** corrected from the summary. Two workloads: one I/O-bound (aggregate a 30 GB table) and one compute-bound (embed or transcode each item). Three strategies: everything on one machine; data spread but pulled back to one machine; data spread and processed where it lives. Report time, network bytes, RAM and CPU/GPU use, and result correctness. Expect the single machine to win the I/O-bound case; publish it anyway.
5. **Recovery:** time from unplugging a worker to the job making progress again.
6. **AI:** tokens per second for best fit, replicas (requests per second) and split, on wired and Wi-Fi.

## 9. Decision gates

| Gate | When | Question | If the answer is no |
| --- | --- | --- | --- |
| A | End of Milestone 0 | Does iroh connect directly over the home Wi-Fi with mDNS? Is split inference at least 1.5× faster than CPU offload for a model that does not fit one GPU? | Network: fall back to manual addresses and relays, and investigate. Split: ship it as experimental and lead the AI story with routing and replicas. |
| B | End of Milestone 1 | Can someone else pair two machines in under 5 minutes? | Fix onboarding before adding features. |
| C | End of Milestone 2 | Is fan-out at least 1.6× faster than one machine on the compute-bound benchmark with two similar devices? | Find the overhead before building AI on top. |
| D | 8 weeks after public alpha | Are at least 50 clusters outside the author's home active weekly? | Narrow to the strongest use (likely AI routing or the lab edition) and cut the rest. |

## 10. Risks

| Risk | Mitigation |
| --- | --- |
| Scope grows back toward 50 features | The triage in the feasibility review. Nothing from Milestones 5–7 before the public alpha. |
| Distributed bugs are slow to find | Simulator from day one; every failure found on real hardware becomes a simulation scenario. |
| Wi-Fi links are too slow or unstable | Link measurement feeds placement; the dashboard states each link's quality; documentation recommends a cable. |
| iroh changes or has a blocking bug | It sits behind the `Transport` trait; a plain QUIC implementation for LAN-only use is a contained fallback. |
| llama.cpp's RPC mode changes or breaks | Pin a tested llama.cpp release per OpenCompute release; split mode is labelled experimental. |
| Windows friction (firewall, SmartScreen, sleep) | `ocx doctor`, sleep goodbyes, code signing before public release. |
| Laptops overheat under sustained load | Conservative defaults: no work on battery, temperature pause, low priority. |
| A competitor closes the Windows gap | General jobs as well as AI; open source; best Windows experience. |
| Name conflict with the Open Compute Project | Choose the public name before launch. |

## 11. Decisions for you

None of these block Milestone 0. Defaults are in bold.

1. **Second computer.** What other machine can you use, and with which OS and GPU? This shapes Spike 2 and all of Milestone 1.
2. **Public name.** **Keep "OpenCompute" as the repo name for now** and pick a distinct public name before launch, or rename now.
3. **Dashboard framework.** **React with Vite**, or Next.js with static export.
4. **Order.** **AI mesh before folder sync** (this plan), or the summary's original order.
5. **Licence.** **Apache-2.0**, or MIT.
6. **Microsoft C++ Build Tools.** Not needed until Milestone 4. Installing them (about 6–7 GB) is required before the desktop app can be built on this laptop.

## 12. Milestone 0 checklist

- [x] `git init` inside `OpenCompute`, add the remote, commit, push.
- [x] Create the Cargo workspace with placeholder `oc-core`, `oc-proto`, `oc-net`, `oc-agent`, `oc-cli` crates and confirm it builds on this laptop.
- [x] Add CI for format, lint and tests on Windows, Ubuntu and macOS. First run passed.
- [x] Spike 4 (inventory).
- [x] Spike 3 (Job Objects) on Windows.
- [x] Spike 1 (iroh) between two processes on this laptop.
- [x] Spike 2 (split inference) on this laptop, with one GPU standing in for two and the link emulated.
- [ ] Spike 1 between two machines on the home Wi-Fi.
- [ ] Spike 2 between two machines with a GPU each.
- [ ] Spike 3 on Linux.
- [ ] Decide Gate A. Both halves are provisionally "yes"; each waits on its two-machine run.
