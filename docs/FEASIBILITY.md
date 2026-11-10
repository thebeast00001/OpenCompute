# OpenCompute — feasibility review

*10 November 2026. Based on the project summary, public sources (linked in section 11), and the hardware of the development laptop.*

## 1. The verdict

**OpenCompute is practical as a "personal compute mesh". It is not practical as a "personal supercomputer that pools RAM and GPUs".**

The project summary contains two different products:

- **Pooling** means several computers behave like one bigger computer: 16 GB + 32 GB + 16 GB becomes 64 GB that any program can use. This cannot be built on home hardware. The network between machines is hundreds to thousands of times slower than the wiring inside one machine, and no software closes that gap. The companies that did build pooling (TidalScale, ScaleMP) needed a custom hypervisor and datacenter networks, and sold to enterprises. TidalScale was absorbed by HPE in 2022.
- **A mesh** means your computers know about each other, and work is sent to whichever machine is best placed to do it, or is cut into pieces that run on several machines at once. This is how Ray, Dask, BOINC and exo work. It is proven, and one person can build it in stages.

The summary already leans toward the mesh (its section 17 lists most of the limits). The recommendation is to commit to it completely: remove the pooling language, remove the features that only make sense under pooling, and build the mesh.

| Question | Answer |
| --- | --- |
| Can the core be built: discover, pair, show hardware, run work on another machine, transfer files? | **Yes.** Every piece has a working open-source precedent. Roughly 4–6 weeks to a two-machine alpha. |
| Can RAM be pooled so ordinary apps see more memory? | **No.** On a home network, remote memory is slower than the SSD already inside the machine. |
| Can GPUs be combined to run one model too big for either? | **Yes, with limits.** The model fits. It does not run faster than the slowest machine, and published tests lose about 7–30% to the network on wired links. |
| Can GPUs be combined for more throughput? | **Yes**, for many independent items (batch embedding, transcription, many chat requests at once). This scales almost linearly because nothing is shared mid-task. |
| Can one person build all 50 features? | **No.** The list is about ten separate products. The plan cuts it to one spine and adds features in order of value. |
| Is there room next to existing tools? | **A narrow, real gap:** Windows + NVIDIA + mixed hardware. The best-known tools are Mac-first or Linux-only. |

## 2. What was checked

- **Prior art:** exo, llama.cpp RPC, prima.cpp, LM Studio's LM Link, GPUStack, Parallax, Petals, distributed-llama, Cake, Ray, TidalScale.
- **Physical limits:** published latency measurements for remote memory, and real cluster benchmarks from people who built home clusters.
- **The proposed stack:** current status of iroh, Tauri 2, Ray on Windows, Firecracker and gVisor, Android background limits.
- **The development machine:** Ryzen 7 7435HS (8 cores, 16 threads), 24 GB RAM, RTX 4060 Laptop GPU (8 GB VRAM), 477 GB NVMe SSD, Wi-Fi 6 only (1.2 Gbps link rate, no wired link active). Rust 1.95, Node 22, pnpm 10, Python 3.14, uv, CMake, Docker and WSL 2 are installed. `protoc` and `gh` are not.

One finding from the machine itself: Windows' standard hardware query reports this GPU as having 4 GB of VRAM. The field is a 32-bit number and cannot hold 8 GB. Any inventory feature has to read VRAM from the NVIDIA library or DXGI instead, or it will be wrong on most modern GPUs.

## 3. Why pooling fails: the numbers

How fast each path can move data, and how long one small access takes. These are typical orders of magnitude, not measurements from this laptop.

| Path | Throughput | Time for one small access |
| --- | --- | --- |
| RAM inside one machine | tens of GB/s | about 0.0001 ms |
| NVMe SSD inside one machine | 2–7 GB/s | 0.02–0.1 ms |
| 10 Gb Ethernet | about 1.2 GB/s | 0.05–0.1 ms |
| 2.5 Gb Ethernet | about 0.3 GB/s | 0.1–0.3 ms |
| 1 Gb Ethernet | about 0.11 GB/s | 0.2–0.5 ms |
| Wi-Fi 6 in a home | 0.03–0.15 GB/s | 1–10 ms, and it varies moment to moment |

Three consequences:

