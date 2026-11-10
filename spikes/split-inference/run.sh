#!/usr/bin/env bash
# Spike 2: what does splitting one model across machines cost, compared with
# keeping it on one GPU or spilling half of it onto the CPU?
#
# One GPU stands in for two: the model's second half runs in llama.cpp's RPC
# server on the same machine, reached either directly or through the link
# emulator (spike-linkemu), which adds delay and a bandwidth cap. In a layer
# split the two halves run one after the other, never at once, so sharing a
# GPU does not make the split look slower than two equal GPUs would.
#
# usage: run.sh <cache_dir> [repetitions]
#   <cache_dir> holds llama-bin/, cudart/ and models/ (see docs/spikes/README.md)

set -u

CACHE="$(cd "$1" && pwd)"
REPS="${2:-3}"
HERE="$(cd "$(dirname "$0")" && pwd)"
BIN="$CACHE/llama-bin"
MODEL="$CACHE/models/qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf"
EMU="$HERE/../target/release/spike-linkemu.exe"
RPC_PORT=50052
HALF_LAYERS=14 # Qwen2.5-7B has 28 transformer layers

export PATH="$CACHE/cudart:$BIN:$PATH"
export LLAMA_CACHE="$CACHE/rpc-cache"
mkdir -p "$LLAMA_CACHE"

for f in "$BIN/llama-bench.exe" "$BIN/ggml-rpc-server.exe" "$MODEL" "$EMU"; do
    [ -e "$f" ] || { echo "missing: $f" >&2; exit 1; }
done

# Run llama-bench and print only its result row(s).
bench() {
    local label="$1"
    shift
    local started=$SECONDS
    local rows
    rows="$("$BIN/llama-bench.exe" -m "$MODEL" -p 512 -n 128 -r "$REPS" -o md "$@" 2>"$CACHE/bench.err" | grep -E '^\|')"
    echo "### $label  (took $((SECONDS - started)) s)"
    if [ -n "$rows" ]; then
        echo "$rows"
    else
        echo "FAILED; last lines of stderr:"
        tail -n 5 "$CACHE/bench.err"
    fi
    echo
}

echo "# Spike 2 results: $(date '+%Y-%m-%d %H:%M'), $REPS repetitions each"
echo "pp512 = processing a 512-token prompt; tg128 = generating 128 tokens. Both in tokens per second."
echo

# SECTIONS selects what runs (space separated). Default: everything.
SECTIONS="${SECTIONS:-single rpc-whole split-remote-last split-remote-first}"
wants() { case " $SECTIONS " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }

if wants single; then
    echo "## One machine"
    bench "Whole model on the GPU" -ngl 99
    bench "Half the layers on the GPU, half on the CPU" -ngl "$HALF_LAYERS"
    bench "CPU only" -ngl 0
fi

"$BIN/ggml-rpc-server.exe" -H 127.0.0.1 -p "$RPC_PORT" -c >"$CACHE/rpc-server.log" 2>&1 &
RPC_PID=$!
trap 'kill $RPC_PID 2>/dev/null' EXIT
sleep 4

if wants rpc-whole; then
    echo "## Whole model behind llama.cpp RPC"
    bench "Whole model on the RPC device, no link delay" -rpc "127.0.0.1:$RPC_PORT" -dev RPC0 -ngl 99
fi

# Run one device order with no delay, then over each emulated link.
# The device listed last holds the final layers and produces the output
# vector, which is as large as the model's vocabulary.
split_runs() {
    local order="$1" what="$2"
    bench "$what, no link delay" -rpc "127.0.0.1:$RPC_PORT" -dev "$order" -ts 1/1 -ngl 99
    # name | one-way delay (ms) | bandwidth (Mbit/s) | emulator port
    while IFS='|' read -r name delay mbit port; do
        "$EMU" "$port" "127.0.0.1:$RPC_PORT" "$delay" "$mbit" >/dev/null 2>&1 &
        local emu_pid=$!
        sleep 1
        bench "$what, $name" -rpc "127.0.0.1:$port" -dev "$order" -ts 1/1 -ngl 99
        kill $emu_pid 2>/dev/null
    done <<'PROFILES'
emulated gigabit Ethernet (0.3 ms round trip, 940 Mbit/s)|0.15|940|50061
emulated good Wi-Fi (3 ms round trip, 400 Mbit/s)|1.5|400|50062
emulated poor Wi-Fi (8 ms round trip, 80 Mbit/s)|4|80|50063
PROFILES
}

if wants split-remote-last; then
    echo "## Split: local GPU runs the first half, RPC device the second half and the output"
    split_runs CUDA0/RPC0 "Remote device last"
fi

if wants split-remote-first; then
    echo "## Split: RPC device runs the first half, local GPU the second half and the output"
    split_runs RPC0/CUDA0 "Remote device first"
fi

echo "done"
