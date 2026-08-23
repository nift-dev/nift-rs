# Safety policy (nift-rs)

## Unsafe code is forbidden

The core crate forbids `unsafe` code:

```rust
#![forbid(unsafe_code)]
```

and the workspace enforces the same rule for every crate:

```toml
[workspace.lints.rust]
unsafe_code = "forbid"
```

`forbid` (not `deny`) is deliberate: `deny` can be overridden locally with
`#[allow(unsafe_code)]`; `forbid` cannot.

## If unsafe ever becomes necessary

The expected answer is: it does not. A pure-Rust parser/engine has no
legitimate need for `unsafe`. If a future change argues for carefully isolated
`unsafe` (for example a validated FFI seam), the policy change itself requires
explicit review with evidence:

1. state the exact unsafety contract (which invariants the caller must uphold);
2. show the isolation boundary (a single module, no reach into the core);
3. justify why safe Rust cannot express it;
4. get explicit architectural approval.

Until then, `unsafe` does not exist in this codebase.

## Panic policy

nift-rs is a library: malformed input, malformed project state, hostile
templates and hostile paths must be handled as `Result`/error values, never
panics. The public API surface must not panic on any input. Internal
invariant violations may use `debug_assert!`/`assert!` in debug builds but must
be structurally prevented in release builds. The hardening checkpoint (NR11)
and the final cold review (NR12) audit this property.

## Correctness priority

Correctness → architecture → safety → DX → concurrency correctness →
performance. Performance work that would risk the safety/correctness
properties is deferred until NR11/NR12 and is never allowed to introduce
`unsafe`.