1. **Remote RAM over gigabit Ethernet is roughly 20–60 times slower than the local SSD.** If a machine runs out of RAM, swapping to its own SSD beats borrowing RAM from another machine. The "remote paging" research stage in the summary would produce something worse than what Windows already does.
2. **A program that touches memory randomly is destroyed by the latency.** Reading 1 GB of remote memory in 4 KB pages is 262,144 round trips. At 0.3 ms each (wired) that is 79 seconds. At 3 ms each (Wi-Fi) it is 13 minutes. Local RAM does it in well under a second.
3. **The research systems the summary cites need hardware homes don't have.** Infiniswap and AIFM run on RDMA network cards. Published measurements put an RDMA read of a remote in-memory page at 3.66 microseconds, and a 2026 study measured 31 µs for RDMA against 73 µs for ordinary TCP on datacenter hardware. Another paper found that over half of the remaining cost is operating-system overhead. Consumer PCs have no RDMA; the one exception is recent Macs over Thunderbolt 5.

The same logic applies to GPU memory. Two 8 GB GPUs in two machines never become one 16 GB GPU. They can each hold part of a model and pass small intermediate results, which is a different and much more limited thing.

## 4. What does work

Three patterns survive the physics. Together they are the product.

**Route.** Send the whole job to the single best machine. The laptop asks, the desktop's GPU answers. Nothing is split, so nothing is lost to the network beyond one request and one reply. This is the highest-value feature and the simplest. LM Studio shipped exactly this for chat models in February 2026 (LM Link), which is good evidence people want it.

**Fan out.** Cut a job into independent pieces and run them on every machine at once: transcode 200 videos, embed 100,000 documents, run 50 simulation settings. Two similar machines finish in about half the time. This is where "more devices means more speed" is true.

**Split.** Place one AI model across several machines because it does not fit on any single one. It works on a wired network and is the most impressive demo, but it is the weakest of the three in practice (section 5).

All three depend on one rule, which the scheduler can apply automatically. Send work elsewhere only when:

```
time to send the data  +  time to run it there   <   time to run it here
     (size / link speed)
```

A 2 GB video on a 100 MB/s link costs 20 seconds to send. If the other machine's GPU encodes it in 60 seconds and this machine's CPU needs 300, sending wins. A 2 GB spreadsheet that takes 4 seconds to sum locally should never leave the machine.

This rule also exposes a problem in the summary's headline benchmark. For a 30 GB dataset, copying half of it to the second machine takes about 2.3 minutes on gigabit Ethernet and about 6 minutes on typical Wi-Fi. A single machine streaming the same data off its NVMe SSD reads all 30 GB in well under a minute. For scan-style analytics, one machine wins. Distribution wins only when the computation per byte is heavy (encoding, AI inference, rendering, simulation). The benchmark must be designed around that, and must publish the case where a single machine wins too.

## 5. Distributed AI: what the evidence says

| Source | Setup | Result |
| --- | --- | --- |
| Jeff Geerling, Aug 2025 | 4 × Framework Desktop (Ryzen AI Max+ 395), 5 Gb Ethernet, llama.cpp RPC | A 120B model ran at 33 tokens/s on one node and about 24 across the cluster. A 405B model ran at 0.7 tokens/s. His advice: buy one bigger machine first. |
| Jeff Geerling, Dec 2025 | 4 × Mac Studio M3 Ultra | llama.cpp over TCP fell from 20.4 tokens/s on one node to 15.2 on four. exo with RDMA over Thunderbolt 5 rose from 19.5 to 31.9. |
| Community benchmark, Apr 2026 | 2 × RTX 5090, 2.5 Gb Ethernet, llama.cpp RPC | A 27B model dropped from 59.8 to 55.4 tokens/s (7% loss). A larger mixture-of-experts model dropped 27%. |
| prima.cpp, ICLR 2026 | 4 ordinary home devices, Wi-Fi, mixed operating systems | A 70B model at about 1.5 tokens/s; a 32B model at 26 tokens/s using speculative decoding. Three strong devices beat six mixed ones. |
| exo README | Macs with Thunderbolt 5 RDMA | "up to 1.8x speedup on 2 devices and 3.2x speedup on 4 devices" using tensor parallelism. |

What this means:

- **Splitting makes a model fit; it does not make it fast.** Without RDMA, every published result is slower than a single machine that could hold the model.
- **The real speedups need hardware home PCs lack.** exo's gains come from RDMA over Thunderbolt 5 on Macs.
- **More devices can be worse.** prima.cpp's scheduler deliberately drops weak devices. A planner that knows when to say "don't split" is part of the product.
- **There is one case where splitting should win on ordinary PCs:** a model too big for one GPU but small enough for two. The single-machine alternative is spilling layers onto the CPU, which is far slower than a second GPU over a wired link. This is exactly the case of an 8 GB laptop GPU plus a second GPU machine. It is plausible but not proven by the sources above, so it is the first thing the plan measures (Milestone 0).
- **llama.cpp's own documentation calls its RPC mode "fragile and insecure" and says never to run it on an open network.** Wrapping it in authenticated, encrypted connections is a real contribution, not just packaging.

