# Benchmarking

```sh
cargo bench --bench compositing      # GPU; needs an adapter, headless ok
cargo bench --bench shader_params    # CPU only

cargo bench --bench compositing -- --save-baseline <name>
cargo bench --bench compositing -- --baseline <name>

./scripts/bench-smoke.sh             # --test on both, no sampling
```

Criterion HTML reports land in `target/criterion/`.

`compositing` runs at 1920×1080 and `device.poll(Wait)` after each iteration
so wall-clock reflects GPU work. Without a GPU adapter it prints `no GPU
adapter — skipping` and exits clean. Numbers are machine-local; close other
GPU work and warm the machine for stability.
