#!/usr/bin/env bash
# Spike 2 follow-up: how long does the first load of a split model take?
#
# On first load the client uploads the remote half's weights (about 2.2 GiB
# for this model) to llama.cpp's RPC server. This times that upload with the
# server's on-disk cache off, with it on but empty, and with it warm.
#
# usage: cold-load.sh <cache_dir>

set -u

CACHE="$(cd "$1" && pwd)"
BIN="$CACHE/llama-bin"
MODEL="$CACHE/models/qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf"
PORT=50052

export PATH="$CACHE/cudart:$BIN:$PATH"
export LLAMA_CACHE="$CACHE/rpc-cache"

# Time one short benchmark; nearly all of it is model loading.
timed_load() {
    local label="$1"
    local started
    started=$(date +%s.%N)
    "$BIN/llama-bench.exe" -m "$MODEL" -p 16 -n 4 -r 1 -o md \
        -rpc "127.0.0.1:$PORT" -dev RPC0/CUDA0 -ts 1/1 -ngl 99 >/dev/null 2>"$CACHE/bench.err"
    local status=$?
    awk -v s="$started" -v e="$(date +%s.%N)" -v l="$label" -v st="$status" \
        'BEGIN { printf "%-34s %6.1f s  (exit %s)\n", l, e - s, st }'
}

run_server() {
    "$BIN/ggml-rpc-server.exe" -H 127.0.0.1 -p "$PORT" "$@" >"$CACHE/rpc-server.log" 2>&1 &
    SERVER_PID=$!
    sleep 4
}

echo "First load of a split model, same machine, no link delay"

run_server
timed_load "server cache off"
kill $SERVER_PID 2>/dev/null
sleep 2

rm -rf "$LLAMA_CACHE"
mkdir -p "$LLAMA_CACHE"
run_server -c
timed_load "server cache on, empty"
timed_load "server cache on, warm"
kill $SERVER_PID 2>/dev/null

du -sh "$LLAMA_CACHE" | awk '{ print "cache size on disk: " $1 }'