The trillion-parameter table in the summary should be deleted. At 500 GB for the smallest format, it is unrelated to anything this hardware class can do, and it sets the wrong expectation.

## 6. Who already does this

| Project | What it does | Where it stops |
| --- | --- | --- |
| **exo** (48k GitHub stars, v1.0.71) | Clusters devices to run large AI models; automatic discovery; dashboard | Built on Apple's MLX. Its README says Linux runs on CPU only with GPU support "under development". Windows is not covered. AI only. Devices must run the same version and build. |
| **LM Studio — LM Link** | Use a model on another of your machines as if it were local, over Tailscale | Routing only: no splitting, no fan-out, no general jobs. Closed source. Requires an LM Studio account. |
| **llama.cpp RPC** | Splits one model across machines; works with CUDA, Metal, CPU | A proof of concept by its own description. No discovery, no security, manual setup. |
| **prima.cpp** | Research system for mixed home devices over Wi-Fi | Research code. CUDA only at the time of the paper. |
| **GPUStack** (v2.3.0) | GPU cluster manager for serving models | Now aimed at datacenters: worker nodes are Linux-only, and the engines are vLLM, SGLang and TensorRT-LLM. |
| **Parallax** (Gradient) | Decentralised model serving across mixed GPUs; has a Windows app | AI only. Tied to a crypto-adjacent network. Young (first release late 2025). |
| **Ray / Dask** | General distributed computing for Python | Ray's docs say multi-node clusters are "only supported on Linux"; Windows clusters are "experimental and untested". Built for engineers with servers, not people with two PCs. |
| **Syncthing / LocalSend** | File sync / nearby file transfer | Files only. Mature — Syncthing has more than a decade of edge-case fixes. |
| **Tailscale / iroh** | Secure device-to-device networking | Plumbing, not a product. iroh is a library OpenCompute can build on. |
| **BOINC / HTCondor** | Volunteer and campus computing on idle PCs | Decades old, proven, and hard to set up. No AI focus. |

**The gap:** nobody offers one zero-setup app that covers general job offload *and* AI serving on **Windows and NVIDIA first, across mixed hardware**. exo owns Macs. GPUStack and Ray own Linux servers. LM Link does routing for chat only. The gaming-PC-plus-laptop owner, the student with two Windows machines, and the college lab full of Windows PCs are under-served.

That gap is real but narrow. It closes if exo ships Linux and Windows GPU support, or if LM Studio adds splitting. The defence is doing general jobs (not only AI) and being the best on Windows.

## 7. Corrections to the proposed design

| The summary says | Problem | Use instead |
| --- | --- | --- |
| Firecracker and gVisor for isolation | Both are Linux-only. Neither runs on the Windows machines this project targets first. | Treat your own devices as trusted, like SSH does. Limit tasks with Windows Job Objects and Linux cgroups. Add a WebAssembly sandbox later for untrusted code. |
| Build QUIC + ICE + STUN + TURN for connectivity | That is a multi-year networking project by itself. | **iroh** (stable 1.x since mid-2026, Rust, Windows/Linux/macOS/Android/iOS). It provides key-based device identity, encryption, hole punching and relay fallback in one library. |
| OpenSSL | Painful to build on Windows; unnecessary. | rustls, which iroh already uses. |
| A central cluster coordinator | Personal machines sleep and close their lids. A fixed coordinator is a single point of failure that is usually a laptop. | Each job is owned by the device that submitted it. Membership is a small signed list every device holds. No master, no election. |
| Next.js for the dashboard | The dashboard is served by the local agent and embedded in Tauri, so there is no Node server. Next.js then needs static export: no server rendering, no API routes, no image optimisation. | A plain React single-page app built with Vite. Same React, Tailwind and shadcn/ui, fewer traps. Next.js is the right tool for the marketing and docs site later. |
| NCCL, vLLM, PyTorch Distributed as AI building blocks | These assume Linux servers with matching GPUs and fast interconnects. | llama.cpp, which runs on every OS and GPU vendor in scope and already has an RPC split mode. |
| iroh-blobs or similar for file transfer | Its README says the current version "is not yet considered production quality". | A small content-addressed chunk store of our own, behind an interface so it can be swapped later. |
| Phones contribute compute | Android limits background data-sync services to about 6 hours per day for apps targeting Android 15+; iOS is stricter. Heat and battery make it pointless anyway. | Phones are remote controls: pair by QR, monitor, send files, chat with models running on the PCs. |
| Remote paging as a research stage | Slower than the local SSD on any home network (section 3). | Remove it. Keep an optional experiment for 10 Gb Ethernet or Thunderbolt links only. |
| Hero number "64 GB combined RAM" | True as arithmetic, false as an impression. It promises pooling. | Show per-device capacity and "largest job that fits right now". Label any sum as a total across devices. |
| CLI named `oc` (implied) | `oc` is already the OpenShift command-line tool. | `ocx` as a working name. |
| The name "OpenCompute" | The Open Compute Project (opencompute.org, founded by Meta, holder of the OCP trademarks) is a major organisation in computing infrastructure. Search results and possible confusion both work against the name. | Keep it as the repo name for now. Decide on a distinct public name before any launch. |

