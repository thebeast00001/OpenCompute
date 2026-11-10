# Spike 2 — raw benchmark output

Unedited output of `spikes/split-inference/run.sh` and `cold-load.sh` on the development laptop, 10 November 2026.

## First batch

### Run: 2026-11-10 14:48, 3 repetitions each
pp512 = processing a 512-token prompt; tg128 = generating 128 tokens. Both in tokens per second.

## One machine
### Whole model on the GPU  (took 11 s)
| model                          |       size |     params | backend    | ngl |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | --: | --------------: | -------------------: |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA       |  99 |           pp512 |      2861.08 ± 47.18 |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA       |  99 |           tg128 |         53.05 ± 0.10 |

### Half the layers on the GPU, half on the CPU  (took 36 s)
| model                          |       size |     params | backend    | ngl |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | --: | --------------: | -------------------: |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA       |  14 |           pp512 |      1286.38 ± 67.82 |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA       |  14 |           tg128 |         13.54 ± 1.71 |

### CPU only  (took 48 s)
| model                          |       size |     params | backend    | ngl |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | --: | --------------: | -------------------: |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA       |   0 |           pp512 |       840.45 ± 26.36 |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA       |   0 |           tg128 |          9.07 ± 0.44 |

## Split across two devices (second half behind llama.cpp RPC)
### Whole model on the RPC device, no link delay  (took 415 s)
| model                          |       size |     params | backend    | ngl | dev          |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | --: | ------------ | --------------: | -------------------: |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA,RPC   |  99 | RPC0         |           pp512 |     1968.58 ± 108.53 |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA,RPC   |  99 | RPC0         |           tg128 |         44.81 ± 1.08 |

### Half local GPU, half RPC device, no link delay  (took 24 s)
| model                          |       size |     params | backend    | ngl | dev          | ts           |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | --: | ------------ | ------------ | --------------: | -------------------: |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA,RPC   |  99 | CUDA0/RPC0   | 1.00/1.00    |           pp512 |      1840.56 ± 89.41 |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA,RPC   |  99 | CUDA0/RPC0   | 1.00/1.00    |           tg128 |         45.14 ± 0.41 |

### Half local GPU, half RPC device, emulated gigabit Ethernet (0.3 ms round trip, 940 Mbit/s)  (took 28 s)
| model                          |       size |     params | backend    | ngl | dev          | ts           |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | --: | ------------ | ------------ | --------------: | -------------------: |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA,RPC   |  99 | CUDA0/RPC0   | 1.00/1.00    |           pp512 |      1526.77 ± 87.52 |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA,RPC   |  99 | CUDA0/RPC0   | 1.00/1.00    |           tg128 |         35.55 ± 0.59 |

### Half local GPU, half RPC device, emulated good Wi-Fi (3 ms round trip, 400 Mbit/s)  (took 39 s)
| model                          |       size |     params | backend    | ngl | dev          | ts           |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | --: | ------------ | ------------ | --------------: | -------------------: |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA,RPC   |  99 | CUDA0/RPC0   | 1.00/1.00    |           pp512 |      1202.86 ± 66.88 |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA,RPC   |  99 | CUDA0/RPC0   | 1.00/1.00    |           tg128 |         25.97 ± 0.14 |

### Half local GPU, half RPC device, emulated poor Wi-Fi (8 ms round trip, 80 Mbit/s)  (took 82 s)
| model                          |       size |     params | backend    | ngl | dev          | ts           |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | --: | ------------ | ------------ | --------------: | -------------------: |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA,RPC   |  99 | CUDA0/RPC0   | 1.00/1.00    |           pp512 |        435.95 ± 8.81 |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA,RPC   |  99 | CUDA0/RPC0   | 1.00/1.00    |           tg128 |          9.25 ± 0.35 |

done

## Second batch: remote device first

### Run: 2026-11-10 15:01, 3 repetitions each
pp512 = processing a 512-token prompt; tg128 = generating 128 tokens. Both in tokens per second.

## Split: RPC device runs the first half, local GPU the second half and the output
### Remote device first, no link delay  (took 21 s)
| model                          |       size |     params | backend    | ngl | dev          | ts           |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | --: | ------------ | ------------ | --------------: | -------------------: |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA,RPC   |  99 | RPC0/CUDA0   | 1.00/1.00    |           pp512 |     2039.35 ± 202.06 |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA,RPC   |  99 | RPC0/CUDA0   | 1.00/1.00    |           tg128 |         47.17 ± 0.49 |

### Remote device first, emulated gigabit Ethernet (0.3 ms round trip, 940 Mbit/s)  (took 24 s)
| model                          |       size |     params | backend    | ngl | dev          | ts           |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | --: | ------------ | ------------ | --------------: | -------------------: |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA,RPC   |  99 | RPC0/CUDA0   | 1.00/1.00    |           pp512 |      1245.82 ± 85.15 |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA,RPC   |  99 | RPC0/CUDA0   | 1.00/1.00    |           tg128 |         48.22 ± 0.25 |

### Remote device first, emulated good Wi-Fi (3 ms round trip, 400 Mbit/s)  (took 28 s)
| model                          |       size |     params | backend    | ngl | dev          | ts           |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | --: | ------------ | ------------ | --------------: | -------------------: |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA,RPC   |  99 | RPC0/CUDA0   | 1.00/1.00    |           pp512 |       849.14 ± 17.11 |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA,RPC   |  99 | RPC0/CUDA0   | 1.00/1.00    |           tg128 |         42.47 ± 0.14 |

### Remote device first, emulated poor Wi-Fi (8 ms round trip, 80 Mbit/s)  (took 60 s)
| model                          |       size |     params | backend    | ngl | dev          | ts           |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | --: | ------------ | ------------ | --------------: | -------------------: |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA,RPC   |  99 | RPC0/CUDA0   | 1.00/1.00    |           pp512 |        276.13 ± 7.29 |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA,RPC   |  99 | RPC0/CUDA0   | 1.00/1.00    |           tg128 |         30.74 ± 0.41 |

done

## Repeat of the whole-model-behind-RPC case (1 repetition)

### Run: 2026-11-10 15:06, 1 repetitions each
pp512 = processing a 512-token prompt; tg128 = generating 128 tokens. Both in tokens per second.

## Whole model behind llama.cpp RPC
### Whole model on the RPC device, no link delay  (took 32 s)
| model                          |       size |     params | backend    | ngl | dev          |            test |                  t/s |
| ------------------------------ | ---------: | ---------: | ---------- | --: | ------------ | --------------: | -------------------: |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA,RPC   |  99 | RPC0         |           pp512 |       1387.17 ± 0.00 |
| qwen2 7B Q4_K - Medium         |   4.36 GiB |     7.62 B | CUDA,RPC   |  99 | RPC0         |           tg128 |         43.12 ± 0.00 |

done

## First-load timing

```
First load of a split model, same machine, no link delay
server cache off                      4.6 s  (exit 0)
server cache on, empty                9.6 s  (exit 0)
server cache on, warm                10.2 s  (exit 0)
cache size on disk: 1.8G
```
