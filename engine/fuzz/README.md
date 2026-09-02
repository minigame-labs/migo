# Fuzz targets

Not workspace members: cargo-fuzz builds with its own profile and sanitizer
flags, and making these members would push those flags onto every other crate.
Each target declares an empty `[workspace]` so cargo treats it as its own root.

```sh
cargo +nightly fuzz run envelope --fuzz-dir engine/fuzz/frame-wire
```

Seed the corpus from `contracts/frame-wire/golden/`. Those are the shapes a real
producer emits, so a fuzzer that starts there spends its budget on mutations of
valid packets rather than rediscovering the magic number.

| Target | Property |
|---|---|
| `frame-wire/envelope` | no input makes the cross-process frame parser panic, read out of bounds, loop unboundedly, or allocate a size the input chose |