Choices in the summary that hold up: Rust for the agent; Tauri 2 for the desktop app (stable since October 2024); Protocol Buffers for messages; SQLite for local state; LAN first and internet later; no Kubernetes, Redis or Supabase; staged delivery; and the "two computers, one cluster" first milestone.

## 8. Feature triage

| Feature from the summary | Decision | Why |
| --- | --- | --- |
| Automatic discovery, pairing, device management | **Build first** | The foundation. Low risk. |
| Hardware inventory and live monitoring | **Build first** | Useful with a single device. |
| Remote task execution | **Build first** | The "route" pattern. |
| Device-to-device file transfer | **Build first** | Also needed internally to move job inputs and AI models. |
| CLI and local API | **Build first** | Everything else sits on them. |
| Scheduler, retries, failure recovery | **Build second** | The "fan out" pattern. |
| Distributed object store with data locality | **Build second** | The honest version of "distributed memory". |
| Python SDK | **Build second** | How developers actually use it. |
| Resource reservations, user-priority rules | **Build second** | A machine that slows down while its owner is using it gets uninstalled. |
| AI model serving with routing | **Build third** | The feature most people will install it for. |
| Split inference across machines | **Build third**, labelled experimental | Works, with caveats; depends on the Milestone 0 measurement. |
| Desktop app, installers, auto-update | **Build fourth** | Needed for anyone other than the author to use it. |
| Shared folders, two-way sync, versioning | **Later** | The deepest rabbit hole on the list. One-way mirroring first. |
| Internet connectivity, mobile client | **Later** | iroh makes it tractable, but LAN must be solid first. |
| Plugin system | **Later** | Needs users before it needs plugins. |
| Workload migration, high-availability coordination | **Research** | Only if real usage demands it. |
| Transparent RAM pooling, remote paging | **Drop** | Physically pointless on home networks. |
| "Combined GPU memory" as one pool | **Drop** | Does not exist; replaced by route, fan out, split. |
| Running games or arbitrary Windows apps across machines | **Drop** | Not possible transparently. |
| Phones contributing compute | **Drop** | Blocked by the OS and by battery and heat. |
| Marketplace for contributed compute | **Drop** | A separate company with unsolved trust problems. |
| Trillion-parameter models | **Drop** | Unrelated to this hardware class. |

## 9. Will anyone use it?

Evidence of demand: exo has 48,000 GitHub stars for "cluster your devices to run AI". LM Studio partnered with Tailscale to ship "use your other machine's GPU". People want this.

Evidence of difficulty: exo is funded and years ahead on Macs, and tools with more resources are converging on the same idea.

Realistic first users, in order:

1. **People who run local AI and own a gaming PC plus a laptop.** They feel the problem daily and already install tools like this.
2. **Students and developers with two or three mixed machines** who want batch jobs to finish faster.
3. **College labs.** A room of 40 Windows PCs sits idle every night. HTCondor has served this need since the 1980s but is hard to run. A zero-setup equivalent that also serves AI models is a credible institutional offering, and Indian engineering colleges are an accessible first market. This is the most plausible path to revenue (fleet management, policies, support), but it comes after the core works.

The summary's answer to "why install it with only one computer" is right, and should be narrowed to two things that are useful alone: a hardware monitor and a one-click local AI server with a standard API. Both become cluster features the moment a second device is paired.

