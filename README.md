# OpenCompute

An open-source app that turns the computers you own into a mesh, so any job
runs on the machine best suited to it, or on all of them at once.

**Status: pre-alpha.** There is no usable software yet. The project is in
Milestone 0: testing the riskiest assumptions before product code is written.

## What it will do

1. **Route** — run a command, a Python function or an AI model on another of
   your machines as easily as on this one.
2. **Fan out** — split a batch job across every machine and collect the
   results in one place.
3. **Split** — run an AI model too big for any single machine across several
   (experimental).

Windows with NVIDIA GPUs is the first-class platform. Linux and macOS follow.

## What it will not do

It does not pool RAM or GPU memory into one bigger computer. On home networks
that is slower than the SSD already inside each machine. The reasoning and the
evidence are in the feasibility review.

## Documents

- [Feasibility review](docs/FEASIBILITY.md) — what is practical, what is not, and the sources.
- [Build plan](docs/PLAN.md) — architecture, components, milestones and tests.
- [Spike results](docs/spikes/) — measurements from Milestone 0.

## Layout

```
crates/     the Rust workspace: agent (ocxd), CLI (ocx) and libraries
spikes/     throwaway experiments, built separately from the workspace
docs/       plan, feasibility review, spike results
```

## Building

Requires Rust 1.91 or newer and a C toolchain.

On Windows that means either Visual Studio Build Tools (C++ workload) with
Rust's default MSVC toolchain, or MinGW-w64 with the GNU toolchain
(`rustup toolchain install stable-x86_64-pc-windows-gnu`).

```bash
cargo build --workspace
```

```bash
cargo test --workspace
```

## Licence

Apache-2.0. See [LICENSE](LICENSE).