On ambition: this is a good open-source project with an uncertain path to a large business. It should be judged first on whether people outside the author's home use it weekly.

## 10. Main risks

| Risk | Likelihood | What reduces it |
| --- | --- | --- |
| Scope creep back toward 50 features | High | The triage above; milestone exit tests; nothing from "Later" before the public alpha. |
| Only one development machine | High (today) | A multi-agent test mode on one PC plus WSL 2 as a Linux node covers most development. A second physical machine is still required to finish Milestone 1. |
| Split inference disappoints on real home networks | Medium | Measured in Milestone 0 before any product code. If it loses, it ships as experimental and routing leads. |
| Wi-Fi-only links make everything slow | Medium | The scheduler measures each link and refuses data-heavy placements over slow ones. Wired Ethernet is the documented recommendation. |
| Windows-specific friction: firewall prompts, unsigned-installer warnings, sleep | Medium | A diagnostic command (`ocx doctor`), sleep notifications to peers, and a code-signing budget before public release. |
| A competitor closes the Windows gap | Medium | General job offload, not only AI; open source; first-class Windows support. |
| Debugging distributed failures eats the schedule | High | Scheduler logic is written to run inside a deterministic simulator from the start. |
| Name conflict with the Open Compute Project | Medium | Decide the public name before launch. |

## 11. Sources

**Distributed AI on consumer hardware**
- exo: https://github.com/exo-explore/exo
- llama.cpp RPC documentation: https://github.com/ggml-org/llama.cpp/blob/master/tools/rpc/README.md
- Jeff Geerling, Framework cluster: https://www.jeffgeerling.com/blog/2025/i-clustered-four-framework-mainboards-test-huge-llms
- Jeff Geerling, Mac Studio RDMA cluster: https://www.jeffgeerling.com/blog/2025/15-tb-vram-on-mac-studio-rdma-over-thunderbolt-5
- AppleInsider summary of the Mac cluster numbers: https://appleinsider.com/articles/25/12/20/ai-calculations-on-mac-cluster-gets-a-big-boost-from-new-rdma-support-on-thunderbolt-5
- prima.cpp (ICLR 2026): https://arxiv.org/html/2504.08791v2
- llama.cpp RPC benchmark notes (10 GbE vs 1 GbE): https://github.com/kjaiswal/llama-cpp-distributed-benchmarks
- Community 2 × RTX 5090 benchmark (forum snapshot, lower reliability): https://reddit.sentinel-team.org/posts/1sfs0wt/snapshots/2026-04-09T16%3A11%3A00.655487Z
- LM Link: https://lmstudio.ai/link and https://lmstudio.ai/docs/lmlink
- GPUStack: https://github.com/gpustack/gpustack
- Parallax paper: https://arxiv.org/pdf/2509.26182v1
- distributed-llama: https://github.com/b4rtaz/distributed-llama
- Cake: https://github.com/evilsocket/cake
- Petals: https://github.com/bigscience-workshop/petals

**Remote memory**
- Memory disaggregation survey: https://arxiv.org/pdf/2202.02223
- MemExchange (RDMA vs TCP latency, 2026): https://arxiv.org/pdf/2607.11579
- FAST '24 RDMA paging measurement: https://www.usenix.net/system/files/fast24-yang-zhe.pdf
- Hermit (software overhead in remote swap): https://par.nsf.gov//servlets/purl/10428579
- TidalScale background: https://blocksandfiles.com/2022/08/17/tidalscale-and-inverted-server-virtualization/ and https://pitchbook.com/profiles/company/58507-48

**Stack**
- iroh: https://github.com/n0-computer/iroh and https://docs.iroh.computer/
- iroh-blobs: https://github.com/n0-computer/iroh-blobs
- Ray installation and Windows caveats: https://docs.ray.io/en/latest/ray-overview/installation.html
- Ray cluster platform support: https://docs.ray.io/en/latest/cluster/getting-started.html
- Tauri 2: https://v2.tauri.app/
- Android foreground-service time limits: https://developer.android.com/develop/background-work/services/fgs/timeout
- Wi-Fi 6 vs gigabit measurement: https://hexus.net/tech/reviews/network/144097-killer-wi-fi-6-ax1650-pcie-card/?page=2

**Naming**
- Open Compute Project: https://www.opencompute.org/about and https://en.wikipedia.org/wiki/Open_Compute_Project

Figures marked as typical or expected are engineering estimates, not measurements. Several benchmark numbers come from secondary write-ups of the original tests; the plan re-measures the ones that decisions depend on.
